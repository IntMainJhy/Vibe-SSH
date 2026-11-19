#!/usr/bin/env python3
"""Verify signed macOS artifacts and the app lifecycle in a disposable profile."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile
import time


def run(*args, env=None):
    return subprocess.check_output(args, env=env, text=True)


def verify(archive):
    sums = (archive.parent / 'SHA256SUMS.txt').read_text().splitlines()
    expected = next(digest for digest, name in (line.split(maxsplit=1) for line in sums)
                    if name.lstrip('*') == archive.name)
    with archive.open('rb') as source:
        assert hashlib.file_digest(source, 'sha256').hexdigest() == expected, archive.name


def extract(archive, directory):
    verify(archive)
    directory.mkdir()
    if archive.suffix == '.dmg':
        mount = directory / 'mounted'
        run('/usr/bin/hdiutil', 'attach', '-nobrowse', '-readonly', '-mountpoint', mount, archive)
        try:
            run('/usr/bin/ditto', mount / 'Vibe-SSH.app', directory / 'Vibe-SSH.app')
        finally:
            run('/usr/bin/hdiutil', 'detach', mount)
    else:
        run('/usr/bin/ditto', '-x', '-k', archive, directory)
    app, = directory.glob('**/Vibe-SSH.app')
    run('/usr/bin/codesign', '--verify', '--strict', '--deep', app)
    return app


def info(app):
    return plistlib.loads((app / 'Contents/Info.plist').read_bytes())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('previous', type=Path)
    parser.add_argument('release', type=Path)
    parser.add_argument('candidate', type=Path)
    parser.add_argument('--zip', type=Path, required=True)
    parser.add_argument('--test-host', type=Path, required=True)
    args = parser.parse_args()
    test_host = json.loads(args.test_host.read_text())
    assert test_host['host'] == '127.0.0.1' and test_host['identity_file']
    assert test_host['host_key_sha256'].startswith('SHA256:') and test_host['agent_access']

    with tempfile.TemporaryDirectory(prefix='vibe-install-', dir='/tmp') as tmp:
        root = Path(tmp)
        old = extract(args.previous, root / 'old')
        release = extract(args.release, root / 'release')
        zipped = extract(args.zip, root / 'zip')
        run('/usr/bin/codesign', '--verify', '--strict', '--deep', args.candidate)
        binary = Path('Contents/MacOS/vibe-ssh-desktop')
        assert (release / binary).read_bytes() == (zipped / binary).read_bytes()
        versions = [info(app)['CFBundleShortVersionString'] for app in (old, release, args.candidate)]
        assert versions[0] != versions[1]

        integration = root / 'profile'
        integration.mkdir()
        data = root / 'data'
        apps = root / 'Applications'
        apps.mkdir()
        installed = apps / 'Vibe-SSH.app'
        cli = integration / '.local/bin/vibe-ssh'
        codex = integration / '.agents/skills/vibe-ssh'
        claude = integration / '.claude/skills/vibe-ssh'
        hosts = data / 'hosts.json'
        history = data / 'history.sqlite'
        env = {**os.environ, 'VIBE_SSH_HOME': str(data), 'VIBE_SSH_INTEGRATION_HOME': str(integration)}
        children = []

        def cli_json(*command):
            return json.loads(run(cli, *command, env=env))

        def start(app):
            process = subprocess.Popen([app / binary], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            children.append(process)
            for _ in range(100):
                assert process.poll() is None, 'app stopped during startup'
                if cli.is_file():
                    status = subprocess.run([cli, 'status'], env=env, text=True, capture_output=True)
                    if status.returncode == 0 and json.loads(status.stdout)['pid'] == process.pid:
                        if (codex / 'SKILL.md').is_file() and (claude / 'SKILL.md').is_file():
                            return process, json.loads(status.stdout)
                time.sleep(.1)
            raise RuntimeError('app did not register its CLI and Skill')

        def stop(process):
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()

        def install(source):
            if installed.exists():
                shutil.rmtree(installed)
            run('/usr/bin/ditto', source, installed)
            run('/usr/bin/codesign', '--verify', '--strict', '--deep', installed)
            return start(installed)

        try:
            assert not any(integration.iterdir()) and not data.exists()
            process, first = install(old)
            assert first['version'] == versions[0] and cli.resolve() == (installed / binary).resolve()
            assert codex.is_symlink() and claude.is_symlink()
            assert cli_json('hosts') == []
            for skill in (codex, claude):
                assert 'vibe-ssh skill' in (skill / 'SKILL.md').read_text()
            print('PASS: signed previous DMG, first launch, empty hosts, both Skill links and CLI', flush=True)

            hosts.write_text(json.dumps([test_host]))
            hosts.chmod(0o600)
            config = hosts.read_bytes()
            session = cli_json('space', 'open', test_host['id'], '--name', 'install acceptance')
            result = cli_json('run', '--lease', session['control']['lease'], '--', 'printf VIBE_INSTALL_OK')
            assert result['stdout']['text'] == 'VIBE_INSTALL_OK'
            record = result.get('record')
            cli_json('space', 'close', '--lease', session['control']['lease'])
            stop(process)

            claude.unlink()
            claude.mkdir()
            marker = claude / 'SKILL.md'
            marker.write_text('unrelated Skill fixture\n')
            process, upgraded = install(release)
            assert upgraded['version'] == versions[1] and upgraded['instance'] != first['instance']
            assert hosts.read_bytes() == config and cli_json('space', 'list') == []
            assert marker.read_text() == 'unrelated Skill fixture\n' and not claude.is_symlink()
            assert 'Read-only directory preview' in run(cli, 'skill', env=env)
            if not record:
                result = cli_json('history', 'list')
                assert result['records'] == []
                session = cli_json('space', 'open', test_host['id'], '--name', 'upgrade acceptance')
                job = cli_json('run', '--lease', session['control']['lease'], '--', 'printf VIBE_INSTALL_OK')
                record = job['record']
                cli_json('space', 'close', '--lease', session['control']['lease'])
            assert cli_json('history', 'get', record)['stdout']['text'] == 'VIBE_INSTALL_OK'
            print('PASS: release DMG/ZIP payload, upgrade, host data, SSH and persisted output', flush=True)
            stop(process)

            process, candidate = install(args.candidate)
            assert candidate['protocol_version'] == 2 and hosts.read_bytes() == config
            assert 'Reenabling access alone' in run(cli, 'skill', env=env)
            assert cli_json('history', 'get', record)['stdout']['text'] == 'VIBE_INSTALL_OK'
            stop(process)

            relocated = apps / 'Relocated Vibe-SSH.app'
            installed.rename(relocated)
            process, _ = start(relocated)
            assert cli.resolve() == (relocated / binary).resolve()
            stop(process)
            shutil.rmtree(relocated)
            assert cli.is_symlink() and not cli.exists()
            assert history.is_file() and hosts.read_bytes() == config
            assert marker.is_file() and (codex / 'SKILL.md').is_file()
            cli.unlink()
            codex.unlink()
            assert marker.read_text() == 'unrelated Skill fixture\n'
            print('PASS: candidate upgrade, version-matched guide, moved-app link repair, uninstall preserves data and foreign Skill', flush=True)
        finally:
            for process in reversed(children):
                stop(process)


if __name__ == '__main__':
    main()
