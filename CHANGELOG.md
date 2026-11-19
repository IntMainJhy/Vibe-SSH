# Changelog

Packages and original notes are in [GitHub Releases](https://github.com/IntMainJhy/Vibe-SSH/releases). The 0.2.0 release line is in public Beta.

## Unreleased

- Offer current project-owned Personal edition source under PolyForm Strict 1.0.0 for permitted noncommercial use, without modification or redistribution grants. Describe it as source-available; keep Enterprise Pro proprietary and separate.
- Preserve previous MIT grants and notices, including Beta 1 downloads. New-license packages must use a new release version. See [licensing scope and migration](LICENSING.md).

## 0.2.0-beta.1

Public macOS Beta for Apple silicon and Intel. See the [release notes](docs/releases/0.2.0-beta.1.md).

- Run ordered command and upload plans through one Agent CLI call, with per-step receipts and lease checks.
- Open and stop remote TCP forwards to a local loopback port from a personal desktop session.
- Review and apply selected directory additions and replacements with version checks, original-file backups, and per-file receipts.
- Pause and resume single-file SFTP uploads against the original server after checking the local file and the remote partial content.
- Retain desktop upload and download results, remote paths, and failure details in personal history.
- Resume interrupted folder uploads and downloads from verified staged bytes and publish the complete tree.
- Retain Agent batch plans and step outcomes, with links to task logs.
- Read archived output by byte offset and export the original stream from the desktop history.
- Configure an outbound HTTP CONNECT or SOCKS5 proxy for a server.
- Pause single-file SFTP downloads and resume them after reconnecting, with verified source metadata and matching partial content.
- Find and export retained terminal text from the desktop session.
- Offer compact job results for Agent deployment commands via `run --summary` and `put --summary`.
- Preview OpenSSH host imports, authenticate with a local key agent, and connect through up to four verified jump hosts.
- Forward fixed TCP ports or use a loopback SOCKS5 proxy in a personal session.
- Transfer folders with SFTP, edit regular-file permissions, and inspect Linux disk mount usage.
- Simplify the monochrome overview and keep file and resource shortcuts with the active session.
- Let agents list remote files, read small UTF-8 files, and sample Linux resources without creating command jobs.
- Explain local socket permission failures instead of reporting that the app is stopped.
- Revoke live Agent writes when a host is disabled or removed. Keep output visible and require explicit handback after reenabling access.
- Use the approved connection icon in the app bundle, Dock and navigation.
- Browse the SFTP root or account home from the locations pane, use clickable breadcrumbs or enter a path, and open files and folders with a double-click or Enter.
- Sort and filter loaded entries, rendering at most 100 items per page. Keep the current directory visible if a folder cannot be opened.
- Fit terminal rows to the visible area and keep the prompt and session controls reachable in short windows.
- Retry failed read-only monitoring, pause hidden-page polling and serialize refresh bursts.

## 0.2.0-preview.5

- Complete native core-flow and isolated Keychain passphrase restart acceptance.
- Add read-only directory diff, explicit content selections and ignore preview in the app and CLI.
- Persist task intent, results and bounded output tails with unknown restart outcomes and no replay.
- Add history privacy and disk/schema/concurrency failure coverage, plus real process restart tests.
- Refine compact history/diff layouts and keep the selected terminal tab visible.


- Adopt MIT and align Rust/frontend license metadata.
- Add English/Chinese introductions, installation guidance and interface screenshots.
- Add contribution, support, security and community guidelines, issue forms and a PR template.

[Release](https://github.com/IntMainJhy/Vibe-SSH/releases/tag/v0.2.0-preview.5) · [Detailed notes](docs/releases/0.2.0-preview.5.md)

## 0.2.0-preview.4

- Add overview, server and session views around the monochrome SSH workspace.
- Add cross-object search and navigation from failed tasks to their output.
- Refine resource trends, agent activity placement, compact layouts and appearance.
- Keep file editing, SFTP and Skill/CLI features with protocol v2.

[Release](https://github.com/IntMainJhy/Vibe-SSH/releases/tag/v0.2.0-preview.4) · [Detailed notes](docs/releases/0.2.0-preview.4.md)

## 0.2.0-preview.3

- Add bounded UTF-8 editing, BOM and line-ending preservation, version checks and backups.
- Preserve drafts on conflict or control changes; include open documents in exit protection.
- Integrate earlier workbench branches into `main`.

[Release](https://github.com/IntMainJhy/Vibe-SSH/releases/tag/v0.2.0-preview.3) · [Detailed notes](docs/releases/0.2.0-preview.3.md)

## 0.2.0-preview.2

- Refine large-text file-page scrolling and transfer-error visibility.
- Improve collection of upstream dependency notices for release packaging.

[Release](https://github.com/IntMainJhy/Vibe-SSH/releases/tag/v0.2.0-preview.2) · [Detailed notes](docs/releases/0.2.0-preview.2.md)

## 0.2.0-preview.1

- Introduce terminal/file/monitoring pages, SFTP management and Linux monitoring.
- Synchronize Skill and CLI links at startup.
- Publish Apple silicon and Intel DMG/ZIP previews with checksums and dependency notices.

[Release](https://github.com/IntMainJhy/Vibe-SSH/releases/tag/v0.2.0-preview.1) · [Detailed notes](docs/releases/0.2.0-preview.1.md)
