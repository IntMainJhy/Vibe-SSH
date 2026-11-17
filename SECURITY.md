# Security policy

Vibe-SSH manages SSH connections and credentials. Flaws involving host verification, credential disclosure, task-space isolation, takeover or local IPC should be reported privately.

## Report a vulnerability

Use GitHub's [private vulnerability reporting](https://github.com/IntMainJhy/Vibe-SSH/security/advisories/new). Do not open a public issue with exploit details, credentials or a live server address.

Include the affected version or commit, operating system, prerequisites, a minimal reproduction and expected impact. Temporary keys, loopback servers and sanitized logs are preferred. Do not test against other people's infrastructure.

The maintainer will assess the report, request evidence if needed, and coordinate fixes and disclosure in the private advisory. The project does not promise a response-time SLA or a bounty.

## Supported versions

Security fixes are developed on `main` and included in subsequent preview releases. Older previews are not maintained as independent branches. Use a current preview or reviewed source revision when reproducing a report.

The application is still preview software. Signing, notarization and remaining native acceptance work are tracked in [the roadmap](ROADMAP.md).

## Relevant boundaries

- Hosts are added and trusted by the user; agent access is enabled individually.
- Human-only operations are not part of the agent socket protocol.
- Every agent write is checked against a task-space lease. Personal spaces are not exposed to agents.
- Credentials use the application's credential path; server output can still contain secrets. Review logs before sharing.
- A same-user process with broad local permissions is not treated as a fully sandboxed adversary.

See the detailed [security and reliability design](docs/04-security-and-reliability.md). Third-party reports should identify the dependency and version so the maintainer can assess whether the relevant code path is shipped or reachable.
