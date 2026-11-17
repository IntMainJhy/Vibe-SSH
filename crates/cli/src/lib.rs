use std::{io::Read, path::PathBuf, process::ExitCode};

use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use serde_json::{Value, json};
use vibe_ssh_core::{Archive, Error, Service, default_home, request, serve, skill};
use vibe_ssh_protocol::{Call, DEFAULT_READ};

#[derive(Parser)]
#[command(
    name = "vibe-ssh",
    version,
    about = "Operate the user's servers through the Vibe-SSH app. Agents: run `vibe-ssh skill` first.",
    after_help = "Exit codes: 0 ok (a job succeeded or is still running), 1 a job failed or its result is unknown, 2 tool or service error (JSON on stderr). Remote exit codes are in the JSON, never the process status."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the usage guide that matches this version, or a deployment flow.
    Skill {
        /// `static-site` or `compose`. Omit for the main guide.
        topic: Option<String>,
    },
    /// Service version, protocol and instance.
    Status,
    /// Hosts the user enabled for agents.
    Hosts,
    /// Task spaces: one per goal, each with its own SSH connection.
    #[command(subcommand)]
    Space(SpaceCommand),
    /// Run a command in a space. With no command, the script is read from stdin.
    Run {
        #[arg(long)]
        lease: String,
        /// Absolute remote working directory.
        #[arg(long)]
        cwd: Option<String>,
        /// Allocate a terminal (merged output, accepts input).
        #[arg(long)]
        pty: bool,
        #[command(flatten)]
        wait: Wait,
        #[arg(last = true)]
        command: Vec<String>,
    },
    /// Upload a local file or directory into a remote directory. Never deletes remote files.
    /// Blocks until the whole upload has been sent.
    Put {
        #[arg(long)]
        lease: String,
        local: PathBuf,
        /// Absolute remote directory; created if missing.
        remote_dir: String,
        /// tar exclude pattern, repeatable (e.g. node_modules, .git, .env).
        #[arg(long)]
        exclude: Vec<String>,
        #[command(flatten)]
        wait: Wait,
    },
    #[command(subcommand)]
    Job(JobCommand),
    /// Link the Skill into ~/.agents/skills and ~/.claude/skills (Codex, Claude Code and others).
    InstallSkill,
    /// Run a headless service. Developer entry: the app normally provides the service.
    Serve,
}

#[derive(Args)]
struct Wait {
    /// Seconds to wait for the job to finish before returning it still running (max 120).
    #[arg(long, default_value_t = 30)]
    wait: u64,
    /// Bytes of trailing output to return per stream.
    #[arg(long, default_value_t = DEFAULT_READ)]
    tail: usize,
}

#[derive(Subcommand)]
enum SpaceCommand {
    /// Open a space on a host for one goal. Prints the lease that authorizes writes.
    Open {
        /// Host id or unique name from `vibe-ssh hosts`.
        host: String,
        /// Short description of the goal, shown to the user.
        #[arg(long)]
        name: String,
        /// Your agent's name, shown to the user (or set VIBE_SSH_ACTOR).
        #[arg(long, env = "VIBE_SSH_ACTOR", default_value = "agent")]
        actor: String,
    },
    List,
    /// A space's control state, current lease and jobs.
    Get {
        space: String,
    },
    /// Hand the space to the user, saying what they need to do.
    Handoff {
        #[arg(long)]
        lease: String,
        #[arg(long)]
        reason: String,
    },
    /// Close the space and its connection. Remote processes are not killed.
    Close {
        #[arg(long)]
        lease: String,
    },
}

#[derive(Subcommand)]
enum JobCommand {
    /// A job with its trailing output.
    Get {
        job: String,
        #[arg(long, default_value_t = DEFAULT_READ)]
        tail: usize,
    },
    /// Wait for output after a cursor, completion, or a control change.
    Wait {
        /// The `cursor` of the previous result.
        cursor: String,
        /// Seconds (max 120). Expiry is not a failure.
        #[arg(long, default_value_t = 30)]
        timeout: u64,
        #[arg(long, default_value_t = DEFAULT_READ)]
        max: usize,
    },
    /// Send a line to a running job. With no text, stdin is sent as-is.
    Input {
        #[arg(long)]
        lease: String,
        job: String,
        /// Send the text exactly, without a trailing newline.
        #[arg(long)]
        raw: bool,
        /// Close the job's stdin afterwards.
        #[arg(long)]
        end: bool,
        #[arg(last = true)]
        text: Vec<String>,
    },
    /// Request an interrupt (Ctrl+C or SIGINT). The job is not finished until it says so.
    Interrupt {
        #[arg(long)]
        lease: String,
        job: String,
    },
}

/// The whole `vibe-ssh` command line. The desktop app runs this when invoked as `vibe-ssh`.
#[tokio::main]
pub async fn main() -> ExitCode {
    match run(Cli::parse().command).await {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            let code = error
                .downcast_ref::<Error>()
                .map_or(json!("TOOL_ERROR"), |e| json!(e.code));
            eprintln!(
                "{}",
                json!({"error": {"code": code, "message": format!("{error:#}")}})
            );
            ExitCode::from(2)
        }
    }
}

fn stdin() -> anyhow::Result<String> {
    let mut text = String::new();
    std::io::stdin()
        .read_to_string(&mut text)
        .context("stdin must be UTF-8")?;
    Ok(text)
}

async fn run(command: Command) -> anyhow::Result<u8> {
    let home = default_home();
    let mut archive = None;
    let call = match command {
        Command::Skill { topic } => {
            print!("{}", skill::guide(topic.as_deref())?);
            return Ok(0);
        }
        Command::InstallSkill => {
            let user = PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?);
            let report = skill::install(&home, &user, None)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            return Ok(if report["skipped"] == json!([]) { 0 } else { 2 });
        }
        Command::Serve => {
            // Headless: unencrypted keys only. The app supplies Keychain secrets and the UI.
            let service = Service::new(home, Box::new(|_| None));
            eprintln!("{}", service.status());
            serve(service).await?.await?;
            return Ok(0);
        }
        Command::Status => Call::Status,
        Command::Hosts => Call::Hosts,
        Command::Space(SpaceCommand::Open { host, name, actor }) => {
            Call::SpaceOpen { host, name, actor }
        }
        Command::Space(SpaceCommand::List) => Call::SpaceList,
        Command::Space(SpaceCommand::Get { space }) => Call::SpaceGet { space },
        Command::Space(SpaceCommand::Handoff { lease, reason }) => {
            Call::SpaceHandoff { lease, reason }
        }
        Command::Space(SpaceCommand::Close { lease }) => Call::SpaceClose { lease },
        Command::Run {
            lease,
            cwd,
            pty,
            wait,
            command,
        } => Call::Run {
            lease,
            command: Some(if command.is_empty() {
                stdin()?
            } else {
                command.join(" ")
            }),
            cwd,
            pty,
            wait_ms: wait.wait.saturating_mul(1000),
            tail_bytes: wait.tail,
        },
        Command::Put {
            lease,
            local,
            remote_dir,
            exclude,
            wait,
        } => {
            // tar runs here, in the agent's own process, with the agent's own file permissions.
            archive = Some(Archive::pack(&local, &exclude)?);
            Call::Put {
                lease,
                remote_dir,
                wait_ms: wait.wait.saturating_mul(1000),
                tail_bytes: wait.tail,
            }
        }
        Command::Job(JobCommand::Get { job, tail }) => Call::JobGet {
            job,
            tail_bytes: tail,
        },
        Command::Job(JobCommand::Wait {
            cursor,
            timeout,
            max,
        }) => Call::JobWait {
            cursor,
            max_bytes: max,
            timeout_ms: timeout.saturating_mul(1000),
        },
        Command::Job(JobCommand::Input {
            lease,
            job,
            raw,
            end,
            text,
        }) => Call::JobInput {
            lease,
            job,
            data: match (text.is_empty(), raw) {
                (true, _) if end => String::new(),
                (true, _) => stdin()?,
                (false, true) => text.join(" "),
                (false, false) => text.join(" ") + "\n",
            },
            end,
        },
        Command::Job(JobCommand::Interrupt { lease, job }) => Call::JobInterrupt { lease, job },
    };
    let reply = request(&home, call, archive).await?;
    println!("{}", serde_json::to_string_pretty(&reply)?);
    Ok(match &reply["status"] {
        Value::String(status) if status == "failed" || status == "unknown" => 1,
        _ => 0,
    })
}
