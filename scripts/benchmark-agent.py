"""Compare independent Codex deployments on a disposable scripts/linux-vm.sh VM."""

import argparse
import http.client
import json
import os
import re
import shutil
import subprocess
import threading
import time
import uuid
from pathlib import Path


BODY = b"Vibe-SSH paired deployment fixture v1\n"
ASSET = bytes(range(256)) * 2048


def fetch(port, site, path="/"):
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=2)
    try:
        connection.request("GET", path, headers={"Host": site})
        response = connection.getresponse()
        return response.read(len(ASSET) + 1) if response.status == 200 else None
    except (OSError, http.client.HTTPException):
        return None
    finally:
        connection.close()


def events(path):
    commands = {}
    usage = None
    completed = False
    for line in path.read_text().splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        item = event.get("item", {})
        if item.get("type") == "command_execution":
            commands[item["id"]] = item
        if event.get("type") == "turn.completed":
            completed = True
            usage = event.get("usage")
    return commands.values(), usage, completed


def run(root, args, pair, arm):
    site = f"bench-{uuid.uuid4().hex[:10]}.test"
    work = root / site
    dist = work / "dist"
    dist.mkdir(parents=True)
    (dist / "index.html").write_bytes(BODY)
    (dist / "asset.bin").write_bytes(ASSET)
    ssh_socket = work / "control"
    ssh_config = work / "ssh-config"
    vm = root / "target/vm"
    ssh_config.write_text(
        f"Host fixture\n"
        f"  HostName 127.0.0.1\n  Port {args.ssh_port}\n  User deploy\n"
        f"  IdentityFile {vm}/id_deploy\n  IdentityAgent none\n"
        f"  IdentitiesOnly yes\n  BatchMode yes\n  StrictHostKeyChecking yes\n"
        f"  GlobalKnownHostsFile /dev/null\n  UserKnownHostsFile {vm}/known_hosts\n"
        f"  UpdateHostKeys no\n  HostKeyAlgorithms ssh-ed25519\n"
        f"  ControlPath {ssh_socket}\n  ControlMaster auto\n"
    )
    cli = root / "integration/.local/bin/vibe-ssh"
    skill = root / "integration/.agents/skills/vibe-ssh/SKILL.md"
    vibe = arm == "vibe-ssh"
    if vibe and not (cli.is_file() and skill.is_file()):
        raise ValueError("launch the candidate and verify the isolated CLI and Skill")
    endpoint = root / "data/agent.sock" if vibe else ssh_socket
    instructions = (
        f"阅读候选应用已经安装的 Skill {skill}。通过 CLI {cli} 完成远端操作，"
        "只用 Vibe-SSH，不用独立 ssh、scp 或 rsync。"
        if vibe
        else f"通过系统 OpenSSH、SCP 和配置 {ssh_config} 完成远端操作。"
        "ssh -F 或 scp -F 使用别名 fixture，允许复用已配置的连接。在本机只使用这些 SSH 工具操作远端。"
    )
    prompt = f"""
{instructions}
这是专用的回环地址 Debian 测试机。Nginx 已安装。将当前目录 dist 的两个文件完整发布到
站点 {site}，HTTP Host 为 {site}。使用它自己的 /srv/{site} 与 Nginx 配置，保留其他站点。
来宾监听端口 80，客户端访问 http://127.0.0.1:{args.http_port}/ 并带上上述 Host。
上线后通过 HTTP 检查正文包含 “Vibe-SSH paired deployment fixture v1”。
随后在此站点自己的已启用配置里故意加一条无效指令 benchmark_invalid_directive，
实际执行 nginx 配置检查，确认该指令报错，切勿加载坏配置。故障期间再次核验旧页面正文。
修复配置、检查、重载，并从 HTTP 再次核验正文。输出观察到的检查结果。
只操作测试站点和测试机，不使用其他服务器、浏览器、额外部署工具或独立代理。
"""
    env = os.environ.copy()
    if vibe:
        env["VIBE_SSH_HOME"] = str(root / "data")
    else:
        env.pop("VIBE_SSH_HOME", None)
    options = [
        "default_permissions=\"deployment_bench\"",
        'permissions.deployment_bench.filesystem={":root"="read"}',
        "permissions.deployment_bench.network.enabled=true",
        f"permissions.deployment_bench.network.unix_sockets={{{json.dumps(str(endpoint))}=\"allow\"}}",
        'shell_environment_policy.include_only=["PATH","HOME","VIBE_SSH_HOME","LANG","TMPDIR","SHELL"]',
    ]
    codex = str(args.codex)
    version = subprocess.check_output([codex, "--version"], text=True, timeout=10).strip()
    command = [
        codex, "-a", "never", "--no-daemon", "exec", "--ephemeral",
        "--ignore-user-config", "--ignore-rules", "--skip-git-repo-check", "--json",
    ]
    for item in options:
        command.extend(["-c", item])
    if args.model:
        command.extend(["--model", args.model])

    stop = threading.Event()
    samples = []

    def sample_http():
        while not stop.is_set():
            samples.append(fetch(args.http_port, site) == BODY)
            stop.wait(0.5)

    monitor = threading.Thread(target=sample_http)
    monitor.start()
    stdout = work / "events.jsonl"
    started = time.monotonic()
    returncode = None
    master = ["ssh", "-F", str(ssh_config)]
    try:
        if not vibe:
            subprocess.run([*master, "-MNf", "fixture"], check=True, timeout=10)
        with stdout.open("w") as out, (work / "stderr.txt").open("w") as err:
            result = subprocess.run(
                [*command, "-"], input=prompt, text=True, cwd=work, env=env,
                stdout=out, stderr=err, timeout=args.timeout,
            )
        returncode = result.returncode
    except subprocess.TimeoutExpired:
        pass
    finally:
        body_ok = fetch(args.http_port, site) == BODY
        asset_ok = fetch(args.http_port, site, "/asset.bin") == ASSET
        elapsed = round(time.monotonic() - started, 3)
        stop.set()
        monitor.join()
        if not vibe:
            subprocess.run(
                [*master, "-O", "exit", "fixture"], capture_output=True, timeout=10,
            )
    commands, usage, completed = events(stdout)
    output = [item.get("aggregated_output", "") for item in commands]
    transcript = "\n".join(output).replace(r'\"', '"')
    invalid = re.search(r'unknown directive ["\']benchmark_invalid_directive["\']', transcript)
    config_ok = bool(invalid and "test is successful" in transcript[invalid.end():])
    first_up = next((i for i, ok in enumerate(samples) if ok), len(samples))
    return {
        "pair": pair, "arm": arm, "fixture": site, "wall_s": elapsed,
        "codex": version, "model": args.model or "default",
        "agent_exit": returncode, "turn_completed": completed, "shell_calls": len(commands),
        "tool_output_bytes": sum(len(text.encode()) for text in output), "tokens": usage,
        "remote_tool_shell_calls": sum(
            bool(re.search(r"\bvibe-ssh\b" if vibe else r"\b(?:ssh|scp) -F\b", item["command"]))
            for item in commands
        ),
        "fixture_body_ok": body_ok, "fixture_asset_ok": asset_ok,
        "first_up_observed": first_up < len(samples),
        "observed_offline_samples_after_first_up": samples[first_up:].count(False),
        "invalid_nginx_directive_observed": bool(invalid), "valid_nginx_check_after_fault": config_ok,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fixture", type=Path)
    parser.add_argument("--ssh-port", type=int, required=True)
    parser.add_argument("--http-port", type=int, required=True)
    parser.add_argument("--pairs", type=int, default=2)
    parser.add_argument("--timeout", type=int, default=240)
    parser.add_argument("--codex", type=Path, default=Path("codex"))
    parser.add_argument("--model")
    args = parser.parse_args()
    executable = shutil.which(args.codex)
    if executable is None:
        parser.error("Codex executable not found")
    args.codex = Path(executable).resolve()
    root = args.fixture.resolve()
    if not (root / "target/vm/qemu.pid").is_file() or not root.is_relative_to("/private/tmp"):
        parser.error("expected a disposable local VM fixture under /private/tmp")
    os.umask(0o077)
    arms = ["vibe-ssh", "openssh"]
    for pair in range(1, args.pairs + 1):
        for arm in arms:
            print(json.dumps(run(root, args, pair, arm)), flush=True)
        arms.reverse()


if __name__ == "__main__":
    main()
