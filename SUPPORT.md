# Getting help

English and Chinese are welcome.

| Need | Where to go |
| --- | --- |
| Install or update | [English installation guide](docs/installation.en.md) / [中文安装说明](docs/installation.md) |
| Understand agent commands | Run `vibe-ssh skill`, or read the [guide](skills/vibe-ssh/guide.md) |
| Ask a question or discuss a workflow | [GitHub Discussions](https://github.com/IntMainJhy/Vibe-SSH/discussions) |
| Report a reproducible defect | [Bug report form](https://github.com/IntMainJhy/Vibe-SSH/issues/new/choose) |
| Propose a feature | [Feature request form](https://github.com/IntMainJhy/Vibe-SSH/issues/new/choose) |
| Report a security problem | [Private security reporting](SECURITY.md) |
| See upcoming work | [Roadmap](ROADMAP.md) and [implementation status](docs/06-development-and-status.md) |

Before reporting a bug, note the preview version, macOS version, Apple silicon/Intel architecture and reproduction steps. Remove credentials and sensitive host information from attachments.

If an agent cannot connect, check that Vibe-SSH is open and the host is enabled for agent access. If `vibe-ssh` is not on PATH, try `"$HOME/.local/bin/vibe-ssh" skill`. Do not replace host trust records or repeatedly replay a failed deployment as a troubleshooting shortcut.

Community support is best-effort. Issues and discussions are not a managed production-operations service.
