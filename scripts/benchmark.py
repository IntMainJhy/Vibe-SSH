#!/usr/bin/env python3
"""Read-only comparison against the disposable VM, never the user's SSH config."""
import argparse
import json
import os
from pathlib import Path
import statistics
import subprocess
import tempfile
import time

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument('--cli', type=Path, default=root / 'target/release/vibe-ssh')
parser.add_argument('--out', type=Path, required=True)
parser.add_argument('--samples', type=int, default=30)
args = parser.parse_args()
host = json.loads(subprocess.check_output([str(root / 'scripts/linux-vm.sh'), 'hosts']))[0]
assert host['host'] == '127.0.0.1', 'only disposable loopback VM allowed'

def measured(command, **kwargs):
    start = time.perf_counter_ns()
    result = subprocess.run(command, check=True, capture_output=True, **kwargs)
    return (time.perf_counter_ns() - start) / 1e6, result.stdout

def summary(values):
    values = sorted(values)
    return {'samples':len(values), 'p50_ms':round(statistics.median(values),3), 'p95_ms':round(values[min(len(values)-1, int(len(values)*.95))],3), 'min_ms':round(min(values),3), 'max_ms':round(max(values),3)}

with tempfile.TemporaryDirectory(prefix='vibe-ssh-bench-') as work:
    work = Path(work)
    home = work / 'data'; home.mkdir()
    (home / 'hosts.json').write_text(json.dumps([host]))
    env = dict(os.environ, VIBE_SSH_HOME=str(home))
    service = subprocess.Popen([str(args.cli), 'serve'], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    control = str(work / 'control')
    known = work / 'known_hosts'
    # Obtain exactly the VM's pre-generated public host key. Never use ssh-keyscan trust.
    pub = (root / 'target/vm/host_key.pub').read_text().split()[:2]
    known.write_text(f'[{host["host"]}]:{host["port"]} '+ ' '.join(pub)+'\n')
    ssh = ['/usr/bin/ssh', '-F','/dev/null','-o','BatchMode=yes','-o','IdentitiesOnly=yes','-o','StrictHostKeyChecking=yes','-o',f'UserKnownHostsFile={known}','-i',host['identity_file'],'-p',str(host['port']),'-S',control,f'{host["username"]}@{host["host"]}']
    cli = lambda *cmd: [str(args.cli), *cmd]
    try:
        for _ in range(100):
            if (home/'agent.sock').exists(): break
            if service.poll() is not None: raise RuntimeError('service exited')
            time.sleep(.02)
        _, opened = measured(cli('space','open',host['id'],'--name','read-only benchmark'), env=env)
        lease = json.loads(opened)['control']['lease']
        measured(ssh[:-1] + ['-o','ControlMaster=yes','-o','ControlPersist=no','-f','-N',ssh[-1]])
        # Warm both paths. Include the CLI process, IPC/ControlMaster, session and remote command.
        vibe = cli('run','--lease',lease,'--','true')
        baseline = ssh + ['true']
        measured(vibe,env=env); measured(baseline)
        times = {'vibe_reused':[], 'openssh_reused':[], 'vibe_status':[]}
        for index in range(args.samples):
            paths = [('vibe_reused',vibe,{'env':env}),('openssh_reused',baseline,{})]
            if index % 2: paths.reverse()
            for name, command, kwargs in paths:
                ms, result = measured(command, **kwargs)
                if name == 'vibe_reused': assert json.loads(result)['status'] == 'succeeded'
                times[name].append(ms)
            ms, _ = measured(cli('status'),env=env); times['vibe_status'].append(ms)
        result = {'date':'2026-11-17', 'target':'Debian 13 QEMU on loopback', 'client':'macOS arm64 release build', 'command':'true', 'method':'Alternating order; both use authenticated persistent SSH connections. Each timing includes the CLI process and the remote exit status. One warmup each. No application build/deployment measured.', 'results':{name:summary(values) for name, values in times.items()}, 'limits':'One computer and one loopback VM. No WAN, packet loss or independent Agent workload; no general speedup claim.'}
        args.out.parent.mkdir(parents=True,exist_ok=True); args.out.write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n')
        print(json.dumps(result,ensure_ascii=False))
        measured(cli('space','close','--lease',lease),env=env)
    finally:
        subprocess.run(ssh[:-1]+['-O','exit',ssh[-1]],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        service.terminate()
        try: service.wait(timeout=5)
        except subprocess.TimeoutExpired: service.kill(); service.wait()
