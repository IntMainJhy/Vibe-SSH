//! Vibe-SSH desktop app. The kernel runs inside this process and serves the agent socket;
//! every command below is a human operation that the socket cannot express.

use std::{path::PathBuf, process::ExitCode, sync::Arc};

use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};
use serde_json::{Value, json};
use tauri::{Emitter, State};
use vibe_ssh_core::{By, Chunk, Error, ErrorCode, Host, Service, default_home, serve, skill};
use vibe_ssh_protocol::{ErrorBody, Owner, SpaceView};

#[cfg(test)]
#[path = "../../../../crates/cli/tests/support/sshd.rs"]
mod sshd;

/// Keychain service name. The account is the host id; the value is its password or passphrase.
const KEYCHAIN: &str = "io.github.intmainjhy.vibe-ssh";

type Kernel<'a> = State<'a, Arc<Service>>;
type Reply<T> = Result<T, ErrorBody>;

fn fail(Error { code, message }: Error) -> ErrorBody {
    ErrorBody { code, message }
}

fn invalid(message: &str) -> ErrorBody {
    ErrorBody {
        code: ErrorCode::InvalidRequest,
        message: message.into(),
    }
}

fn user_home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

#[tauri::command]
async fn snapshot(kernel: Kernel<'_>) -> Reply<Value> {
    Ok(json!({
        "status": kernel.status(),
        "hosts": kernel.load_hosts().map_err(fail)?,
        "spaces": kernel.spaces(false).await,
        "skill": skill::installed(kernel.home(), &user_home()),
    }))
}

/// Shows the server's key so the user can compare it with a trusted source before saving.
#[tauri::command]
async fn host_key(host: String, port: u16) -> Reply<(String, String)> {
    vibe_ssh_core::host_key(&host, port).await.map_err(fail)
}

#[tauri::command]
async fn save_host(kernel: Kernel<'_>, mut host: Host, secret: Option<String>) -> Reply<Host> {
    if host.name.trim().is_empty() || host.host.trim().is_empty() || host.username.is_empty() {
        return Err(invalid("name, address and username are required"));
    }
    if !host.host_key_sha256.starts_with("SHA256:") {
        return Err(invalid("fetch and verify the host fingerprint first"));
    }
    if let Some(path) = &host.identity_file {
        let path = match path.strip_prefix("~") {
            Ok(rest) => user_home().join(rest),
            Err(_) => path.clone(),
        };
        if !path.is_file() {
            return Err(invalid("the private key file does not exist"));
        }
        host.identity_file = Some(path);
    }
    let mut hosts = kernel.load_hosts().map_err(fail)?;
    if host.id.is_empty() {
        host.id = uuid::Uuid::new_v4().simple().to_string()[..8].to_owned();
    }
    if let Some(secret) = secret.filter(|secret| !secret.is_empty()) {
        set_generic_password(KEYCHAIN, &host.id, secret.as_bytes()).map_err(|e| ErrorBody {
            code: ErrorCode::Internal,
            message: format!("Keychain: {e}"),
        })?;
    }
    match hosts.iter_mut().find(|known| known.id == host.id) {
        Some(known) => *known = host.clone(),
        None => hosts.push(host.clone()),
    }
    kernel.save_hosts(&hosts).map_err(fail)?;
    Ok(host)
}

#[tauri::command]
async fn delete_host(kernel: Kernel<'_>, id: String) -> Reply<()> {
    let mut hosts = kernel.load_hosts().map_err(fail)?;
    hosts.retain(|host| host.id != id);
    kernel.save_hosts(&hosts).map_err(fail)?;
    let _ = delete_generic_password(KEYCHAIN, &id);
    Ok(())
}

/// Opens the user's own space on a host with a login shell. Agents cannot see it.
#[tauri::command]
async fn open_terminal(kernel: Kernel<'_>, host: String) -> Reply<Value> {
    let space = kernel
        .open(&host, "终端", "me", Owner::Human)
        .await
        .map_err(fail)?
        .space;
    match kernel
        .run(By::Human(&space), None, None, Some((120, 40)))
        .await
    {
        Ok(job) => Ok(json!({ "space": space, "job": job })),
        Err(error) => {
            let _ = kernel.close(By::Human(&space)).await;
            Err(fail(error))
        }
    }
}

/// Another shell in a space the user holds, including an agent space after a takeover.
#[tauri::command]
async fn shell(kernel: Kernel<'_>, space: String) -> Reply<String> {
    kernel
        .run(By::Human(&space), None, None, Some((120, 40)))
        .await
        .map_err(fail)
}

#[tauri::command]
async fn takeover(kernel: Kernel<'_>, space: String) -> Reply<SpaceView> {
    kernel.takeover(&space).await.map_err(fail)
}

#[tauri::command]
async fn release(kernel: Kernel<'_>, space: String) -> Reply<SpaceView> {
    kernel.release(&space).await.map_err(fail)
}

#[tauri::command]
async fn close_space(kernel: Kernel<'_>, space: String) -> Reply<()> {
    kernel.close(By::Human(&space)).await.map_err(fail)
}

#[tauri::command]
async fn job_stream(
    kernel: Kernel<'_>,
    job: String,
    out: u64,
    err: u64,
    wait_ms: u64,
) -> Reply<Chunk> {
    kernel
        .job_stream(&job, (out, err), wait_ms)
        .await
        .map_err(fail)
}

#[tauri::command]
async fn job_input(kernel: Kernel<'_>, space: String, job: String, data: String) -> Reply<()> {
    kernel
        .input(By::Human(&space), &job, data.as_bytes(), false)
        .await
        .map_err(fail)
}

#[tauri::command]
async fn job_resize(
    kernel: Kernel<'_>,
    space: String,
    job: String,
    columns: u32,
    rows: u32,
) -> Reply<()> {
    kernel
        .resize(By::Human(&space), &job, columns, rows)
        .await
        .map_err(fail)
}

#[tauri::command]
async fn job_interrupt(kernel: Kernel<'_>, space: String, job: String) -> Reply<()> {
    kernel
        .interrupt(By::Human(&space), &job)
        .await
        .map_err(fail)
}

/// Explicit user action: links the Skill for agents and this binary as the `vibe-ssh` command.
#[tauri::command]
async fn install_skill(kernel: Kernel<'_>) -> Reply<Value> {
    let cli = std::env::current_exe().ok();
    skill::install(kernel.home(), &user_home(), cli.as_deref()).map_err(fail)
}

/// Registers the kernel and every human command. Shared with the tests' mock runtime.
fn commands<R: tauri::Runtime>(
    builder: tauri::Builder<R>,
    service: Arc<Service>,
) -> tauri::Builder<R> {
    let handler: Box<dyn Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync> =
        Box::new(tauri::generate_handler![
            snapshot,
            host_key,
            save_host,
            delete_host,
            open_terminal,
            shell,
            takeover,
            release,
            close_space,
            job_stream,
            job_input,
            job_resize,
            job_interrupt,
            install_skill
        ]);
    // VIBE_SSH_TRACE=1 logs which commands the interface calls, for diagnosing a blank window.
    let trace = std::env::var_os("VIBE_SSH_TRACE").is_some();
    builder.manage(service).invoke_handler(move |invoke| {
        if trace {
            eprintln!("invoke {}", invoke.message.command());
        }
        handler(invoke)
    })
}

fn main() -> ExitCode {
    // Multi-call binary: linked as `vibe-ssh`, this executable is the agent CLI.
    let invoked = std::env::args_os().next().map(PathBuf::from);
    if invoked.as_deref().and_then(|path| path.file_name()) == Some("vibe-ssh".as_ref()) {
        return vibe_ssh_cli::main();
    }
    // ponytail: a Keychain prompt blocks one runtime worker; move to spawn_blocking if it stalls.
    let service = Service::new(
        default_home(),
        Box::new(|host| {
            let secret = get_generic_password(KEYCHAIN, &host.id).ok()?;
            String::from_utf8(secret).ok()
        }),
    );
    commands(tauri::Builder::default(), service.clone())
        .setup(move |app| {
            tauri::async_runtime::block_on(serve(service.clone()))?;
            let (app, mut events) = (app.handle().clone(), service.subscribe());
            tauri::async_runtime::spawn(async move {
                while events.changed().await.is_ok() {
                    let _ = app.emit("changed", ());
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Vibe-SSH failed to start");
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use tauri::{
        WebviewWindow, WebviewWindowBuilder,
        ipc::{CallbackFn, InvokeBody},
        test::{
            INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder, mock_context, noop_assets,
        },
        webview::InvokeRequest,
    };
    use vibe_ssh_core::{By, ErrorCode, Service};
    use vibe_ssh_protocol::Owner;

    use super::{commands, sshd::Sshd};

    /// Calls a command the way the frontend does: camelCase argument names, JSON in and out.
    fn invoke(
        webview: &WebviewWindow<MockRuntime>,
        command: &str,
        args: Value,
    ) -> Result<Value, Value> {
        let request = InvokeRequest {
            cmd: command.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        };
        get_ipc_response(webview, request).map(|body| body.deserialize().unwrap())
    }

    #[test]
    fn interface_commands_add_a_host_take_over_type_and_return_an_agent_space() {
        let sshd = Sshd::start();
        let home = sshd.path("home");
        std::fs::create_dir(&home).unwrap();
        // No secret source: this test never touches the Keychain.
        let service = Service::new(home, Box::new(|_| None));
        let app = commands(mock_builder(), service.clone())
            .build(mock_context(noop_assets()))
            .unwrap();
        let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        use tauri::async_runtime::block_on as block;

        // The form shows the probed key, and saving is refused until a fingerprint was fetched.
        let key = invoke(
            &webview,
            "host_key",
            json!({"host": "127.0.0.1", "port": sshd.port}),
        )
        .unwrap();
        assert_eq!(key, json!(["ssh-ed25519", sshd.fingerprint]));
        let mut form = serde_json::to_value(sshd.host("", "id_plain", true)).unwrap();
        form["name"] = json!("box");
        form["host_key_sha256"] = json!("");
        let refused =
            invoke(&webview, "save_host", json!({"host": form, "secret": null})).unwrap_err();
        assert_eq!(refused["code"], "INVALID_REQUEST");
        form["host_key_sha256"] = key[1].clone();
        let saved = invoke(&webview, "save_host", json!({"host": form, "secret": null})).unwrap();
        let host = saved["id"].as_str().unwrap().to_owned();
        assert_eq!(host.len(), 8, "a new host gets a generated id");

        // An agent starts working on that host.
        let agent = block(service.open(&host, "deploy", "codex", Owner::Agent)).unwrap();
        let lease = agent.control.lease.clone().unwrap();
        let space = agent.space.as_str();
        let job = block(service.run(By::Lease(&lease), Some("cat".into()), None, Some((80, 24))))
            .unwrap();
        let job = job.as_str();
        let seen = invoke(&webview, "snapshot", json!({})).unwrap();
        assert_eq!(seen["spaces"][0]["actor"], "codex");
        assert_eq!(seen["spaces"][0]["jobs"][0]["status"], "running");
        assert_eq!(seen["hosts"][0]["agent_access"], true);

        // Typing is refused until the user takes over; after that the agent is refused.
        let typed = json!({"space": space, "job": job, "data": "from the user\n"});
        let early = invoke(&webview, "job_input", typed.clone()).unwrap_err();
        assert_eq!(early["code"], "CONTROL_REVOKED");
        let taken = invoke(&webview, "takeover", json!({"space": space})).unwrap();
        assert_eq!(taken["control"]["owner"], "human");
        let late = block(service.input(By::Lease(&lease), job, b"from the agent\n", false));
        assert_eq!(late.unwrap_err().code, ErrorCode::ControlRevoked);
        invoke(&webview, "job_input", typed.clone()).unwrap();
        let size = json!({"space": space, "job": job, "columns": 100, "rows": 30});
        invoke(&webview, "job_resize", size).unwrap();

        // The terminal view reads raw bytes with the argument names the frontend sends.
        let (mut text, mut next) = (String::new(), (0, 0));
        for _ in 0..50 {
            let args = json!({"job": job, "out": next.0, "err": next.1, "waitMs": 200});
            let chunk = invoke(&webview, "job_stream", args).unwrap();
            let bytes: Vec<u8> = serde_json::from_value(chunk["out"].clone()).unwrap();
            text.push_str(&String::from_utf8_lossy(&bytes));
            next = (
                chunk["next"][0].as_u64().unwrap(),
                chunk["next"][1].as_u64().unwrap(),
            );
            // Once echoed by the terminal, once written back by `cat`.
            if text.matches("from the user").count() >= 2 {
                break;
            }
        }
        assert_eq!(text.matches("from the user").count(), 2, "{text:?}");
        assert!(!text.contains("from the agent"), "{text:?}");

        let returned = invoke(&webview, "release", json!({"space": space})).unwrap();
        assert_eq!(returned["control"]["owner"], "agent");
        let after = invoke(&webview, "job_input", typed).unwrap_err();
        assert_eq!(after["code"], "CONTROL_REVOKED");

        // The user's own terminal is a separate space that the agent side cannot see.
        let mine = invoke(&webview, "open_terminal", json!({"host": host})).unwrap();
        let mine = mine["space"].as_str().unwrap();
        assert!(block(service.space(mine, true)).is_err());
        assert!(
            invoke(&webview, "shell", json!({"space": mine}))
                .unwrap()
                .is_string()
        );
        invoke(&webview, "close_space", json!({"space": mine})).unwrap();
        invoke(&webview, "close_space", json!({"space": space})).unwrap();
        assert_eq!(
            invoke(&webview, "snapshot", json!({})).unwrap()["spaces"],
            json!([])
        );
    }
}
