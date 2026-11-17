#!/usr/bin/env python3
"""Runs chosen bash blocks of a deployment guide exactly as written, placeholders filled in.

Used to check that skills/vibe-ssh/*.md still work against a real server:

    scripts/run-guide.py <guide.md> <lease> <block,block,...> [OLD=NEW ...]

`vibe-ssh` must be on PATH and VIBE_SSH_HOME must point at the running service's home.
Exits 1 if any job in the chosen blocks did not succeed.
"""
import json, os, re, subprocess, sys

guide, lease, picks, subs = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4:]
blocks = re.findall(r"```bash\n(.*?)```", open(guide, encoding="utf-8").read(), flags=re.S)
env = os.environ
failed = False
for index in (int(n) for n in picks.split(",")):
    script = blocks[index - 1].replace("<lease>", lease)
    for pair in subs:
        old, new = pair.split("=", 1)
        script = script.replace(old, new)
    done = subprocess.run(["bash", "-c", script], env=env, capture_output=True, text=True)
    text, at, decoder = done.stdout, 0, json.JSONDecoder()
    while at < len(text):
        while at < len(text) and text[at].isspace():
            at += 1
        if at >= len(text):
            break
        job, at = decoder.raw_decode(text, at)
        out = (job.get("stdout", {}).get("text", "") + job.get("stderr", {}).get("text", "")).strip()
        print(f"  block {index}: {job.get('status')} exit={job.get('exit_code')} | {out[-260:]!r}")
        failed |= job.get("status") not in ("succeeded",)
    if done.stderr.strip():
        print(f"  block {index}: TOOL ERROR {done.stderr.strip()[:300]}")
        failed = True
sys.exit(1 if failed else 0)
