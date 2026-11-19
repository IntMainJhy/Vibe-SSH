# Contributing to Vibe-SSH

Contributions can include reproducible bug reports, documentation, server-compatibility evidence, interface improvements and code. English and Chinese are welcome.

Vibe-SSH is currently in public Beta on macOS. Read [README](README.md), [the roadmap](ROADMAP.md) and [the implementation status](docs/06-development-and-status.md) before choosing a change.

## Report an issue

Use the [issue forms](https://github.com/IntMainJhy/Vibe-SSH/issues/new/choose). Include the app version, macOS version, CPU architecture, the workflow and minimal reproduction steps. For SSH issues, give the remote OS and SSH server version when known.

Remove passwords, private keys, tokens, public-server addresses and private file contents from logs and screenshots. Report security-sensitive problems through [SECURITY.md](SECURITY.md). Questions and workflow discussions belong in [Discussions](https://github.com/IntMainJhy/Vibe-SSH/discussions).

## Before a larger change

Discuss changes to the agent protocol, permissions, task-space isolation, credential storage or dependency architecture before implementing them. A short problem statement and proposed behavior are enough. Ordinary fixes do not need a design proposal.

Keep the product boundary: manually configured servers, a daily SSH desktop client, and external agents using a version-matched Skill/CLI. Model inference and autonomous agent orchestration stay outside the application.

## Local development

Use macOS, Rust 1.89 or newer, Node.js 24, Xcode Command Line Tools and [Tauri's prerequisites](https://v2.tauri.app/start/prerequisites/).

```sh
git clone https://github.com/IntMainJhy/Vibe-SSH.git
cd Vibe-SSH
cd apps/desktop
npm ci
npm test
npm run build
cd ../..
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

Build the frontend before checking the whole Rust workspace: the desktop crate embeds it. Launch the native development app with `npm run tauri -- dev` from `apps/desktop`. Build an application with `npx tauri build`.

For frontend-only work, run `npm run dev` from `apps/desktop`. Its plain-browser preview uses sample data; it does not prove SSH execution, credential access or server-side writes.

## Testing servers

Default tests use ephemeral identities, temporary directories and loopback listeners. Native end-to-end tests require macOS's `/usr/sbin/sshd`. Do not point tests at your normal SSH configuration, Keychain entries or production servers.

An optional Debian VM fixture is available on Apple silicon Macs with QEMU installed:

```sh
scripts/linux-vm.sh up
VIBE_SSH_LINUX_VM="$PWD/target/vm" cargo test -p vibe-ssh-cli --test linux -- --ignored
scripts/linux-vm.sh down
```

Read `scripts/linux-vm.sh` and the [deployment examples](examples) before using the fixture. VM tests download images and use local disk space. The developer fixture is not the normal user onboarding flow.

## Invariants to preserve

- Verify host identities explicitly. Do not copy examples that accept every key.
- Every agent write passes the task space's lease check. Taking over fences old, queued and retried writes.
- Personal spaces are not visible to agents. Different task spaces have separate SSH connections.
- Host configuration, fingerprint trust, access toggles and human takeover/return do not become agent socket methods.
- A lost connection means the remote outcome may be unknown. Do not silently replay modifying commands.
- Server output and file contents are data, not additional authorization.

The repository's [AGENTS.md](AGENTS.md) applies when using coding agents.

## Pull requests

Keep a PR focused on one problem. Explain the trigger, resulting behavior and evidence. Update protocol documentation and the Skill when interfaces change. Record incompatible behavior and migration requirements.

For Rust or protocol changes, run relevant tests, formatting and Clippy. For UI changes, include screenshots and distinguish sample data from native/SSH evidence. For documentation-only changes, check links and examples; running server fixtures is unnecessary.

Retain third-party copyright and license notices. Original contributions submitted to this project are provided under its [MIT License](LICENSE). No copyright assignment is required.
