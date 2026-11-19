# Vibe-SSH roadmap

Vibe-SSH is a macOS SSH client that external agents operate through Skills and a CLI. The focus is a useful daily terminal, visible server operations and reliable human takeover.

This is a directional roadmap, not a release-date commitment. Source code and [implementation evidence](docs/06-development-and-status.md) determine whether a capability is available.

## Available in the macOS Beta

- Manual hosts, fingerprint verification and per-host agent access.
- Personal terminals and agent task spaces with separate SSH connections.
- Lease-checked execution, incremental output, upload, handoff and takeover/return.
- A macOS workspace with tabbed terminals, search and appearance controls.
- SFTP operations and bounded UTF-8 editing with version checks and backups.
- Linux resource monitoring while its page is open.
- Directory comparison, ignore rules and reviewed application with version checks and backups.
- Bounded durable command/output history, privacy filtering and honest restart outcomes.
- Native core-flow acceptance and encrypted-key passphrase restart in an isolated real Keychain.
- Version-matched Skill/CLI integration and Nginx and Compose reference flows.
- Apple silicon and Intel macOS Beta packages. Qualification on physical Intel hardware is pending.

## Next priorities

- [x] Revoke live agent writes when host access is disabled or the host is removed.
- [ ] Complete clean-system installation, update and uninstall acceptance on Apple silicon and physical Intel Macs (isolated Apple silicon profile and Rosetta checks passed).
- [x] Complete M3 native-window replay for directory comparison and retained output/restart outcomes.
- [ ] Verify independent agent-led deployment and recovery (Codex static site passed; Claude Code model gateway unavailable).
- [x] Add bounded agent file reads and Linux resource snapshots.
- [x] Preview OpenSSH imports, authenticate with a local SSH agent, connect through verified jump hosts, and manage fixed or dynamic local forwarding.
- [x] Add desktop remote TCP forwarding for loopback services.
- [x] Batch Agent deployment steps and repeat the paired Codex static-site benchmark.
- [x] Connect through unauthenticated HTTP CONNECT or SOCKS5 proxies.
- [x] Add folder transfers, regular-file permission editing and per-mount usage.
- [x] Pause and resume single-file downloads across restarts.
- [x] Add terminal-buffer search and text export.
- [x] Pause and resume single-file uploads across restarts.
- [x] Resume folder uploads and downloads after validating the source tree and staged bytes.
- [x] Review and apply selected directory changes with version checks and backups.
- [x] Keep desktop upload and download outcomes in the task history.
- [x] Retain Agent deployment plans, linked step results, and bounded raw-log archives.
- [x] Add SFTP root navigation and keep the terminal prompt reachable when the window shrinks.
- [ ] Repeat native file and folder transfer, recovery and log export checks on the release build.
- [ ] Complete Developer ID signing and Apple notarization.
- [ ] Extend compatibility and performance coverage beyond the [paired Codex static-site deployment](docs/performance/2026-11-18-agent-deployment.md).

## Later exploration

- Additional deployment guides and server compatibility coverage.
- Windows/Linux desktop packages after platform-specific design and testing.

Built-in model inference, cloud credential hosting and multi-agent orchestration are outside the current scope.

See the [stable release plan](docs/07-stable-release-plan.md) for sequencing and scope, and the [delivery plan](docs/05-delivery-and-acceptance.md) for acceptance criteria.
