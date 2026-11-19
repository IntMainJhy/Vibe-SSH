# Install the Vibe-SSH macOS Beta

Vibe-SSH is an SSH desktop client with an interface for external AI agents. Current desktop packages are for macOS; Linux refers to remote servers.

## Download and verify

Open the [Beta release](https://github.com/IntMainJhy/Vibe-SSH/releases/tag/v0.2.0-beta.1) and choose a package matching your Mac:

| Mac | Package suffix |
| --- | --- |
| Apple silicon | `macos-arm64.dmg` or `macos-arm64.zip` |
| Intel | `macos-x86_64.dmg` or `macos-x86_64.zip` |

Download `SHA256SUMS.txt` from the same release. In the download directory, calculate the package's SHA-256 and compare the value and filename with that file:

```sh
shasum -a 256 <downloaded-package.dmg>
```

Move `Vibe-SSH.app` into **Applications** before first launch. For ZIP packages, extract the archive and move the app from the extracted folder.

## First launch

Beta packages are ad-hoc signed. They are not yet Developer ID signed or Apple notarized, so macOS may block first launch. After checking the source and checksum, use **Privacy & Security → Open Anyway** if offered. Keep system-wide security checks enabled.

Add a host manually with its address, port, username and authentication method. Verify its fingerprint through a trusted channel before saving. Agent access is off by default and is enabled separately for each host.

The server page can preview OpenSSH configuration imports. Supported routes include saved jump hosts and unauthenticated HTTP CONNECT and SOCKS5 proxies. The app can use a local SSH key agent. Personal sessions can manage local, dynamic and remote TCP forwarding.

## Connect an external agent

On each launch, the app synchronizes its embedded bootstrap Skill at:

- `~/.agents/skills/vibe-ssh`
- `~/.claude/skills/vibe-ssh`

It also creates `~/.local/bin/vibe-ssh`. Unrelated existing entries are preserved and reported in the app. Shell configuration is not modified.

Keep the app open and run:

```sh
vibe-ssh skill
# If ~/.local/bin is not on the agent's PATH:
"$HOME/.local/bin/vibe-ssh" skill
```

The command prints the guide matching the installed version. An agent needs local command execution and the relevant Skill discovery convention. If a host is missing, check its agent-access toggle in the app.

If `vibe-ssh status` works in a terminal but fails inside an agent, check whether the agent host permits access to `~/.vibe-ssh/agent.sock`. Where supported, allow only the resolved absolute socket path. Vibe-SSH does not change agent sandbox settings. An unavailable model gateway can prevent an agent from running even when the Skill is installed.

Moving the app changes its executable path. Launch it from its new location to repair links owned by Vibe-SSH. Unrelated same-name entries are not automatically replaced.

## Updates and uninstalling

Quit the app before replacing it with a newer package. Relaunch from Applications to refresh version-matched links. Connections, active task IDs and leases end on restart. Existing task and output records remain in `~/.vibe-ssh/history.sqlite` (schema 1). New jobs also save bounded raw output separately. See the [history retention details](adr/0007-task-history.md). Previously unfinished records become unknown; the app never replays commands. Check remote state before considering another operation. Unknown/corrupt schemas are preserved and require a compatible app.

To uninstall, quit and move the app to the Trash. Host configuration lives at `~/.vibe-ssh/hosts.json`; credentials use the Keychain service `io.github.intmainjhy.vibe-ssh`. Task history is also retained in `~/.vibe-ssh/history.sqlite`. Removing the app does not automatically delete those records. Remove only Vibe-SSH records and links you intend to remove, preserving unrelated entries.

## Current limits

See the [Beta release notes](releases/0.2.0-beta.1.md) for tested environments, known limitations and feedback links.

[中文安装说明](installation.md) · [Support](../SUPPORT.md) · [Security policy](../SECURITY.md)
