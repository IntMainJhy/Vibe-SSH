# Repository presentation and maintenance

## Positioning

Vibe-SSH is an open-source macOS SSH/SFTP client for people and external AI agents. Users add servers manually. Agents use Skills and the local CLI; people watch and take over isolated task spaces.

Keep the English README and Chinese README aligned on features, platform support and preview limits. Link detailed evidence rather than copying engineering history into the introduction. Do not market an unreleased working-tree feature as part of a download.

## GitHub About fields

Description:

> Open-source macOS SSH/SFTP client for people and AI agents. Let Codex, Claude Code and other CLI agents deploy and troubleshoot your servers through Skills, with isolated task spaces, live terminal output and human takeover. Rust + Tauri. 支持外部 AI 操作与人工接管的 SSH 客户端。

Website: <https://github.com/IntMainJhy/Vibe-SSH/releases>

Topics:

```text
ssh ssh-client sftp terminal macos ai-agents agent-skills codex
claude-code vibe-coding devops deployment remote-development rust tauri typescript
```

These fields are configured in GitHub's repository settings; this document is their maintenance record, not an automatically applied configuration file. Do not add `mcp`, unsupported desktop platforms or unrelated trending topics to attract traffic.

GitHub repository search supports the repository name, description and Topics, and README searches with `in:readme`. Use accurate product terms in both languages. Topics and public documentation improve discoverability, but do not guarantee indexing time, rankings, stars or downloads.

Sources: [repository search](https://docs.github.com/en/search-github/searching-on-github/searching-for-repositories) and [repository Topics](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/classifying-your-repository-with-topics).

## AtomGit repository and manual synchronization

Public repository: <https://gitcode.com/InMainJhy/Vibe-SSH>. The AtomGit account is `InMainJhy`; the GitHub account is `IntMainJhy`.

**The owner requested manual synchronization.** The previously configured AtomGit Pull mirror has been removed. Keep automatic Pull/Push mirroring disabled; do not recreate it or add scheduled synchronization without a new explicit request.

GitHub is the source of truth. Publish reviewed changes there first, then push the published main branch to AtomGit using the existing `atomgit` remote at `https://gitcode.com/InMainJhy/Vibe-SSH.git`:

```sh
git fetch origin
git push atomgit refs/remotes/origin/main:refs/heads/main
git ls-remote origin refs/heads/main
git ls-remote atomgit refs/heads/main
```

The remote name is only a local Git alias; it does not enable automatic synchronization. Check that both main-branch SHAs match before reporting completion. The same commit SHA guarantees identical tracked files, including both READMEs and the promotional assets. If a push is rejected because the destination diverged, inspect the differences before proceeding; do not force push or use `git push --mirror`.

Synchronize other published branches or tags explicitly when requested, preserving their existing commit IDs. Git pushes do not copy GitHub Issues, Discussions, Actions runs or release attachment files; download links continue to point to GitHub Releases.

AtomGit's description records the external-agent workflow, manual server setup, human takeover, Rust/Tauri stack, MIT license and macOS preview scope. Its five project tags are `ssh`, `ai-agents`, `codex`, `sftp` and `devops`. The project URL points to the GitHub Releases page.

## Community entry points

Enable Discussions for questions and workflow sharing, and private vulnerability reporting for sensitive security issues. The issue chooser links to both. Public issues use bilingual bug and feature forms; contributions follow the PR template and contribution guide.

Keep Issues enabled. Do not create placeholder issues, fabricated testimonials, fake usage counts or automatic promotional messages. Releases remain previews until the actual release gates are met.

## Licensing

The root LICENSE is MIT, attributed to IntMainJhy and Vibe-SSH contributors. Rust packages inherit the workspace license; the frontend declares MIT. Release packaging copies LICENSE along with English/Chinese installation notes and separate third-party notices.

The GitHub license badge links to the actual license file. Existing published archives are not rebuilt by a documentation update; the packaging additions apply to future releases.

## Updating the public page

1. Read the current source, release notes and implementation evidence before changing capability claims.
2. Update both READMEs, the roadmap or changelog when relevant. Check downloads against real release assets.
3. Refresh interface screenshots from public-safe sample data; record their source in [assets](../assets/README.md).
4. Check local links, image files, issue-form YAML, package metadata and packaging syntax.
5. Publish focused changes, verify the main-branch rendering and GitHub About fields, and inspect the resulting workflow status.
