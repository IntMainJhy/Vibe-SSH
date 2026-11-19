//! End-to-end: the real `vibe-ssh` binary, the local socket, and the system OpenSSH server
//! on loopback with temporary keys. Nothing here reads the user's SSH configuration.
#![cfg(target_os = "macos")]

#[path = "support/sshd.rs"]
mod sshd;

use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use serde_json::Value;
use sshd::Sshd;
use tokio::io::AsyncWriteExt;
use vibe_ssh_core::{By, Service, serve};
use vibe_ssh_protocol::Owner;

struct Env {
    sshd: Sshd,
    service: Arc<Service>,
}

impl Env {
    async fn start() -> Self {
        let sshd = Sshd::start();
        let home = sshd.path("home");
        std::fs::create_dir(&home).unwrap();
        // The embedding process (the app) supplies secrets; agents never see them.
        let service = Service::new(
            home,
            Box::new(|host| (host.id == "encrypted").then(|| sshd::PASSPHRASE.to_owned())),
        );
        service
            .save_hosts(&[
                sshd.host("box", "id_plain", true),
                sshd.host("encrypted", "id_encrypted", true),
                sshd.host("mine", "id_plain", false),
            ])
            .await
            .unwrap();
        serve(service.clone()).await.unwrap();
        Self { sshd, service }
    }

    fn remote(&self, name: &str) -> PathBuf {
        let path = self.sshd.path("remote").join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        path
    }

    /// Runs the real CLI as an agent would. Returns `(exit code, stdout JSON, stderr JSON)`.
    async fn cli(&self, args: &[&str], stdin: Option<&str>) -> (i32, Value, Value) {
        let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_vibe-ssh"))
            .args(args)
            .env("VIBE_SSH_HOME", self.service.home())
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        if let Some(text) = stdin {
            let mut pipe = child.stdin.take().unwrap();
            pipe.write_all(text.as_bytes()).await.unwrap();
        }
        let output = child.wait_with_output().await.unwrap();
        let json = |bytes: &[u8]| serde_json::from_slice(bytes).unwrap_or(Value::Null);
        (
            output.status.code().unwrap(),
            json(&output.stdout),
            json(&output.stderr),
        )
    }

    async fn ok(&self, args: &[&str]) -> Value {
        let (code, out, err) = self.cli(args, None).await;
        assert_eq!(code, 0, "{args:?}: {out} {err}");
        out
    }

    async fn open(&self, host: &str) -> (String, String) {
        let view = self
            .ok(&[
                "space", "open", host, "--name", "e2e goal", "--actor", "e2e",
            ])
            .await;
        (text(&view["space"]), text(&view["control"]["lease"]))
    }

    /// Follows a job with cursors until it leaves `running`. Returns `(final job, all stdout)`.
    async fn follow(&self, mut job: Value) -> (Value, String) {
        let mut seen = text(&job["stdout"]["text"]);
        for _ in 0..40 {
            if job["status"] != "running" {
                return (job, seen);
            }
            let (_, next, err) = self
                .cli(
                    &["job", "wait", &text(&job["cursor"]), "--timeout", "5"],
                    None,
                )
                .await;
            assert!(err.is_null(), "{err}");
            seen.push_str(next["stdout"]["text"].as_str().unwrap());
            job = next;
        }
        panic!("job never finished: {job}");
    }
}

fn text(value: &Value) -> String {
    value
        .as_str()
        .unwrap_or_else(|| panic!("expected a string, got {value}"))
        .to_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn outbound_http_and_socks_proxies_route_pinned_ssh_without_direct_fallback() {
    use tokio::{
        io::{AsyncReadExt, BufReader},
        net::{TcpListener, TcpStream},
    };
    use vibe_ssh_core::{ProxyKind, ProxySettings};
    use vibe_ssh_protocol::ErrorCode;

    for kind in [ProxyKind::HttpConnect, ProxyKind::Socks5] {
        let env = Env::start().await;
        let mut host = env.sshd.host("routed", "id_plain", true);
        let port = host.port;
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let proxy = ProxySettings {
            kind,
            address: "127.0.0.1".into(),
            port: listener.local_addr().unwrap().port(),
        };
        let relayed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let accept = {
            let relayed = relayed.clone();
            tokio::spawn(async move {
                for _ in 0..3 {
                    let (client, _) = listener.accept().await.unwrap();
                    let relayed = relayed.clone();
                    tokio::spawn(async move {
                        let mut client = BufReader::new(client);
                        let mut ssh = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
                        match kind {
                            ProxyKind::HttpConnect => {
                                use tokio::io::AsyncBufReadExt;
                                let mut headers = String::new();
                                loop {
                                    let before = headers.len();
                                    assert!(client.read_line(&mut headers).await.unwrap() > 0);
                                    if &headers[before..] == "\r\n" {
                                        break;
                                    }
                                }
                                assert!(headers.starts_with(&format!(
                                    "CONNECT 127.0.0.1:{port} HTTP/1.1\r\n"
                                )));
                                let mut reply =
                                    b"HTTP/1.1 200 Connection established\r\n\r\n".to_vec();
                                loop {
                                    let byte = ssh.read_u8().await.unwrap();
                                    reply.push(byte);
                                    if byte == b'\n' {
                                        break;
                                    }
                                }
                                client.write_all(&reply).await.unwrap();
                            }
                            ProxyKind::Socks5 => {
                                let mut methods = [0; 3];
                                client.read_exact(&mut methods).await.unwrap();
                                assert_eq!(methods, [5, 1, 0]);
                                client.write_all(&[5, 0]).await.unwrap();
                                let mut target = [0; 10];
                                client.read_exact(&mut target).await.unwrap();
                                assert_eq!(&target[..8], &[5, 1, 0, 1, 127, 0, 0, 1]);
                                assert_eq!(&target[8..], &port.to_be_bytes());
                                client
                                    .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])
                                    .await
                                    .unwrap();
                            }
                        }
                        relayed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let _ = tokio::io::copy_bidirectional(&mut client, &mut ssh).await;
                    });
                }
            })
        };
        host.proxy = Some(proxy.clone());
        let fingerprint = env
            .service
            .probe_host(&host.host, port, None, Some(&proxy))
            .await
            .unwrap()
            .1;
        assert_eq!(fingerprint, host.host_key_sha256);
        let mut wrong = host.clone();
        wrong.host_key_sha256 = "SHA256:wrong".into();
        env.service.save_hosts(&[wrong]).await.unwrap();
        assert_eq!(
            env.service
                .open("routed", "wrong pin", "fixture", Owner::Agent)
                .await
                .unwrap_err()
                .code,
            ErrorCode::HostKeyMismatch
        );
        env.service.save_hosts(&[host.clone()]).await.unwrap();
        let (space, lease) = env.open("routed").await;
        let reply = env
            .ok(&["run", "--lease", &lease, "--", "printf proxy-ok"])
            .await;
        assert_eq!(reply["stdout"]["text"], "proxy-ok");
        env.service.close(By::Human(&space)).await.unwrap();
        accept.await.unwrap();
        assert_eq!(relayed.load(std::sync::atomic::Ordering::SeqCst), 3);

        let denied = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        host.proxy.as_mut().unwrap().port = denied.local_addr().unwrap().port();
        env.service.save_hosts(&[host]).await.unwrap();
        let deny = tokio::spawn(async move {
            let (mut client, _) = denied.accept().await.unwrap();
            match kind {
                ProxyKind::HttpConnect => {
                    let mut header = [0; 4];
                    client.read_exact(&mut header).await.unwrap();
                    assert_eq!(&header, b"CONN");
                    client
                        .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n")
                        .await
                        .unwrap();
                }
                ProxyKind::Socks5 => {
                    let mut header = [0; 3];
                    client.read_exact(&mut header).await.unwrap();
                    client.write_all(&[5, 0xff]).await.unwrap();
                }
            }
        });
        let error = env
            .service
            .open("routed", "proxy denied", "fixture", Owner::Agent)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::ConnectionFailed);
        deny.await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_checks_the_plan_before_open_and_reports_completed_and_skipped_steps() {
    let env = Env::start().await;
    let args = ["batch", "--host", "box", "--name", "publish"];
    let source = env.remote("input");
    let remote = env.remote("release");
    let skipped = env.remote("must-not-run");
    std::fs::write(&source, b"published bytes").unwrap();
    let run = |name: &str, command: String| serde_json::json!({"type":"run","name":name,"command":command});
    let plan = serde_json::json!({"steps": [
        run("prepare", format!("mkdir '{}'", remote.display())),
        {"type":"put","name":"upload","local":source,"remote_dir":remote},
        run("verify", format!("cat '{}/input'", remote.display())),
        run("fail", "printf 'expected failure' >&2; exit 7".into()),
        run("skip", format!("touch '{}'", skipped.display()))
    ]});
    let mut invalid = plan.clone();
    invalid["steps"][4]["cwd"] = serde_json::json!("relative");
    let (code, _, error) = env.cli(&args, Some(&invalid.to_string())).await;
    assert_eq!(code, 2);
    assert_eq!(error["error"]["code"], "INVALID_REQUEST");
    assert!(env.service.spaces(true).await.is_empty());
    let (code, response, error) = env.cli(&args, Some(&plan.to_string())).await;
    assert_eq!(code, 1, "{response} {error}");
    assert_eq!(response["outcome"], "stopped");
    assert_eq!(response["unsubmitted_steps"], 1);
    assert_eq!(
        response["steps"][2]["result"]["stdout"]["text"],
        "published bytes"
    );
    assert_eq!(response["steps"][3]["result"]["exit_code"], 7);
    assert!(
        response["steps"][2]["result"]["record_saved"]
            .as_bool()
            .unwrap()
    );
    assert!(!response.to_string().contains("\"lease\""));
    assert!(!skipped.exists());
    assert_eq!(env.service.spaces(true).await.len(), 1);
    let record = env
        .service
        .history_get(response["deployment"].as_str().unwrap(), 8192, true)
        .await
        .unwrap()
        .record;
    use vibe_ssh_protocol::DeploymentStepStatus as Step;
    assert_eq!(record.status, vibe_ssh_protocol::JobStatus::Failed);
    assert_eq!(
        record
            .deployment
            .iter()
            .map(|step| step.status)
            .collect::<Vec<_>>(),
        [
            Step::Succeeded,
            Step::Succeeded,
            Step::Succeeded,
            Step::Failed,
            Step::Skipped
        ]
    );
    assert_eq!(record.deployment[3].exit_code, Some(7));
    assert_eq!(
        record.deployment[2].job_record.as_deref(),
        response["steps"][2]["result"]["record"].as_str()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_stops_on_takeover_and_cannot_reuse_the_old_lease_after_return() {
    let env = Env::start().await;
    let (space, lease) = env.open("box").await;
    let entered = env.remote("entered");
    let gate = env.remote("gate");
    let skipped = env.remote("fenced");
    let plan = serde_json::json!({"steps": [
        {"type":"run","name":"in progress","command":format!(
            "touch '{}'; while [ ! -e '{}' ]; do sleep 0.05; done", entered.display(), gate.display())},
        {"type":"run","name":"fenced","command":format!("touch '{}'", skipped.display())}
    ]}).to_string();
    let args = ["batch", "--lease", &lease, "--wait", "5"];
    let ((code, response, error), ()) = tokio::join!(env.cli(&args, Some(&plan)), async {
        tokio::time::timeout(Duration::from_secs(3), async {
            while !entered.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        env.service.takeover(&space).await.unwrap();
    });
    assert_eq!(code, 1, "{response} {error}");
    assert_eq!(response["steps"][0]["result"]["status"], "running");
    assert_eq!(response["steps"][0]["result"]["control"]["owner"], "human");
    assert_eq!(response["unsubmitted_steps"], 1);
    let deployment = response["deployment"].as_str().unwrap();
    let before = env
        .service
        .history_get(deployment, 8192, true)
        .await
        .unwrap()
        .record;
    assert_eq!(before.reason.as_deref(), Some("control_changed"));
    assert_eq!(
        before.deployment[1].status,
        vibe_ssh_protocol::DeploymentStepStatus::Skipped
    );
    env.service.release(&space).await.unwrap();
    std::fs::write(&gate, b"").unwrap();
    let (code, response, error) = env.cli(&args, Some(&plan)).await;
    assert_eq!(code, 2, "{response} {error}");
    assert_eq!(response["steps"][0]["error"]["code"], "STALE_LEASE");
    assert!(!skipped.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn summary_preserves_failed_job_and_output_without_echoing_script_or_lease() {
    let env = Env::start().await;
    let (_, lease) = env.open("box").await;
    let script = "echo normal; echo diagnostic >&2; exit 7";
    let (code, summary, errors) = env
        .cli(&["run", "--summary", "--lease", &lease], Some(script))
        .await;
    assert_eq!(code, 1);
    assert_eq!(errors, Value::Null);
    assert_eq!(summary["status"], "failed");
    assert_eq!(summary["exit_code"], 7);
    assert_eq!(summary["stdout"]["text"], "normal\n");
    assert_eq!(summary["stderr"]["text"], "diagnostic\n");
    assert_eq!(summary["record_saved"], true);
    assert_eq!(summary["control"]["owner"], "agent");
    assert!(summary.get("command").is_none());
    assert!(summary["control"].get("lease").is_none());
    assert!(summary["cursor"].is_string());
    let (code, full, errors) = env.cli(&["job", "get", &text(&summary["job"])], None).await;
    assert_eq!(code, 1);
    assert_eq!(errors, Value::Null);
    assert_eq!(full["command"], script);
    assert_eq!(full["record"], summary["record"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn directory_preview_and_retained_jobs_are_private_and_survive_process_restart() {
    let sshd = Sshd::start();
    let home = sshd.path("history-home");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(
        home.join("hosts.json"),
        serde_json::to_vec(&vec![sshd.host("box", "id_plain", true)]).unwrap(),
    )
    .unwrap();
    let cli = |args: Vec<String>| {
        let home = home.clone();
        async move {
            let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_vibe-ssh"))
                .args(args)
                .env("VIBE_SSH_HOME", home)
                .output()
                .await
                .unwrap();
            let decode =
                |bytes: &[u8]| serde_json::from_slice::<Value>(bytes).unwrap_or(Value::Null);
            (
                output.status.code().unwrap(),
                decode(&output.stdout),
                decode(&output.stderr),
            )
        }
    };
    let start = || {
        tokio::process::Command::new(env!("CARGO_BIN_EXE_vibe-ssh"))
            .arg("serve")
            .env("VIBE_SSH_HOME", &home)
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap()
    };
    let wait = || async {
        for _ in 0..100 {
            if cli(vec!["status".into()]).await.0 == 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("service did not start");
    };
    let mut process = start();
    wait().await;
    let opened = cli(vec![
        "space".into(),
        "open".into(),
        "box".into(),
        "--name".into(),
        "history".into(),
    ])
    .await;
    assert_eq!(opened.0, 0, "{opened:?}");
    let space = text(&opened.1["space"]);
    let lease = text(&opened.1["control"]["lease"]);
    let project = sshd.path("project");
    let remote = sshd.path("remote-diff");
    std::fs::create_dir(&project).unwrap();
    std::fs::create_dir(&remote).unwrap();
    std::fs::write(project.join("中文.txt"), "same").unwrap();
    std::fs::write(remote.join("中文.txt"), "same").unwrap();
    let preview = cli(vec![
        "diff".into(),
        space.clone(),
        project.display().to_string(),
        remote.display().to_string(),
        "--select".into(),
        "中文.txt".into(),
    ])
    .await;
    assert_eq!(preview.0, 0, "{preview:?}");
    assert_eq!(preview.1["entries"][0]["status"], "same");
    let archived = cli(vec![
        "run".into(), "--lease".into(), lease.clone(), "--".into(),
        "printf '部署开头'; dd if=/dev/zero bs=32768 count=40 2>/dev/null; printf '完成'; printf '\\377diagnostic' >&2".into(),
    ]).await;
    assert_eq!(archived.0, 0, "{archived:?}");
    assert!(archived.1["stdout"]["total_bytes"].as_u64().unwrap() > 1_048_576);
    let archive_id = text(&archived.1["record"]);
    let marker = sshd.path("exactly-once");
    let command = format!(
        "printf 'once\\n' >> '{}'; printf 'persisted output\\n'; cat",
        marker.display().to_string().replace('\'', "'\\''")
    );
    let running = cli(vec![
        "run".into(),
        "--lease".into(),
        lease.clone(),
        "--wait".into(),
        "0".into(),
        "--".into(),
        command,
    ])
    .await;
    assert_eq!(running.0, 0, "{running:?}");
    let record = text(&running.1["record"]);
    let job = text(&running.1["job"]);
    for n in 0..100 {
        let detail = cli(vec!["history".into(), "get".into(), record.clone()]).await;
        if detail.1["stdout"]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("persisted output")
        {
            break;
        }
        assert!(n < 99, "output was never persisted: {detail:?}");
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    process.kill().await.unwrap();
    process.wait().await.unwrap();
    let mut restarted = start();
    wait().await;
    let recovered = cli(vec!["history".into(), "get".into(), record.clone()]).await;
    assert_eq!(recovered.0, 1, "unknown is not success: {recovered:?}");
    assert_eq!(recovered.1["reason"], "client_restarted");
    assert!(text(&recovered.1["stdout"]["text"]).contains("persisted output"));
    let mut offset = 0;
    let mut count = 0;
    loop {
        let page = cli(vec![
            "history".into(),
            "log".into(),
            archive_id.clone(),
            "--offset".into(),
            offset.to_string(),
            "--limit".into(),
            "65536".into(),
        ])
        .await;
        assert_eq!(page.0, 0, "{} {:?}", page.0, page.2);
        assert_eq!(page.1["complete"], true);
        let output = page.1["text"].as_str().unwrap();
        if offset == 0 {
            assert!(output.starts_with("部署开头"));
        }
        count += output.as_bytes().iter().filter(|byte| **byte == 0).count();
        offset = page.1["next_offset"].as_u64().unwrap();
        if offset == page.1["archived_bytes"].as_u64().unwrap() {
            assert!(output.ends_with("完成"));
            break;
        }
    }
    assert_eq!(count, 40 * 32768);
    let diagnostic = cli(vec![
        "history".into(),
        "log".into(),
        archive_id.clone(),
        "--stream".into(),
        "stderr".into(),
    ])
    .await;
    assert_eq!(diagnostic.1["invalid_utf8"], true);
    assert_eq!(
        std::fs::read(home.join("logs").join(format!("{archive_id}.stderr.raw"))).unwrap(),
        b"\xffdiagnostic"
    );
    assert_eq!(
        std::fs::read_to_string(marker).unwrap(),
        "once\n",
        "restart never replays a command"
    );
    let stale = cli(vec!["job".into(), "get".into(), job]).await;
    assert_eq!(stale.2["error"]["code"], "INSTANCE_MISMATCH");
    let stale = cli(vec![
        "run".into(),
        "--lease".into(),
        lease,
        "--".into(),
        "false".into(),
    ])
    .await;
    assert_eq!(stale.2["error"]["code"], "INSTANCE_MISMATCH");
    // Revoking access or removing the host hides all its retained records.
    std::fs::write(home.join("hosts.json"), "[]").unwrap();
    assert!(
        cli(vec!["history".into(), "list".into()]).await.1["records"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        cli(vec!["history".into(), "get".into(), record]).await.2["error"]["code"],
        "NOT_FOUND"
    );
    restarted.kill().await.unwrap();
    restarted.wait().await.unwrap();
    // A broken history store must reject the task before remote execution.
    std::fs::write(
        home.join("hosts.json"),
        serde_json::to_vec(&vec![sshd.host("box", "id_plain", true)]).unwrap(),
    )
    .unwrap();
    std::fs::write(home.join("history.sqlite"), b"corrupted fixture").unwrap();
    let mut broken = start();
    wait().await;
    let opened = cli(vec![
        "space".into(),
        "open".into(),
        "box".into(),
        "--name".into(),
        "refused intent".into(),
    ])
    .await;
    let refusal = cli(vec![
        "run".into(),
        "--lease".into(),
        text(&opened.1["control"]["lease"]),
        "--".into(),
        format!("touch '{}'", sshd.path("must-not-run").display()),
    ])
    .await;
    assert_eq!(refusal.0, 2, "{refusal:?}");
    assert!(!sshd.path("must-not-run").exists());
    assert_eq!(
        std::fs::read(home.join("history.sqlite")).unwrap(),
        b"corrupted fixture"
    );
    broken.kill().await.unwrap();
    broken.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_runs_uploads_and_follows_jobs_on_real_openssh() {
    let env = Env::start().await;
    let hosts = env.ok(&["hosts"]).await;
    let ids: Vec<_> = hosts
        .as_array()
        .unwrap()
        .iter()
        .map(|h| text(&h["id"]))
        .collect();
    assert_eq!(
        ids,
        ["box", "encrypted"],
        "hosts without agent access are not listed"
    );
    let (_, lease) = env.open("box").await;

    // Separate streams and a real exit code; the CLI's own exit code only says "not success".
    let (code, job, _) = env
        .cli(
            &[
                "run",
                "--lease",
                &lease,
                "--",
                "echo out; echo err >&2; exit 3",
            ],
            None,
        )
        .await;
    assert_eq!(code, 1);
    assert_eq!(
        (job["status"].as_str(), job["exit_code"].as_u64()),
        (Some("failed"), Some(3))
    );
    assert_eq!(
        (text(&job["stdout"]["text"]), text(&job["stderr"]["text"])),
        ("out\n".into(), "err\n".into())
    );

    // A working directory full of shell syntax reaches the remote shell intact.
    let odd = env.remote("it's $HOME `here`");
    std::fs::create_dir_all(&odd).unwrap();
    let job = env
        .ok(&[
            "run",
            "--lease",
            &lease,
            "--cwd",
            odd.to_str().unwrap(),
            "--",
            "pwd",
        ])
        .await;
    assert_eq!(
        text(&job["stdout"]["text"]).trim_end(),
        odd.to_str().unwrap()
    );

    // Each run is its own shell: state does not leak into the next one.
    env.ok(&["run", "--lease", &lease, "--", "cd /tmp && export LEAK=1"])
        .await;
    let job = env
        .ok(&["run", "--lease", &lease, "--", "echo \"[${LEAK:-unset}]\""])
        .await;
    assert_eq!(text(&job["stdout"]["text"]), "[unset]\n");

    // Upload a project directory, then a multi-line script from stdin that reads it back.
    let project = env.remote("../project");
    std::fs::create_dir_all(project.join("node_modules/dep")).unwrap();
    std::fs::create_dir_all(project.join("页面")).unwrap();
    std::fs::write(project.join("node_modules/dep/index.js"), "skip").unwrap();
    std::fs::write(project.join(".env"), "SECRET=1").unwrap();
    std::fs::write(project.join("页面/index.html"), "<h1>你好 🚀</h1>").unwrap();
    let www = env.remote("www");
    std::fs::create_dir_all(&www).unwrap();
    std::fs::write(www.join("existing.txt"), "keep me").unwrap();
    let www = www.to_str().unwrap();
    let put = env
        .ok(&[
            "put",
            "--lease",
            &lease,
            project.to_str().unwrap(),
            www,
            "--exclude",
            "node_modules",
            "--exclude",
            ".env",
        ])
        .await;
    assert_eq!(put["status"], "succeeded", "{put}");
    let script = "set -e\ncat 页面/index.html\necho\nls -A | LC_ALL=C sort | tr '\\n' ' '\n";
    let (code, job, err) = env
        .cli(&["run", "--lease", &lease, "--cwd", www], Some(script))
        .await;
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        text(&job["stdout"]["text"]),
        "<h1>你好 🚀</h1>\nexisting.txt 页面 "
    );

    // An upload whose sender dies midway is recorded as incomplete, never as a success.
    let big = env.remote("../big");
    std::fs::create_dir_all(&big).unwrap();
    // Real bytes: tar would store a sparse file as a hole and finish before it can be cut.
    let size: u64 = 64 * 1024 * 1024;
    std::fs::write(big.join("blob"), vec![0xa5; size as usize]).unwrap();
    let partial = env.remote("partial");
    let mut sender = std::process::Command::new(env!("CARGO_BIN_EXE_vibe-ssh"))
        .args([
            "put",
            "--lease",
            &lease,
            big.to_str().unwrap(),
            partial.to_str().unwrap(),
        ])
        .env("VIBE_SSH_HOME", env.service.home())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let cut = loop {
        let space = env.ok(&["space", "list"]).await;
        let last = space[0]["jobs"].as_array().unwrap().last().unwrap().clone();
        if text(&last["command"]).contains("partial") && partial.join("blob").exists() {
            break text(&last["job"]);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    sender.kill().unwrap();
    sender.wait().unwrap();
    let (code, first, error) = env
        .cli(
            &["job", "wait", &format!("{cut}:0:0"), "--timeout", "20"],
            None,
        )
        .await;
    assert!(code < 2, "{error}");
    let (ended, _) = env.follow(first).await;
    assert_eq!(
        (
            ended["status"].as_str(),
            ended["reason"].as_str(),
            &ended["exit_code"]
        ),
        (Some("failed"), Some("upload_incomplete"), &Value::Null)
    );
    // The remote tar pads a truncated entry to its declared size, so compare content, not length.
    let copy = std::fs::read(partial.join("blob")).unwrap();
    assert!(
        copy.len() < size as usize || copy.last() != Some(&0xa5),
        "the copy is partial"
    );

    // A long job outlives the first wait and is followed by cursor without re-running it.
    let marker = env.remote("ran-once");
    let command = format!(
        "echo x >> {0}; echo first; sleep 2; echo second",
        marker.display()
    );
    let job = env
        .ok(&["run", "--lease", &lease, "--wait", "1", "--", &command])
        .await;
    assert_eq!(job["status"], "running");
    assert_eq!(
        job["exit_code"],
        Value::Null,
        "a running job has no exit code"
    );
    let (done, seen) = env.follow(job).await;
    assert_eq!(
        (done["status"].as_str(), done["exit_code"].as_u64()),
        (Some("succeeded"), Some(0))
    );
    assert_eq!(seen, "first\nsecond\n");
    assert_eq!(std::fs::read_to_string(marker).unwrap(), "x\n");

    // A real terminal: the prompt arrives, input is answered, output is merged.
    let job = env
        .ok(&[
            "run",
            "--lease",
            &lease,
            "--pty",
            "--wait",
            "1",
            "--",
            "printf 'name? '; read n; echo \"hi $n\"",
        ])
        .await;
    assert_eq!(
        (job["status"].as_str(), job["stream_mode"].as_str()),
        (Some("running"), Some("merged"))
    );
    assert!(text(&job["stdout"]["text"]).contains("name? "));
    env.ok(&[
        "job",
        "input",
        "--lease",
        &lease,
        &text(&job["job"]),
        "--",
        "bob",
    ])
    .await;
    let (done, seen) = env.follow(job).await;
    assert_eq!(done["status"], "succeeded");
    assert!(seen.contains("hi bob"), "{seen:?}");

    // Interrupt is a request; the job ends when the remote process actually dies.
    let job = env
        .ok(&["run", "--lease", &lease, "--wait", "1", "--", "sleep 30"])
        .await;
    let asked = env
        .ok(&["job", "interrupt", "--lease", &lease, &text(&job["job"])])
        .await;
    assert_eq!(asked["interrupt_requested"], true);
    let (done, _) = env.follow(asked).await;
    assert_ne!(done["status"], "succeeded");
    assert!(
        done["elapsed_ms"].as_u64().unwrap() < 20_000,
        "the process was really stopped"
    );

    // An encrypted key works with a passphrase the agent never sees.
    let (_, other) = env.open("encrypted").await;
    assert_eq!(
        env.ok(&["run", "--lease", &other, "--", "true"]).await["status"],
        "succeeded"
    );

    env.ok(&["space", "close", "--lease", &lease]).await;
    let (code, _, err) = env
        .cli(&["run", "--lease", &lease, "--", "true"], None)
        .await;
    assert_eq!(
        (code, err["error"]["code"].as_str()),
        (2, Some("NOT_FOUND"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn takeover_fences_the_agent_across_the_real_socket() {
    let env = Env::start().await;
    let (space, lease) = env.open("box").await;
    let job = env
        .ok(&[
            "run", "--lease", &lease, "--pty", "--wait", "1", "--", "cat",
        ])
        .await;
    let id = text(&job["job"]);
    env.ok(&["job", "input", "--lease", &lease, &id, "--", "from agent"])
        .await;

    // The user's own terminal on the same host is a separate space agents cannot see.
    let mine = env
        .service
        .open("mine", "my terminal", "me", Owner::Human)
        .await
        .unwrap();
    let listed = env.ok(&["space", "list"]).await;
    assert_eq!(listed.as_array().unwrap().len(), 1);
    let (code, _, err) = env.cli(&["space", "get", &mine.space], None).await;
    assert_eq!(
        (code, err["error"]["code"].as_str()),
        (2, Some("NOT_FOUND"))
    );
    let (code, _, err) = env
        .cli(&["space", "open", "mine", "--name", "x"], None)
        .await;
    assert_eq!(
        (code, err["error"]["code"].as_str()),
        (2, Some("HOST_NOT_AUTHORIZED"))
    );

    env.service.takeover(&space).await.unwrap();
    let forbidden = env.remote("must-not-exist");
    let touch = format!("touch {}", forbidden.display());
    for args in [
        vec![
            "job",
            "input",
            "--lease",
            &lease,
            &id,
            "--",
            "late agent input",
        ],
        vec!["run", "--lease", &lease, "--", &touch],
        vec!["job", "interrupt", "--lease", &lease, &id],
        vec!["space", "close", "--lease", &lease],
    ] {
        let (code, _, err) = env.cli(&args, None).await;
        assert_eq!(
            (code, err["error"]["code"].as_str()),
            (2, Some("CONTROL_REVOKED")),
            "{args:?}"
        );
    }
    // Reading stays available, and the takeover did not kill the process.
    let seen = env.ok(&["job", "get", &id]).await;
    assert_eq!(
        (seen["status"].as_str(), seen["control"]["owner"].as_str()),
        (Some("running"), Some("human"))
    );
    assert_eq!(seen["control"]["lease"], Value::Null);
    env.service
        .input(By::Human(&space), &id, b"from human\n", false)
        .await
        .unwrap();

    env.service.release(&space).await.unwrap();
    let (code, _, err) = env
        .cli(
            &["job", "input", "--lease", &lease, &id, "--", "stale"],
            None,
        )
        .await;
    assert_eq!(
        (code, err["error"]["code"].as_str()),
        (2, Some("STALE_LEASE"))
    );
    let fresh = text(&env.ok(&["space", "get", &space]).await["control"]["lease"]);
    env.ok(&["job", "input", "--lease", &fresh, &id, "--", "agent again"])
        .await;
    env.ok(&[
        "job", "input", "--lease", &fresh, &id, "--raw", "--", "\u{4}",
    ])
    .await;
    let (done, _) = env.follow(env.ok(&["job", "get", &id]).await).await;
    assert_eq!(done["status"], "succeeded");
    let all = text(&env.ok(&["job", "get", &id, "--tail", "4096"]).await["stdout"]["text"]);
    for line in ["from agent", "from human", "agent again"] {
        assert!(all.contains(line), "{all:?}");
    }
    assert!(
        !all.contains("late agent input") && !all.contains("stale"),
        "{all:?}"
    );
    assert!(
        !forbidden.exists(),
        "a refused step must never reach the server"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn host_revocation_preserves_observation_and_fences_existing_spaces() {
    let env = Env::start().await;
    let (space, lease) = env.open("box").await;
    let (_, second) = env.open("box").await;
    let (_, other) = env.open("encrypted").await;
    let personal = env
        .service
        .open("box", "personal", "me", Owner::Human)
        .await
        .unwrap();
    let job = env
        .ok(&[
            "run", "--lease", &lease, "--pty", "--wait", "0", "--", "cat",
        ])
        .await;
    let id = text(&job["job"]);
    let mut hosts = env.service.load_hosts().unwrap();
    hosts[0].agent_access = false;
    env.service.save_hosts(&hosts).await.unwrap();
    assert!(
        env.ok(&["hosts"])
            .await
            .as_array()
            .unwrap()
            .iter()
            .all(|host| host["id"] != "box")
    );
    let forbidden = env.remote("revoked-command");
    let command = format!("touch {}", forbidden.display());
    let local = env.remote("project");
    std::fs::create_dir(&local).unwrap();
    let remote = local.to_str().unwrap();
    for (args, error) in [
        (
            vec!["space", "open", "box", "--name", "denied"],
            "HOST_NOT_AUTHORIZED",
        ),
        (vec!["diff", &space, remote, remote], "HOST_NOT_AUTHORIZED"),
        (
            vec!["run", "--lease", &lease, "--", &command],
            "CONTROL_REVOKED",
        ),
        (
            vec!["run", "--lease", &second, "--", &command],
            "CONTROL_REVOKED",
        ),
        (
            vec!["job", "input", "--lease", &lease, &id, "--", "late"],
            "CONTROL_REVOKED",
        ),
        (
            vec!["job", "interrupt", "--lease", &lease, &id],
            "CONTROL_REVOKED",
        ),
        (
            vec!["space", "handoff", "--lease", &lease, "--reason", "late"],
            "CONTROL_REVOKED",
        ),
        (vec!["space", "close", "--lease", &lease], "CONTROL_REVOKED"),
    ] {
        let (code, _, reply) = env.cli(&args, None).await;
        assert_eq!(
            (code, reply["error"]["code"].as_str()),
            (2, Some(error)),
            "{args:?}: {reply}"
        );
    }
    assert!(!forbidden.exists());
    let seen = env.ok(&["job", "get", &id]).await;
    assert_eq!(seen["status"], "running");
    assert_eq!(seen["control"]["owner"], "human");
    assert_eq!(seen["control"]["lease"], Value::Null);
    assert_eq!(
        env.service.release(&space).await.unwrap_err().code,
        vibe_ssh_core::ErrorCode::HostNotAuthorized
    );
    env.service.interrupt(By::Human(&space), &id).await.unwrap();
    let finished = env.follow(seen).await.0;
    assert_eq!(finished["status"], "failed");
    assert_eq!(
        env.ok(&["run", "--lease", &other, "--", "true"]).await["status"],
        "succeeded"
    );
    let mine = env
        .service
        .run(By::Human(&personal.space), Some("true".into()), None, None)
        .await
        .unwrap();
    assert_eq!(
        env.service
            .job_settled(&mine, 2000, 64, false)
            .await
            .unwrap()
            .exit_code,
        Some(0)
    );
    hosts[0].agent_access = true;
    env.service.save_hosts(&hosts).await.unwrap();
    assert_eq!(
        env.ok(&["space", "get", &space]).await["control"]["owner"],
        "human"
    );
    let fresh = env
        .service
        .release(&space)
        .await
        .unwrap()
        .control
        .lease
        .unwrap();
    let (code, _, error) = env
        .cli(&["run", "--lease", &lease, "--", &command], None)
        .await;
    assert_eq!(
        (code, error["error"]["code"].as_str()),
        (2, Some("STALE_LEASE"))
    );
    assert!(!forbidden.exists());
    assert_eq!(
        env.ok(&["run", "--lease", &fresh, "--", "true"]).await["status"],
        "succeeded"
    );
    env.service
        .update_hosts(|hosts| hosts.retain(|host| host.id != "box"))
        .await
        .unwrap();
    let (code, _, error) = env
        .cli(&["run", "--lease", &fresh, "--", &command], None)
        .await;
    assert_eq!(
        (code, error["error"]["code"].as_str()),
        (2, Some("CONTROL_REVOKED"))
    );
    assert!(!forbidden.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_remote_reads_are_bounded_private_and_do_not_create_jobs() {
    use std::os::unix::fs::symlink;

    let env = Env::start().await;
    let (space, _) = env.open("box").await;
    let personal = env
        .service
        .open("box", "personal", "me", Owner::Human)
        .await
        .unwrap();
    let root = env.remote("只读目录 '含空格'");
    std::fs::create_dir(&root).unwrap();
    let text_path = root.join("配置.txt");
    let original = "你好🚀\r\n";
    std::fs::write(&text_path, original).unwrap();
    std::fs::write(root.join("binary"), [0xff, 0, 7]).unwrap();
    std::fs::write(root.join("oversize"), vec![b'x'; 65_537]).unwrap();
    symlink(&text_path, root.join("link")).unwrap();
    let fifo = root.join("pipe");
    assert!(
        std::process::Command::new("/usr/bin/mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let root_path = root.to_str().unwrap();
    let text_path = text_path.to_str().unwrap();
    let one = env
        .ok(&["files", "list", &space, root_path, "--limit", "1"])
        .await;
    assert_eq!(one["entries"].as_array().unwrap().len(), 1);
    assert_eq!(one["truncated"], true);
    let all = env
        .ok(&["files", "list", &space, root_path, "--limit", "5"])
        .await;
    let entries = all["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 5);
    assert_eq!(all["truncated"], false);
    assert!(
        entries
            .iter()
            .any(|entry| entry["name"] == "link" && entry["kind"] == "symlink")
    );
    assert!(
        entries
            .iter()
            .any(|entry| entry["name"] == "配置.txt" && entry["kind"] == "file")
    );
    env.service.takeover(&space).await.unwrap();
    let exact_limit = original.len().to_string();
    assert_eq!(
        env.ok(&[
            "files",
            "read",
            &space,
            text_path,
            "--max-bytes",
            &exact_limit
        ])
        .await,
        serde_json::json!({"path": text_path, "text": original, "bytes": original.len()})
    );
    for leaf in ["binary", "oversize", "link", "pipe"] {
        let path = root.join(leaf);
        let (code, _, error) = env
            .cli(
                &[
                    "files",
                    "read",
                    &space,
                    path.to_str().unwrap(),
                    "--max-bytes",
                    "65536",
                ],
                None,
            )
            .await;
        assert_eq!(
            (code, error["error"]["code"].as_str()),
            (2, Some("INVALID_REQUEST")),
            "{leaf}: {error}"
        );
    }
    for args in [
        vec!["files", "list", &space, root_path, "--limit", "1001"],
        vec!["files", "read", &space, text_path, "--max-bytes", "65537"],
        vec!["files", "read", &space, text_path, "--max-bytes", "1"],
    ] {
        let (code, _, error) = env.cli(&args, None).await;
        assert_eq!(
            (code, error["error"]["code"].as_str()),
            (2, Some("INVALID_REQUEST"))
        );
    }
    let assert_denied = |code, error: Value, expected| {
        assert_eq!((code, error["error"]["code"].as_str()), (2, Some(expected)))
    };
    for args in [
        vec!["files", "list", &personal.space, root_path],
        vec!["files", "read", &personal.space, text_path],
        vec!["resources", &personal.space],
    ] {
        let (code, _, error) = env.cli(&args, None).await;
        assert_denied(code, error, "NOT_FOUND");
    }
    assert_eq!(
        env.ok(&["space", "get", &space]).await["jobs"],
        serde_json::json!([])
    );
    assert_eq!(
        env.ok(&["history", "list"]).await["records"],
        serde_json::json!([])
    );
    assert_eq!(env.service.open_documents(), 0);
    assert_eq!(
        env.ok(&["status"]).await["capabilities"],
        serde_json::json!([
            "files_list",
            "files_read",
            "resources",
            "deployment_records",
            "history_log"
        ])
    );
    let mut hosts = env.service.load_hosts().unwrap();
    hosts[0].agent_access = false;
    env.service.save_hosts(&hosts).await.unwrap();
    for args in [
        vec!["files", "list", &space, root_path],
        vec!["files", "read", &space, text_path],
        vec!["resources", &space],
    ] {
        let (code, _, error) = env.cli(&args, None).await;
        assert_denied(code, error, "HOST_NOT_AUTHORIZED");
    }
    assert_eq!(std::fs::read_to_string(text_path).unwrap(), original);
}

#[tokio::test(flavor = "multi_thread")]
async fn local_ssh_agent_signs_only_after_the_pinned_host_is_verified() {
    let sshd = Sshd::start();
    let home = sshd.path("agent-test");
    std::fs::create_dir(&home).unwrap();
    let socket = sshd.path("keys.sock");
    let mut key_agent = tokio::process::Command::new("/usr/bin/ssh-agent")
        .args(["-D", "-a"])
        .arg(&socket)
        .kill_on_drop(true)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    for _ in 0..100 {
        if socket.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let cli = env!("CARGO_BIN_EXE_vibe-ssh");
    let mut host = sshd.host("box", "id_plain", true);
    host.ssh_agent = true;
    host.identity_file = None;
    let mut bad = host.clone();
    bad.id = "bad".into();
    bad.name = "bad".into();
    bad.host_key_sha256 = "SHA256:wrong".into();
    std::fs::write(
        home.join("hosts.json"),
        serde_json::to_vec(&[host, bad]).unwrap(),
    )
    .unwrap();
    let mut service = tokio::process::Command::new(cli)
        .arg("serve")
        .env("VIBE_SSH_HOME", &home)
        .env("SSH_AUTH_SOCK", &socket)
        .kill_on_drop(true)
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    for _ in 0..100 {
        if home.join("agent.sock").exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let command = |args: &[&str]| {
        let mut command = tokio::process::Command::new(cli);
        command.args(args).env("VIBE_SSH_HOME", &home);
        command
    };
    let open = |host| command(&["space", "open", host, "--name", "key agent login"]);
    let rejected = open("bad").output().await.unwrap();
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("HOST_KEY_MISMATCH"));
    let empty = open("box").output().await.unwrap();
    assert!(String::from_utf8_lossy(&empty.stderr).contains("AUTHENTICATION_FAILED"));
    let added = tokio::process::Command::new("/usr/bin/ssh-add")
        .arg(sshd.path("id_plain"))
        .env("SSH_AUTH_SOCK", &socket)
        .output()
        .await
        .unwrap();
    assert!(added.status.success());
    let opened = open("box").output().await.unwrap();
    assert!(
        opened.status.success(),
        "{}",
        String::from_utf8_lossy(&opened.stderr)
    );
    let view: Value = serde_json::from_slice(&opened.stdout).unwrap();
    let lease = text(&view["control"]["lease"]);
    let job = command(&["run", "--lease", &lease, "--", "printf SSH_AGENT_OK"])
        .output()
        .await
        .unwrap();
    assert!(job.status.success());
    let job: Value = serde_json::from_slice(&job.stdout).unwrap();
    assert_eq!(job["stdout"]["text"], "SSH_AGENT_OK");
    service.kill().await.unwrap();
    key_agent.kill().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn jump_and_local_forward_use_separate_verified_loopback_channels() {
    use tokio::io::AsyncReadExt;

    let env = Env::start().await;
    let target = Sshd::start();
    let intermediate = Sshd::start();
    let mut hop = intermediate.host("middle", "id_plain", false);
    hop.jump_host = Some("mine".into());
    let mut host = target.host("target", "id_plain", true);
    host.jump_host = Some("middle".into());
    let mut wrong = host.clone();
    wrong.name = "wrong".into();
    wrong.id = "wrong".into();
    wrong.host_key_sha256 = "SHA256:wrong".into();
    env.service
        .update_hosts(|hosts| hosts.extend([hop.clone(), host.clone(), wrong]))
        .await
        .unwrap();
    let probe = env
        .service
        .probe_host(&host.host, host.port, Some("mine"), None)
        .await
        .unwrap();
    assert_eq!(probe.1, target.fingerprint);
    let probe = env
        .service
        .probe_host(&host.host, host.port, Some("middle"), None)
        .await
        .unwrap();
    assert_eq!(probe.1, target.fingerprint);
    env.service
        .update_hosts(|hosts| {
            hosts
                .iter_mut()
                .find(|item| item.id == "middle")
                .unwrap()
                .host_key_sha256 = "SHA256:wrong-intermediate".into();
        })
        .await
        .unwrap();
    let (code, _, error) = env
        .cli(&["space", "open", "target", "--name", "reject jump"], None)
        .await;
    assert_eq!(
        (code, error["error"]["code"].as_str()),
        (2, Some("HOST_KEY_MISMATCH"))
    );
    let mut cycle = hop.clone();
    cycle.jump_host = Some("target".into());
    assert_eq!(
        env.service.validate_route(&cycle).unwrap_err().code,
        vibe_ssh_core::ErrorCode::InvalidRequest
    );
    cycle.jump_host = Some("missing".into());
    assert_eq!(
        env.service.validate_route(&cycle).unwrap_err().code,
        vibe_ssh_core::ErrorCode::InvalidRequest
    );
    env.service
        .update_hosts(|hosts| *hosts.iter_mut().find(|item| item.id == "middle").unwrap() = hop)
        .await
        .unwrap();
    let (code, _, error) = env
        .cli(&["space", "open", "wrong", "--name", "reject target"], None)
        .await;
    assert_eq!(
        (code, error["error"]["code"].as_str()),
        (2, Some("HOST_KEY_MISMATCH"))
    );
    let (space, lease) = env.open("target").await;
    let job = env
        .ok(&["run", "--lease", &lease, "--", "printf JUMP_OK"])
        .await;
    assert_eq!(job["stdout"]["text"], "JUMP_OK");
    assert!(
        env.service
            .start_forward(&space, 0, "127.0.0.1".into(), 1)
            .await
            .is_err()
    );
    let personal = env
        .service
        .open("target", "personal", "me", Owner::Human)
        .await
        .unwrap();
    let endpoint = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let remote_port = endpoint.local_addr().unwrap().port();
    let forwarded = env
        .service
        .start_forward(&personal.space, 0, "127.0.0.1".into(), remote_port)
        .await
        .unwrap();
    let address = ("127.0.0.1", forwarded.local_port);
    assert!(
        env.service
            .start_forward(
                &personal.space,
                forwarded.local_port,
                "127.0.0.1".into(),
                remote_port
            )
            .await
            .is_err()
    );
    let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
    let (mut remote, _) = tokio::time::timeout(Duration::from_secs(3), endpoint.accept())
        .await
        .unwrap()
        .unwrap();
    client.write_all(b"ping").await.unwrap();
    let mut bytes = [0; 4];
    tokio::time::timeout(Duration::from_secs(3), remote.read_exact(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&bytes, b"ping");
    remote.write_all(b"pong").await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), client.read_exact(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&bytes, b"pong");
    env.service
        .stop_forward(&personal.space, &forwarded.id)
        .await
        .unwrap();
    assert!(tokio::net::TcpStream::connect(address).await.is_err());
    assert!(
        env.service
            .list_forwards(&personal.space)
            .await
            .unwrap()
            .is_empty()
    );
    let next = env
        .service
        .start_forward(&personal.space, 0, "127.0.0.1".into(), remote_port)
        .await
        .unwrap();
    env.service.close(By::Human(&personal.space)).await.unwrap();
    assert!(
        tokio::net::TcpStream::connect(("127.0.0.1", next.local_port))
            .await
            .is_err()
    );
    env.service
        .update_hosts(|hosts| {
            hosts
                .iter_mut()
                .find(|host| host.id == "mine")
                .unwrap()
                .host_key_sha256 = "SHA256:wrong".into()
        })
        .await
        .unwrap();
    assert_eq!(
        env.service
            .probe_host(&host.host, host.port, Some("mine"), None)
            .await
            .unwrap_err()
            .code,
        vibe_ssh_core::ErrorCode::HostKeyMismatch
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dynamic_socks_forwards_tcp_and_refuses_unavailable_methods() {
    use tokio::{
        io::AsyncReadExt,
        net::{TcpListener, TcpStream},
    };

    tokio::time::timeout(Duration::from_secs(10), async {
        let env = Env::start().await;
        let (agent, _) = env.open("box").await;
        assert!(env.service.start_dynamic_forward(&agent, 0).await.is_err());
        let space = env
            .service
            .open("box", "socks", "me", Owner::Human)
            .await
            .unwrap()
            .space;
        let remote = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let forward = env.service.start_dynamic_forward(&space, 0).await.unwrap();
        let address = ("127.0.0.1", forward.local_port);
        let mut refused = TcpStream::connect(address).await.unwrap();
        refused.write_all(&[5, 1, 2]).await.unwrap();
        assert_eq!(refused.read_u16().await.unwrap(), 0x05ff);
        let mut bind = TcpStream::connect(address).await.unwrap();
        bind.write_all(&[5, 1, 0]).await.unwrap();
        assert_eq!(bind.read_u16().await.unwrap(), 0x0500);
        bind.write_all(&[5, 2, 0, 1]).await.unwrap();
        let mut rejected = [0; 10];
        bind.read_exact(&mut rejected).await.unwrap();
        assert_eq!(rejected[1], 7);
        let mut client = TcpStream::connect(address).await.unwrap();
        client.write_all(&[5, 1, 0]).await.unwrap();
        assert_eq!(client.read_u16().await.unwrap(), 0x0500);
        let host = b"127.0.0.1";
        client
            .write_all(&[5, 1, 0, 3, host.len() as u8])
            .await
            .unwrap();
        client.write_all(host).await.unwrap();
        client
            .write_u16(remote.local_addr().unwrap().port())
            .await
            .unwrap();
        let mut reply = [0; 10];
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(&reply[..4], &[5, 0, 0, 1]);
        let (mut target, _) = remote.accept().await.unwrap();
        let mut payload = [0; 4];
        client.write_all(b"ping").await.unwrap();
        target.read_exact(&mut payload).await.unwrap();
        assert_eq!(&payload, b"ping");
        target.write_all(b"pong").await.unwrap();
        client.read_exact(&mut payload).await.unwrap();
        assert_eq!(&payload, b"pong");
        env.service.stop_forward(&space, &forward.id).await.unwrap();
        assert!(TcpStream::connect(address).await.is_err());
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn remote_forward_only_relays_registered_loopback_targets_and_releases_ports() {
    use tokio::{
        io::AsyncReadExt,
        net::{TcpListener, TcpStream},
    };

    tokio::time::timeout(Duration::from_secs(10), async {
        let env = Env::start().await;
        let local = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let local_port = local.local_addr().unwrap().port();
        let (agent, _) = env.open("box").await;
        assert!(
            env.service
                .start_remote_forward(&agent, 0, local_port)
                .await
                .is_err()
        );
        let space = env
            .service
            .open("box", "reverse tunnel", "me", Owner::Human)
            .await
            .unwrap()
            .space;
        let rule = env
            .service
            .start_remote_forward(&space, 0, local_port)
            .await
            .unwrap();
        let remote_port = serde_json::to_value(&rule).unwrap()["remote_port"]
            .as_u64()
            .unwrap() as u16;
        let address = ("127.0.0.1", remote_port);
        let mut remote = TcpStream::connect(address).await.unwrap();
        let (mut target, _) = local.accept().await.unwrap();
        remote.write_all(b"ping").await.unwrap();
        let mut payload = [0; 4];
        target.read_exact(&mut payload).await.unwrap();
        assert_eq!(&payload, b"ping");
        target.write_all(b"pong").await.unwrap();
        remote.read_exact(&mut payload).await.unwrap();
        assert_eq!(&payload, b"pong");
        assert!(
            env.service
                .start_remote_forward(&space, remote_port, local_port)
                .await
                .is_err()
        );
        env.service.stop_forward(&space, &rule.id).await.unwrap();
        assert!(TcpStream::connect(address).await.is_err());
        assert_eq!(target.read(&mut payload).await.unwrap(), 0);
        env.service
            .start_remote_forward(&space, remote_port, local_port)
            .await
            .unwrap();
        env.service.close(By::Human(&space)).await.unwrap();
        for _ in 0..20 {
            if TcpStream::connect(address).await.is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("the remote listener was still open after the SSH connection closed");
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_serve_answers_and_a_missing_service_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &'static [&'static str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_vibe-ssh"))
            .args(args)
            .env("VIBE_SSH_HOME", dir.path())
            .output()
            .unwrap()
    };
    let down = run(&["status"]);
    assert_eq!(down.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&down.stderr).contains("SERVICE_UNAVAILABLE"));
    assert!(
        String::from_utf8_lossy(&run(&["skill"]).stdout).contains("[vibe-ssh:skill] end of guide")
    );
    let flow = run(&["skill", "compose"]);
    assert!(String::from_utf8_lossy(&flow.stdout).contains("docker compose -p APP"));
    assert_eq!(run(&["skill", "kubernetes"]).status.code(), Some(2));

    let mut server = std::process::Command::new(env!("CARGO_BIN_EXE_vibe-ssh"))
        .arg("serve")
        .env("VIBE_SSH_HOME", dir.path())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut up = run(&["status"]);
    for _ in 0..100 {
        if up.status.success() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        up = run(&["status"]);
    }
    let status: Value = serde_json::from_slice(&up.stdout).unwrap();
    assert_eq!(status["protocol_version"], 2);
    assert_eq!(run(&["hosts"]).stdout, b"[]\n");
    // A client speaking another protocol version is refused before anything is parsed further.
    let mut raw = tokio::net::UnixStream::connect(dir.path().join("agent.sock"))
        .await
        .unwrap();
    raw.write_all(b"{\"v\":1,\"call\":{\"method\":\"hosts\"}}\n")
        .await
        .unwrap();
    let mut reply = String::new();
    tokio::io::AsyncReadExt::read_to_string(&mut raw, &mut reply)
        .await
        .unwrap();
    assert!(reply.contains("VERSION_MISMATCH"), "{reply}");
    let mode = |path: &Path| {
        std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(path).unwrap().permissions())
            & 0o777
    };
    assert_eq!(
        (mode(dir.path()), mode(&dir.path().join("agent.sock"))),
        (0o700, 0o600)
    );

    let second = run(&["serve"]);
    assert_eq!(
        second.status.code(),
        Some(2),
        "one service per home directory"
    );
    server.kill().unwrap();
    server.wait().unwrap();
}
