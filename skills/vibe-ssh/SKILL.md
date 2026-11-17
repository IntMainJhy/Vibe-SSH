---
name: vibe-ssh
description: Operate the user's own servers over SSH through the Vibe-SSH app - run commands, upload project files, deploy and troubleshoot - while the user watches and can take over. Use it whenever a task needs a remote server the user has added to Vibe-SSH, instead of calling ssh or scp directly.
metadata:
  bootstrap: "true"
---

# vibe-ssh

Before any server task, run:

```bash
vibe-ssh skill
```

and follow its output. It prints the usage guide that matches the installed
version. The guide ends with a line starting `[vibe-ssh:skill] end of guide`;
if that line is missing the output was truncated, so save it to a file and read
the file in parts.

If `vibe-ssh` is not on PATH, run `"$HOME/.local/bin/vibe-ssh" skill`.
Opening the installed Vibe-SSH app automatically configures the version-matched
Skill and command. If neither command exists, ask the user to open the app and
check its integration warning; never overwrite unrelated skills or fall back
to plain `ssh` with the user's keys.
