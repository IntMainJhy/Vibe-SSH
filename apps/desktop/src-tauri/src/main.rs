//! Vibe-SSH desktop app. The kernel runs inside this process and serves the agent socket;
//! every command below is a human operation that the socket cannot express.

use std::{path::PathBuf, process::ExitCode, sync::Arc};

mod credentials;
use credentials::Credentials;
use serde_json::{Value, json};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{Emitter, State};
mod workbench;
use vibe_ssh_core::{By, Chunk, Error, ErrorCode, Host, Service, default_home, serve, skill};
use vibe_ssh_protocol::{ErrorBody, Owner, SpaceView};

#[cfg(test)]
#[path = "../../../../crates/cli/tests/support/sshd.rs"]
mod sshd;

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

// Allows isolated integration validation without touching the real Agent directories.
fn integration_home() -> PathBuf {
    std::env::var_os("VIBE_SSH_INTEGRATION_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(user_home)
}

#[tauri::command]
async fn snapshot(
    kernel: Kernel<'_>,
    integration: State<'_, workbench::Integration>,
) -> Reply<Value> {
    Ok(json!({
        "status": kernel.status(),
        "open_documents": kernel.open_documents(),
        "hosts": kernel.load_hosts().map_err(fail)?,
        "spaces": kernel.spaces(false).await,
        "skill": skill::installed(kernel.home(), &integration_home()),
        "integration": integration.0.lock().unwrap().clone(),
    }))
}

/// Shows the server's key so the user can compare it with a trusted source before saving.
#[tauri::command]
async fn host_key(host: String, port: u16) -> Reply<(String, String)> {
    vibe_ssh_core::host_key(&host, port).await.map_err(fail)
}

#[tauri::command]
async fn history_list(
    kernel: Kernel<'_>,
    offset: usize,
    limit: usize,
) -> Reply<vibe_ssh_protocol::HistoryPage> {
    kernel
        .history_list(offset, limit, false)
        .await
        .map_err(fail)
}
#[tauri::command]
async fn history_get(
    kernel: Kernel<'_>,
    record: String,
    tail: usize,
) -> Reply<vibe_ssh_protocol::HistoryDetail> {
    kernel.history_get(&record, tail, false).await.map_err(fail)
}

#[tauri::command]
async fn save_host(
    kernel: Kernel<'_>,
    credentials: State<'_, Credentials>,
    mut host: Host,
    secret: Option<String>,
) -> Reply<Host> {
    host.name = host.name.trim().to_owned();
    host.host = host.host.trim().to_owned();
    host.username = host.username.trim().to_owned();
    if host.name.trim().is_empty() || host.host.trim().is_empty() || host.username.is_empty() {
        return Err(invalid("name, address and username are required"));
    }
    if !host.host_key_sha256.starts_with("SHA256:") {
        return Err(invalid("fetch and verify the host fingerprint first"));
    }
    if host.port == 0 || host.host.contains(char::is_whitespace) || host.host.contains('\0') {
        return Err(invalid(
            "address must not contain spaces; port must be 1..=65535",
        ));
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
        credentials
            .set(&host.id, secret.as_bytes())
            .map_err(|e| ErrorBody {
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
async fn delete_host(
    kernel: Kernel<'_>,
    credentials: State<'_, Credentials>,
    id: String,
) -> Reply<()> {
    let mut hosts = kernel.load_hosts().map_err(fail)?;
    hosts.retain(|host| host.id != id);
    kernel.save_hosts(&hosts).map_err(fail)?;
    credentials.delete(&id);
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

/// Only called after the user confirms exit. Never exposed on the agent socket.
#[tauri::command]
fn exit_app<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    app.exit(0);
}

/// A user-triggered clipboard write, independent of WebKit's Clipboard API support.
#[tauri::command]
fn copy_text(text: String) -> Reply<()> {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    if text.len() > vibe_ssh_protocol::MAX_COMMAND {
        return Err(invalid("clipboard text is too large"));
    }
    let failure = |_| ErrorBody {
        code: ErrorCode::Internal,
        message: "无法写入系统剪贴板，请重试。".into(),
    };
    let mut child = Command::new("/usr/bin/pbcopy")
        .env("LC_CTYPE", "UTF-8")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(failure)?;
    let written = child
        .stdin
        .take()
        .expect("piped")
        .write_all(text.as_bytes());
    let finished = child.wait();
    written.map_err(failure)?;
    if !finished.map_err(failure)?.success() {
        return Err(invalid("无法写入系统剪贴板，请重试。"));
    }
    Ok(())
}

/// Script errors, blocked resources and layout facts from the window. Shown when traced.
#[tauri::command]
fn ui_report(message: String) {
    if std::env::var_os("VIBE_SSH_TRACE").is_some() {
        eprintln!("ui {message}");
    }
}

/// Registers the kernel and every human command. Shared with the tests' mock runtime.
fn commands<R: tauri::Runtime>(
    builder: tauri::Builder<R>,
    service: Arc<Service>,
    credentials: Credentials,
) -> tauri::Builder<R> {
    let handler: Box<dyn Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync> =
        Box::new(tauri::generate_handler![
            snapshot,
            host_key,
            history_list,
            history_get,
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
            workbench::files,
            workbench::choose_diff,
            workbench::forget_diff,
            workbench::rescan_diff,
            workbench::compare_directory,
            workbench::text_file,
            workbench::save_text,
            workbench::forget_text,
            workbench::resources,
            workbench::file_action,
            workbench::upload_files,
            workbench::download_file,
            workbench::cancel_transfer,
            exit_app,
            copy_text,
            ui_report
        ]);
    // VIBE_SSH_TRACE=1 logs which commands the interface calls, for diagnosing a blank window.
    let trace = std::env::var_os("VIBE_SSH_TRACE").is_some();
    builder
        .manage(service)
        .manage(credentials)
        .manage(workbench::Transfers::default())
        .manage(workbench::DiffSources::default())
        .manage(workbench::Integration::default())
        .invoke_handler(move |invoke| {
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
    let credentials = Credentials::open();
    let source = credentials.clone();
    let service = Service::new(default_home(), Box::new(move |host| source.get(&host.id)));
    let quitting = service.clone();
    let menu_service = service.clone();
    commands(tauri::Builder::default(), service.clone(), credentials)
        .menu(|app| {
            // macOS's predefined Quit terminates directly; use a normal menu action
            // so Cmd+Q goes through our task-aware confirmation as well.
            let menu = Menu::default(app)?;
            menu.remove_at(0)?;
            let quit = MenuItem::with_id(
                app,
                "quit-confirm",
                "退出 Vibe-SSH",
                true,
                Some("CmdOrCtrl+Q"),
            )?;
            let app_menu = Submenu::with_items(
                app,
                "Vibe-SSH",
                true,
                &[
                    &PredefinedMenuItem::about(app, Some("关于 Vibe-SSH"), None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::services(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::hide(app, Some("隐藏 Vibe-SSH"))?,
                    &PredefinedMenuItem::hide_others(app, None)?,
                    &PredefinedMenuItem::show_all(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &quit,
                ],
            )?;
            menu.prepend(&app_menu)?;
            Ok(menu)
        })
        .on_menu_event(move |app, event| {
            if event.id().as_ref() == "quit-confirm" {
                if menu_service.running_jobs() > 0 || menu_service.open_documents() > 0 || {
                    use tauri::Manager;
                    app.state::<workbench::Transfers>().active() > 0
                } {
                    let _ = app.emit("quit-requested", ());
                } else {
                    app.exit(0);
                }
            }
        })
        .setup(move |app| {
            tauri::async_runtime::block_on(serve(service.clone()))?;
            workbench::configure(app, &service);
            let (app, mut events) = (app.handle().clone(), service.subscribe());
            tauri::async_runtime::spawn(async move {
                while events.changed().await.is_ok() {
                    let _ = app.emit("changed", ());
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Vibe-SSH failed to start")
        .run(move |app, event| {
            // Cmd+Q and the application menu also pass through the same confirmation.
            // An explicit exit(0) from the confirmation has a code and proceeds.
            if let tauri::RunEvent::ExitRequested {
                code: None, api, ..
            } = event
                && (quitting.running_jobs() > 0 || quitting.open_documents() > 0 || {
                    use tauri::Manager;
                    app.state::<workbench::Transfers>().active() > 0
                })
            {
                api.prevent_exit();
                let _ = app.emit("quit-requested", ());
            }
        });
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use tauri::{
        Manager, WebviewWindow, WebviewWindowBuilder,
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
        let app = commands(
            mock_builder(),
            service.clone(),
            super::Credentials::default(),
        )
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
        form["port"] = json!(0);
        let refused =
            invoke(&webview, "save_host", json!({"host": form, "secret": null})).unwrap_err();
        assert_eq!(
            refused["code"], "INVALID_REQUEST",
            "an invalid port is never saved"
        );
        form["port"] = json!(sshd.port);
        let saved = invoke(&webview, "save_host", json!({"host": form, "secret": null})).unwrap();
        let host = saved["id"].as_str().unwrap().to_owned();
        assert_eq!(host.len(), 8, "a new host gets a generated id");

        assert_eq!(service.running_jobs(), 0);
        // An agent starts working on that host.
        let agent = block(service.open(&host, "deploy", "codex", Owner::Agent)).unwrap();
        let lease = agent.control.lease.clone().unwrap();
        let space = agent.space.as_str();
        let job = block(service.run(By::Lease(&lease), Some("cat".into()), None, Some((80, 24))))
            .unwrap();
        let job = job.as_str();
        assert_eq!(
            service.running_jobs(),
            1,
            "the native quit guard sees running jobs"
        );
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
        // The same frontend bridge really browses and mutates the temporary SFTP directory.
        let directory = sshd.path("browser-files");
        std::fs::create_dir(&directory).unwrap();
        let folder = directory.join("中文 folder");
        invoke(
            &webview,
            "file_action",
            json!({"space":mine,"action":"mkdir","path":folder,"destination":null}),
        )
        .unwrap();
        let listed = invoke(&webview, "files", json!({"space":mine,"path":directory})).unwrap();
        assert_eq!(listed["entries"][0]["name"], "中文 folder");
        assert_eq!(listed["entries"][0]["kind"], "directory");
        invoke(
            &webview,
            "file_action",
            json!({"space":mine,"action":"delete","path":folder,"destination":null}),
        )
        .unwrap();
        let refused = invoke(
            &webview,
            "file_action",
            json!({"space":space,"action":"mkdir","path":folder,"destination":null}),
        )
        .unwrap_err();
        assert_eq!(refused["code"], "CONTROL_REVOKED");
        assert!(invoke(&webview, "resources", json!({"space":mine})).is_err());
        let text_path = sshd.path("editor.txt");
        std::fs::write(&text_path, "original").unwrap();
        let text = invoke(
            &webview,
            "text_file",
            json!({"space":mine,"path":text_path}),
        )
        .unwrap();
        assert_eq!(text["text"], "original");
        assert_eq!(service.open_documents(), 1);
        let updated = invoke(
            &webview,
            "save_text",
            json!({"space":mine,"revision":text["revision"],"text":"界面保存\r\n"}),
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&text_path).unwrap(), "界面保存\r\n");
        assert_eq!(
            std::fs::read_to_string(updated["backup"].as_str().unwrap()).unwrap(),
            "original"
        );
        assert_eq!(service.open_documents(), 1);
        std::fs::write(&text_path, "external change").unwrap();
        let changed = invoke(
            &webview,
            "save_text",
            json!({"space":mine,"revision":updated["document"]["revision"],"text":"overwrite"}),
        )
        .unwrap_err();
        assert_eq!(changed["code"], "FILE_CHANGED");
        invoke(
            &webview,
            "forget_text",
            json!({"space":mine,"revision":updated["document"]["revision"]}),
        )
        .unwrap();
        assert_eq!(service.open_documents(), 0);
        let project = sshd.path("compare-project");
        std::fs::create_dir(&project).unwrap();
        std::fs::write(project.join("选择.txt"), "same").unwrap();
        std::fs::write(directory.join("选择.txt"), "same").unwrap();
        let manifest = vibe_ssh_core::diff::scan(&project, &[]).unwrap();
        app.state::<super::workbench::DiffSources>()
            .fixture("chosen", project, manifest);
        let preview = invoke(
            &webview,
            "compare_directory",
            json!({"id":"chosen","selected":["选择.txt"],"space":mine,"remote":directory}),
        )
        .unwrap();
        assert_eq!(preview["entries"][0]["status"], "same");
        assert!(
            invoke(
                &webview,
                "compare_directory",
                json!({"id":"not-chosen","selected":[],"space":mine,"remote":directory})
            )
            .is_err()
        );
        invoke(&webview, "forget_diff", json!({"id":"chosen"})).unwrap();
        let history = invoke(&webview, "history_list", json!({"offset":0,"limit":50})).unwrap();
        let personal = history["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["origin"] == "human")
            .unwrap();
        let retained = invoke(
            &webview,
            "history_get",
            json!({"record":personal["record"],"tail":8192}),
        )
        .unwrap();
        assert!(retained["stdout"]["total_bytes"].is_number());
        assert!(
            block(service.history_get(personal["record"].as_str().unwrap(), 8192, true)).is_err(),
            "human history is visible only on the human bridge"
        );
        invoke(&webview, "close_space", json!({"space": mine})).unwrap();
        invoke(&webview, "close_space", json!({"space": space})).unwrap();
        assert_eq!(
            invoke(&webview, "snapshot", json!({})).unwrap()["spaces"],
            json!([])
        );
        assert!(
            !invoke(&webview, "history_list", json!({"offset":0,"limit":50})).unwrap()["records"]
                .as_array()
                .unwrap()
                .is_empty(),
            "closing spaces preserves task records"
        );
    }
}
