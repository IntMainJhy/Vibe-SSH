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
    let script = "set -e\ncat 页面/index.html\necho\nls -A | sort | tr '\\n' ' '\n";
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
    let (code, ended, _) = env
        .cli(
            &["job", "wait", &format!("{cut}:0:0"), "--timeout", "20"],
            None,
        )
        .await;
    assert_eq!(code, 1, "{ended}");
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
