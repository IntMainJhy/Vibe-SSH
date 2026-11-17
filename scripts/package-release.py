#!/usr/bin/env python3
"""Package an already built macOS app, installation notes and third-party notices."""
import json
from pathlib import Path
import platform
import shutil
import subprocess
import tomllib

root = Path(__file__).resolve().parents[1]
version = tomllib.loads((root/'Cargo.toml').read_text())['workspace']['package']['version']
arch = platform.machine()
assert arch in ('arm64', 'x86_64') and platform.system() == 'Darwin'
name = f'Vibe-SSH-{version}-macos-{arch}'
out = root/'target/distribution'; out.mkdir(parents=True, exist_ok=True)
stage = out/name
if stage.exists(): shutil.rmtree(stage)
stage.mkdir()
app = root/'target/release/bundle/macos/Vibe-SSH.app'
subprocess.run(['codesign','--verify','--strict','--deep',str(app)],check=True)
subprocess.run(['ditto',str(app),str(stage/'Vibe-SSH.app')],check=True)
shutil.copyfile(root/'docs/installation.md',stage/'安装说明.md')
revision = subprocess.check_output(['git','rev-parse','HEAD'],cwd=root,text=True).strip()
(stage/'BUILD.txt').write_text(f'Version: {version}\nArchitecture: {arch}\nSource: https://github.com/IntMainJhy/Vibe-SSH/commit/{revision}\nSignature: ad-hoc, not Developer ID signed or notarized\n')
notice = stage/'THIRD-PARTY-NOTICES'; notice.mkdir()
metadata = json.loads(subprocess.check_output(['cargo','metadata','--format-version','1','--locked','--filter-platform',f'{"aarch64" if arch == "arm64" else "x86_64"}-apple-darwin'],cwd=root))
manifest = []
for package in metadata['packages']:
    if package['source'] is None: continue
    source = Path(package['manifest_path']).parent
    identifier = f'{package["name"]}-{package["version"]}'
    dest = notice/identifier
    files = [p for p in source.iterdir() if p.is_file() and any(p.name.lower().startswith(prefix) for prefix in ('license','licence','copying','notice','copyright'))]
    dest.mkdir()
    for path in files: shutil.copyfile(path,dest/path.name)
    manifest.append({'name':package['name'],'version':package['version'],'license':package.get('license'),'repository':package.get('repository'),'source':'https://crates.io/crates/'+package['name']+'/'+package['version'],'notice_files':[p.name for p in files]})
# Production frontend dependencies; build-tool-only packages are not shipped in the JS bundle.
packages = subprocess.check_output(['npm','ls','--omit=dev','--all','--parseable'],cwd=root/'apps/desktop',text=True).splitlines()[1:]
for directory in packages:
    source = Path(directory)
    package = json.loads((source/'package.json').read_text())
    dependency = package['name']
    dest = notice/('npm-'+dependency.replace('/','-')+'-'+package['version'])
    if dest.exists(): continue
    dest.mkdir()
    files = [p for p in source.iterdir() if p.is_file() and p.name.lower().startswith(('license','licence','notice','copying','copyright'))]
    for path in files: shutil.copyfile(path,dest/path.name)
    manifest.append({'name':dependency,'version':package['version'],'license':package.get('license'),'source':'https://www.npmjs.com/package/'+dependency,'notice_files':[p.name for p in files]})
(notice/'manifest.json').write_text(json.dumps(manifest,ensure_ascii=False,indent=2)+'\n')
archive = out/(name+'.zip')
subprocess.run(['ditto','-c','-k','--sequesterRsrc','--keepParent',str(stage),str(archive)],check=True)
(stage/'Applications').symlink_to('/Applications')
dmg = out/(name+'.dmg')
if dmg.exists(): dmg.unlink()
subprocess.run(['hdiutil','create','-quiet','-volname',f'Vibe-SSH {version}','-srcfolder',str(stage),'-format','UDZO',str(dmg)],check=True)
subprocess.run(['hdiutil','verify','-quiet',str(dmg)],check=True)
print(json.dumps({'version':version,'architecture':arch,'artifacts':[str(archive),str(dmg)]}))
