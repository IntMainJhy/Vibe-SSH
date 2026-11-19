//! End-to-end against a real Linux server: the local Debian VM from `scripts/linux-vm.sh`.
//! Skipped unless `VIBE_SSH_LINUX_VM` points at that VM's directory (`target/vm`).
//!
//!   scripts/linux-vm.sh up
//!   VIBE_SSH_LINUX_VM="$PWD/target/vm" cargo test -p vibe-ssh-cli --test linux -- --nocapture

use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use serde_json::Value;
use tokio::io::AsyncWriteExt;
use vibe_ssh_core::{By, Host, Service, serve};

struct Vm {
    dir: PathBuf,
    home: tempfile::TempDir,
    service: Arc<Service>,
    /// A fresh remote directory per test run, removed at the end.
    work: String,
}

fn script(dir: &Path, args: &[&str]) -> String {
    let root = dir.parent().and_then(Path::parent).expect("target/vm");
    let output = std::process::Command::new(root.join("scripts/linux-vm.sh"))
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "linux-vm.sh {args:?}");
    String::from_utf8(output.stdout).unwrap()
}

impl Vm {
    async fn start() -> Option<Self> {
        let Some(dir) = std::env::var_os("VIBE_SSH_LINUX_VM").map(PathBuf::from) else {
            panic!("set VIBE_SSH_LINUX_VM to run against the Linux VM");
        };
        let hosts: Vec<Host> = serde_json::from_str(&script(&dir, &["hosts"])).unwrap();
        let password = script(&dir, &["password"]).trim().to_owned();
        let home = tempfile::tempdir().unwrap();
        // The app supplies these from the Keychain; here they come from the VM's own files.
        let service = Service::new(
            home.path().to_owned(),
            Box::new(move |_| Some(password.clone())),
        );
        service.save_hosts(&hosts).await.unwrap();
        serve(service.clone()).await.unwrap();
        let work = format!("/tmp/vibe-ssh-test-{}", std::process::id());
        Some(Self {
            dir,
            home,
            service,
            work,
        })
    }

    async fn cli(&self, args: &[&str], stdin: Option<&str>) -> (i32, Value, Value) {
        let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_vibe-ssh"))
            .args(args)
            .env("VIBE_SSH_HOME", self.home.path())
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
            child
                .stdin
                .take()
                .unwrap()
                .write_all(text.as_bytes())
                .await
                .unwrap();
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
                "space",
                "open",
                host,
                "--name",
                "linux e2e",
                "--actor",
                "e2e",
            ])
            .await;
        (text(&view["space"]), text(&view["control"]["lease"]))
    }

    /// Runs a command and returns its trimmed stdout, requiring success.
    async fn sh(&self, lease: &str, command: &str) -> String {
        let job = self.ok(&["run", "--lease", lease, "--", command]).await;
        assert_eq!(job["status"], "succeeded", "{command}: {job}");
        text(&job["stdout"]["text"]).trim_end().to_owned()
    }

    /// Follows a job by cursor until `done` holds for the collected stdout or the job ends.
    async fn follow(&self, mut job: Value, done: impl Fn(&str) -> bool) -> (Value, String) {
        let mut seen = text(&job["stdout"]["text"]);
        for _ in 0..60 {
            if job["status"] != "running" || done(&seen) {
                return (job, seen);
            }
            job = self
                .ok_any(&["job", "wait", &text(&job["cursor"]), "--timeout", "5"])
                .await;
            seen.push_str(job["stdout"]["text"].as_str().unwrap());
        }
        panic!("job never got there: {job} {seen:?}");
    }

    /// Like `ok`, but a finished job that failed (CLI exit 1) is still a valid answer.
    async fn ok_any(&self, args: &[&str]) -> Value {
        let (code, out, err) = self.cli(args, None).await;
        assert!(code < 2, "{args:?}: {err}");
        out
    }
}

fn text(value: &Value) -> String {
    value
        .as_str()
        .unwrap_or_else(|| panic!("expected a string, got {value}"))
        .to_owned()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires VIBE_SSH_LINUX_VM and the disposable Debian VM"]
async fn linux_remote_forward_is_a_loopback_listener_on_the_guest() {
    use tokio::{io::AsyncReadExt, net::TcpListener};
    use vibe_ssh_protocol::Owner;

    let Some(vm) = Vm::start().await else { return };
    let (_, observer) = vm.open("vm").await;
    let personal = vm
        .service
        .open("vm", "remote loopback", "me", Owner::Human)
        .await
        .unwrap()
        .space;
    let target = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let rule = vm
        .service
        .start_remote_forward(&personal, 0, target.local_addr().unwrap().port())
        .await
        .unwrap();
    let remote_port = serde_json::to_value(&rule).unwrap()["remote_port"]
        .as_u64()
        .unwrap();
    let listening = format!("ss -H -ltn 'sport = :{remote_port}' | awk '{{ print $4 }}'");
    assert_eq!(
        vm.sh(&observer, &listening).await,
        format!("127.0.0.1:{remote_port}")
    );
    let request = format!("curl --noproxy '*' -fsS --max-time 3 http://127.0.0.1:{remote_port}/");
    let (body, ()) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(vm.sh(&observer, &request), async {
            let (mut stream, _) = target.accept().await.unwrap();
            let mut bytes = [0; 1024];
            let read = stream.read(&mut bytes).await.unwrap();
            assert!(bytes[..read].starts_with(b"GET /"));
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\nrelay test",
                )
                .await
                .unwrap();
        })
    })
    .await
    .unwrap();
    assert_eq!(body, "relay test");
    vm.service.stop_forward(&personal, &rule.id).await.unwrap();
    assert_eq!(vm.sh(&observer, &listening).await, "");
    vm.service.close(By::Human(&personal)).await.unwrap();
    vm.ok(&["space", "close", "--lease", &observer]).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires VIBE_SSH_LINUX_VM and the disposable Debian VM"]
async fn linux_server_exec_pty_upload_interrupt_and_credentials() {
    let Some(vm) = Vm::start().await else { return };
    let (space, lease) = vm.open("vm").await;
    let work = vm.work.as_str();
    assert_eq!(vm.sh(&lease, "uname -s; id -un").await, "Linux\ndeploy");

    let sample = vm.ok(&["resources", &space]).await;
    assert!(
        sample["cpu_percent"]
            .as_f64()
            .is_some_and(|cpu| (0.0..=100.0).contains(&cpu))
    );
    assert!(sample["cores"].as_u64().unwrap() > 0);
    assert!(
        sample["memory_available"].as_u64().unwrap() <= sample["memory_total"].as_u64().unwrap()
    );
    assert!(sample["disk_used"].as_u64().unwrap() <= sample["disk_total"].as_u64().unwrap());
    assert_eq!(sample["load"].as_array().unwrap().len(), 3);
    assert!(
        sample["mounts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|mount| mount["path"] == "/" && mount["total"] == sample["disk_total"])
    );
    assert_eq!(
        vm.ok(&["space", "get", &space]).await["jobs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // Exit codes, separate streams, and a signal death reported as a signal.
    let (code, job, _) = vm
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
    assert_eq!((code, job["exit_code"].as_u64()), (1, Some(3)));
    assert_eq!(
        (text(&job["stdout"]["text"]), text(&job["stderr"]["text"])),
        ("out\n".into(), "err\n".into())
    );
    let (_, killed, _) = vm
        .cli(&["run", "--lease", &lease, "--", "kill -TERM $$"], None)
        .await;
    assert_eq!(
        (killed["status"].as_str(), killed["reason"].as_str()),
        (Some("failed"), Some("exit_signal"))
    );
    assert_eq!(killed["exit_code"], Value::Null);

    // A working directory full of shell syntax, created and entered through the tool itself.
    let odd = format!("{work}/it's $HOME `here` 目录");
    vm.sh(
        &lease,
        &format!("mkdir -p \"{work}\" && mkdir -p '{work}/it'\\''s $HOME `here` 目录'"),
    )
    .await;
    let job = vm
        .ok(&["run", "--lease", &lease, "--cwd", &odd, "--", "pwd"])
        .await;
    assert_eq!(text(&job["stdout"]["text"]).trim_end(), odd);

    // Upload: content identical on the Linux side, excluded paths absent, existing files kept.
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join("node_modules/dep")).unwrap();
    std::fs::create_dir_all(project.path().join("页面")).unwrap();
    std::fs::write(project.path().join("node_modules/dep/index.js"), "skip").unwrap();
    std::fs::write(project.path().join(".env"), "SECRET=1").unwrap();
    std::fs::write(project.path().join("页面/index.html"), "<h1>你好 🚀</h1>\n").unwrap();
    let blob: Vec<u8> = (0..5_000_000u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8)
        .collect();
    std::fs::write(project.path().join("blob.bin"), &blob).unwrap();
    let www = format!("{work}/www");
    vm.sh(
        &lease,
        &format!("mkdir -p {www} && echo keep > {www}/existing.txt"),
    )
    .await;
    let put = vm
        .ok(&[
            "put",
            "--lease",
            &lease,
            project.path().to_str().unwrap(),
            &www,
            "--exclude",
            "node_modules",
            "--exclude",
            ".env",
        ])
        .await;
    assert_eq!(put["status"], "succeeded", "{put}");
    let listing = "set -e\nls -A | LC_ALL=C sort | tr '\\n' ' '\necho\ncat 页面/index.html\nsha256sum blob.bin | cut -d' ' -f1\nstat -c %U blob.bin\n";
    let (code, job, err) = vm
        .cli(&["run", "--lease", &lease, "--cwd", &www], Some(listing))
        .await;
    assert_eq!(code, 0, "{err} {job}");
    let local = std::process::Command::new("/usr/bin/shasum")
        .args(["-a", "256"])
        .arg(project.path().join("blob.bin"))
        .output()
        .unwrap();
    let digest = String::from_utf8(local.stdout).unwrap();
    let digest = digest.split_whitespace().next().unwrap();
    assert_eq!(
        text(&job["stdout"]["text"]),
        format!("blob.bin existing.txt 页面 \n<h1>你好 🚀</h1>\n{digest}\ndeploy\n")
    );
    assert_eq!(
        text(&job["stderr"]["text"]),
        "",
        "GNU tar must not warn about macOS metadata"
    );

    // A long job is followed by cursor and ran exactly once.
    let command = format!("echo x >> {work}/ran; echo first; sleep 2; echo second");
    let job = vm
        .ok(&["run", "--lease", &lease, "--wait", "1", "--", &command])
        .await;
    let (done, seen) = vm.follow(job, |_| false).await;
    assert_eq!(
        (done["status"].as_str(), seen.as_str()),
        (Some("succeeded"), "first\nsecond\n")
    );
    assert_eq!(vm.sh(&lease, &format!("cat {work}/ran")).await, "x");

    // A real Linux PTY: a prompt, an answer, and the terminal size the job was given.
    let job = vm
        .ok(&[
            "run",
            "--lease",
            &lease,
            "--pty",
            "--wait",
            "1",
            "--",
            "stty size; printf 'name? '; read n; echo \"hi $n\"",
        ])
        .await;
    let (job, seen) = vm.follow(job, |seen| seen.contains("name? ")).await;
    assert!(seen.starts_with("40 120"), "{seen:?}");
    vm.ok(&[
        "job",
        "input",
        "--lease",
        &lease,
        &text(&job["job"]),
        "--",
        "bob",
    ])
    .await;
    let (done, seen) = vm.follow(job, |_| false).await;
    assert_eq!(done["status"], "succeeded");
    assert!(seen.contains("hi bob"), "{seen:?}");

    // Interrupt really stops the remote process, for a plain job and for a terminal job.
    for pty in [false, true] {
        let marker = format!("vibe-e2e-sleep-{}-{pty}", std::process::id());
        let command = format!("exec -a {marker} sleep 300");
        let mut args = vec!["run", "--lease", &lease, "--wait", "1"];
        if pty {
            args.push("--pty");
        }
        args.extend(["--", &command]);
        let job = vm.ok(&args).await;
        assert_eq!(
            vm.sh(&lease, &format!("pgrep -fc '^{marker}' || true"))
                .await,
            "1"
        );
        let asked = vm
            .ok(&["job", "interrupt", "--lease", &lease, &text(&job["job"])])
            .await;
        let (done, _) = vm.follow(asked, |_| false).await;
        assert_ne!(done["status"], "succeeded", "pty={pty}");
        assert_eq!(
            vm.sh(&lease, &format!("pgrep -fc '^{marker}' || true"))
                .await,
            "0",
            "pty={pty}"
        );
    }

    // Eight jobs run at once on the space's single connection; the ninth is refused locally.
    let mut jobs = Vec::new();
    for _ in 0..8 {
        jobs.push(text(
            &vm.ok(&["run", "--lease", &lease, "--wait", "0", "--", "sleep 4"])
                .await["job"],
        ));
    }
    let (code, _, err) = vm
        .cli(
            &["run", "--lease", &lease, "--wait", "0", "--", "true"],
            None,
        )
        .await;
    assert_eq!(
        (code, err["error"]["code"].as_str()),
        (2, Some("RESOURCE_LIMIT"))
    );
    for job in jobs {
        let (done, _) = vm
            .follow(vm.ok(&["job", "get", &job]).await, |_| false)
            .await;
        assert_eq!(done["status"], "succeeded");
    }

    // Takeover on a live Linux shell: the agent's refused command leaves no trace.
    vm.service.takeover(&space).await.unwrap();
    let (code, _, err) = vm
        .cli(
            &[
                "run",
                "--lease",
                &lease,
                "--",
                &format!("touch {work}/forbidden"),
            ],
            None,
        )
        .await;
    assert_eq!(
        (code, err["error"]["code"].as_str()),
        (2, Some("CONTROL_REVOKED"))
    );
    let mine = vm
        .service
        .run(By::Human(&space), Some(format!("ls {work}")), None, None)
        .await
        .unwrap();
    let seen = vm
        .service
        .job_settled(&mine, 10_000, 4_096, false)
        .await
        .unwrap();
    assert!(
        !seen.stdout.text.contains("forbidden"),
        "{}",
        seen.stdout.text
    );
    vm.service.release(&space).await.unwrap();
    let lease = text(&vm.ok(&["space", "get", &space]).await["control"]["lease"]);

    // Other credentials: an encrypted key and a password, both supplied by the service side.
    for host in ["vm-encrypted", "vm-password"] {
        let (_, other) = vm.open(host).await;
        let who = vm.sh(&other, "id -un").await;
        assert_eq!(
            who,
            if host == "vm-password" {
                "ops"
            } else {
                "deploy"
            }
        );
        vm.ok(&["space", "close", "--lease", &other]).await;
    }

    // sudo asks for a password: the agent hands off, the user types it, the agent continues.
    let (ops_space, ops) = vm.open("vm-ops").await;
    let job = vm
        .ok(&[
            "run",
            "--lease",
            &ops,
            "--pty",
            "--wait",
            "1",
            "--",
            "sudo -k; sudo id -un",
        ])
        .await;
    let (job, seen) = vm.follow(job, |seen| seen.contains("password")).await;
    assert!(seen.contains("[sudo]"), "{seen:?}");
    let id = text(&job["job"]);
    let handed = vm
        .ok(&[
            "space",
            "handoff",
            "--lease",
            &ops,
            "--reason",
            "sudo needs your password",
        ])
        .await;
    assert_eq!(handed["control"]["owner"], "human");
    let (code, _, err) = vm
        .cli(&["job", "input", "--lease", &ops, &id, "--", "guess"], None)
        .await;
    assert_eq!(
        (code, err["error"]["code"].as_str()),
        (2, Some("CONTROL_REVOKED"))
    );
    let password = format!("{}\n", script(&vm.dir, &["password"]).trim());
    vm.service
        .input(By::Human(&ops_space), &id, password.as_bytes(), false)
        .await
        .unwrap();
    vm.service.release(&ops_space).await.unwrap();
    let (done, seen) = vm
        .follow(vm.ok(&["job", "get", &id, "--tail", "4096"]).await, |_| {
            false
        })
        .await;
    assert_eq!(done["status"], "succeeded", "{seen:?}");
    assert!(seen.lines().any(|line| line.trim() == "root"), "{seen:?}");
    assert!(
        !seen.contains(password.trim()),
        "the password is not echoed into the job output"
    );

    vm.sh(&lease, &format!("rm -rf {work}")).await;
    vm.ok(&["space", "close", "--lease", &lease]).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
}
