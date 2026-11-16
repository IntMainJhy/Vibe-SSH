use std::{path::PathBuf, process::ExitCode};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde_json::json;
use vibe_ssh_core::{ConnectionConfig, SshConnection};
use vibe_ssh_protocol::{ExecutionRequest, JobStatus, PROTOCOL_VERSION};

#[derive(Parser)]
#[command(
    name = "vibe-ssh",
    version,
    about = "Vibe-SSH M0 kernel verification CLI"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Report the currently implemented development capabilities.
    Capabilities,
    /// Print the bundled Skill for this development version.
    Skill,
    /// Developer-only probe; not the final saved-host or daemon interface.
    Probe {
        /// Manually prepared JSON connection config with a verified fingerprint.
        #[arg(long)]
        config: PathBuf,
        /// JSON array of execution requests. Stops after the first non-success.
        #[arg(long)]
        requests: PathBuf,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(error) => {
            eprintln!(
                "{}",
                json!({"error": {"code": "TOOL_ERROR", "message": format!("{error:#}")}})
            );
            ExitCode::from(2)
        }
    }
}

async fn run() -> Result<bool> {
    match Cli::parse().command {
        Command::Capabilities => {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "version": env!("CARGO_PKG_VERSION"),
                    "protocol_version": PROTOCOL_VERSION,
                    "stage": "m0_development",
                    "implemented": ["capabilities", "skill", "probe.public_key", "probe.pinned_host", "probe.exec", "probe.pty", "probe.bounded_output"],
                    "not_available": ["desktop", "saved_hosts", "credential_vault", "daemon_ipc", "job_wait", "deployment_workflows"]
                }))?
            );
            Ok(true)
        }
        Command::Skill => {
            print!("{}", include_str!("../../../skills/vibe-ssh/SKILL.md"));
            Ok(true)
        }
        Command::Probe { config, requests } => {
            let config: ConnectionConfig = serde_json::from_slice(
                &tokio::fs::read(config)
                    .await
                    .context("could not read connection config")?,
            )
            .context("invalid connection config")?;
            let requests: Vec<ExecutionRequest> = serde_json::from_slice(
                &tokio::fs::read(requests)
                    .await
                    .context("could not read requests file")?,
            )
            .context("invalid requests JSON")?;
            anyhow::ensure!(
                !requests.is_empty() && requests.len() <= 32,
                "provide 1..=32 execution requests"
            );
            let connection = SshConnection::connect(&config).await?;
            let mut results = Vec::new();
            let mut success = true;
            for request in requests {
                let result = connection.run(request).await?;
                success = result.status == JobStatus::Succeeded;
                results.push(result);
                if !success {
                    break;
                }
            }
            let _ = connection.disconnect().await;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({"results": results, "success": success}))?
            );
            Ok(success)
        }
    }
}
