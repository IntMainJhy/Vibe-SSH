#!/usr/bin/env python3
"""Launch a disposable native app + loopback sshd for window acceptance.

Keep this process running while operating the acceptance window. Ctrl+C cleans
only children, copied bundle, keys and files created by this process.
"""
import argparse
import getpass
import json
import os
from pathlib import Path
import plistlib
import shlex
import signal
import socket
import subprocess
import tempfile
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--app', type=Path, default=ROOT/'target/release/bundle/macos/Vibe-SSH.app')
parser.add_argument('--info', type=Path, default=ROOT/'target/native-acceptance/current.json')
parser.add_argument('--binary', type=Path, help='opt-in native-acceptance build to put in the copied bundle')
args = parser.parse_args()
if not (args.app/'Contents/MacOS/vibe-ssh-desktop').is_file():
    parser.error('build the native app first')
args.info.parent.mkdir(parents=True, exist_ok=True)
children = []
stop = False

def stopped(*_):
    global stop
    stop = True

signal.signal(signal.SIGINT, stopped)
signal.signal(signal.SIGTERM, stopped)
with tempfile.TemporaryDirectory(prefix='vibe-native-', dir='/tmp') as temporary:
    base = Path(temporary)
    try:
        remote = base/'remote'; remote.mkdir()
        uploads = base/'uploads'; uploads.mkdir()
        downloads = base/'downloads'; downloads.mkdir()
        data = base/'data'; data.mkdir()
        integration = base/'integration'; integration.mkdir()
        (base/'acceptance-root').write_text('disposable Vibe-SSH window acceptance\n')
        (remote/'配置.txt').write_text('原始内容\n', encoding='utf-8')
        (uploads/'上传 文件.txt').write_text('native upload 确认\n', encoding='utf-8')
        for name, password in [('host',''), ('identity',''), ('identity_encrypted','native-test-only')]:
            subprocess.run(['/usr/bin/ssh-keygen','-q','-t','ed25519','-N',password,'-f',str(base/name)],check=True)
        fingerprint = subprocess.check_output(['/usr/bin/ssh-keygen','-lf',str(base/'host.pub')],text=True).split()[1]
        (base/'authorized_keys').write_bytes((base/'identity.pub').read_bytes()+(base/'identity_encrypted.pub').read_bytes())
        with socket.socket() as listener:
            listener.bind(('127.0.0.1',0)); port = listener.getsockname()[1]
        # Start the shell and SFTP inside the test directory. All test file actions
        # must stay there; this is not an OS sandbox or a production server.
        wrapper = base/'command.sh'
        wrapper.write_text('#!/bin/sh\nexport LC_CTYPE=UTF-8\ncd '+shlex.quote(str(remote))+' || exit 1\n'
            'case "$SSH_ORIGINAL_COMMAND" in\n'
            '  /usr/libexec/sftp-server*) exec /usr/libexec/sftp-server -d '+shlex.quote(str(remote))+' ;;\n'
            '  "") exec /bin/sh -i ;;\n'
            '  *) exec /bin/sh -c "$SSH_ORIGINAL_COMMAND" ;;\nesac\n')
        wrapper.chmod(0o700)
        config = base/'sshd_config'
        config.write_text(f'Port {port}\nListenAddress 127.0.0.1\nHostKey {base}/host\n'
            f'PidFile {base}/sshd.pid\nAuthorizedKeysFile {base}/authorized_keys\n'
            'PasswordAuthentication no\nKbdInteractiveAuthentication no\nUsePAM no\n'
            'StrictModes no\nLogLevel ERROR\nSubsystem sftp /usr/libexec/sftp-server\n'
            f'ForceCommand {wrapper}\n')
        log = (base/'sshd.log').open('w')
        sshd = subprocess.Popen(['/usr/sbin/sshd','-D','-e','-f',str(config)],stdout=subprocess.DEVNULL,stderr=log)
        children.append(sshd)
        for _ in range(100):
            if sshd.poll() is not None:
                raise RuntimeError('test sshd exited: '+(base/'sshd.log').read_text())
            try:
                with socket.create_connection(('127.0.0.1',port), timeout=.1): break
            except OSError: time.sleep(.1)
        else: raise RuntimeError('test sshd did not listen')
        # A copied bundle gets a separate identity; the user's normal app and its
        # active spaces are never terminated or reconfigured by this harness.
        bundle = base/'Vibe-SSH Acceptance.app'
        subprocess.run(['/usr/bin/ditto',str(args.app.resolve()),str(bundle)],check=True)
        if args.binary:
            import shutil
            shutil.copy2(args.binary, bundle/'Contents/MacOS/vibe-ssh-desktop')
        plist = bundle/'Contents/Info.plist'
        info = plistlib.loads(plist.read_bytes())
        identifier = 'io.github.intmainjhy.vibe-ssh.acceptance.'+uuid.uuid4().hex[:8]
        info.update(CFBundleIdentifier=identifier,CFBundleName='Vibe-SSH Acceptance',CFBundleDisplayName='Vibe-SSH Acceptance')
        plist.write_bytes(plistlib.dumps(info))
        subprocess.run(['/usr/bin/codesign','--force','--sign','-','--deep',str(bundle)],check=True,stdout=subprocess.DEVNULL)
        environment = {**os.environ,'VIBE_SSH_HOME':str(data),'VIBE_SSH_INTEGRATION_HOME':str(integration),'VIBE_SSH_TRACE':'1'}
        app_log = (base/'app.log').open('w')
        app = subprocess.Popen([str(bundle/'Contents/MacOS/vibe-ssh-desktop')],env=environment,stdout=app_log,stderr=app_log)
        children.append(app)
        for _ in range(100):
            if app.poll() is not None: raise RuntimeError('acceptance app exited')
            if (data/'agent.sock').exists(): break
            time.sleep(.1)
        else: raise RuntimeError('acceptance app did not start')
        facts = dict(base=str(base),app=str(bundle),bundle_id=identifier,pid=app.pid,host='127.0.0.1',port=port,
            username=getpass.getuser(),fingerprint=fingerprint,identity=str(base/'identity'),encrypted_identity=str(base/'identity_encrypted'),
            remote=str(remote),uploads=str(uploads),downloads=str(downloads),data=str(data),integration=str(integration),
            cli=str(integration/'.local/bin/vibe-ssh'))
        args.info.write_text(json.dumps(facts,ensure_ascii=False,indent=2)+'\n')
        args.info.chmod(0o600)
        print(json.dumps({'info':str(args.info),'bundle_id':identifier,'pid':app.pid,'port':port},ensure_ascii=False),flush=True)
        reported_exit = False
        while not stop:
            if app.poll() is not None and not reported_exit:
                print('acceptance window exited; fixture remains available for restart',flush=True)
                reported_exit = True
            restart = base/'restart.request'
            if restart.exists() and app.poll() is not None:
                restart.unlink()
                app = subprocess.Popen([str(bundle/'Contents/MacOS/vibe-ssh-desktop')],env=environment,stdout=app_log,stderr=app_log)
                children.append(app)
                facts['pid'] = app.pid
                args.info.write_text(json.dumps(facts,ensure_ascii=False,indent=2)+'\n')
                reported_exit = False
                print('restarted isolated acceptance app',flush=True)
            time.sleep(.25)
    finally:
        for child in reversed(children):
            if child.poll() is None:
                child.terminate()
                try: child.wait(timeout=5)
                except subprocess.TimeoutExpired: child.kill(); child.wait()
        keychain = base/'acceptance.keychain-db'
        if keychain.exists():
            subprocess.run(['/usr/bin/security','delete-keychain',str(keychain)],check=True)
        args.info.unlink(missing_ok=True)
