# Vibe-SSH roadmap

Vibe-SSH is a macOS SSH client that external agents operate through Skills and a CLI. The focus is a useful daily terminal, visible server operations and reliable human takeover.

This is a directional roadmap, not a release-date commitment. Source code and [implementation evidence](docs/06-development-and-status.md) determine whether a capability is available.

## Available in the preview line

- Manual hosts, fingerprint verification and per-host agent access.
- Personal terminals and agent task spaces with separate SSH connections.
- Lease-checked execution, incremental output, upload, handoff and takeover/return.
- A macOS workspace with tabbed terminals, search and appearance controls.
- SFTP operations and bounded UTF-8 editing with version checks and backups.
- Linux resource monitoring while its page is open.
- Read-only directory-diff preview with explicit content selections and ignore rules.
- Bounded durable command/output history, privacy filtering and honest restart outcomes.
- Native core-flow acceptance and encrypted-key passphrase restart in an isolated real Keychain.
- Version-matched Skill/CLI integration and Nginx and Compose reference flows.
- Apple silicon and Intel macOS preview packages.

## Next priorities

- [ ] Complete M3 native-window replay and clean-system installation/update/uninstall acceptance.
- [ ] Verify independent agent-led deployment and recovery against acceptance scenarios.
- [ ] Strengthen project synchronization policies before adding an explicit apply operation.
- [ ] Extend task history to transfer/deployment records and full logs with explicit retention semantics.
- [ ] Complete Developer ID signing and Apple notarization.
- [ ] Extend compatibility and repeatable performance evidence without broad speedup claims.

## Later exploration

- SSH Agent authentication, jump hosts, proxies and port forwarding.
- Additional deployment guides and server compatibility coverage.
- Windows/Linux desktop packages after platform-specific design and testing.

Built-in model inference, cloud credential hosting and multi-agent orchestration are outside the current scope.

Technical acceptance is in [the delivery plan](docs/05-delivery-and-acceptance.md), with milestone-order corrections in [ADR 0002](docs/adr/0002-task-spaces-and-agent-neutrality.md). Discuss proposals in [Discussions](https://github.com/IntMainJhy/Vibe-SSH/discussions) or a feature request before expanding these boundaries.
