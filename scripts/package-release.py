#!/usr/bin/env python3
"""Package an already built macOS app, installation notes and third-party notices."""
import json
from pathlib import Path
import platform
import shutil
import subprocess
import tomllib
import plistlib
import re
import argparse

root = Path(__file__).resolve().parents[1]
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--app',type=Path,default=root/'target/release/bundle/macos/Vibe-SSH.app')
args=parser.parse_args()
version = tomllib.loads((root/'Cargo.toml').read_text())['workspace']['package']['version']
arch = platform.machine()
assert arch in ('arm64', 'x86_64') and platform.system() == 'Darwin'
name = f'Vibe-SSH-{version}-macos-{arch}'
out = root/'target/distribution'; out.mkdir(parents=True, exist_ok=True)
stage = out/name
if stage.exists(): shutil.rmtree(stage)
stage.mkdir()
app = args.app.resolve()
assert plistlib.loads((app/'Contents/Info.plist').read_bytes())['CFBundleShortVersionString'] == version, 'rebuild the app before packaging'
subprocess.run(['codesign','--verify','--strict','--deep',str(app)],check=True)
subprocess.run(['ditto',str(app),str(stage/'Vibe-SSH.app')],check=True)
shutil.copyfile(root/'docs/installation.md',stage/'安装说明.md')
revision = subprocess.check_output(['git','rev-parse','HEAD'],cwd=root,text=True).strip()
(stage/'BUILD.txt').write_text(f'Version: {version}\nArchitecture: {arch}\nSource: https://github.com/IntMainJhy/Vibe-SSH/commit/{revision}\nSignature: ad-hoc, not Developer ID signed or notarized\n')
notice = stage/'THIRD-PARTY-NOTICES'; notice.mkdir()
metadata = json.loads(subprocess.check_output(['cargo','metadata','--format-version','1','--locked','--filter-platform',f'{"aarch64" if arch == "arm64" else "x86_64"}-apple-darwin'],cwd=root))
# Some crate archives omit workspace-level notices; retrieve them from the exact
# upstream commit recorded by Cargo, rather than silently shipping only an SPDX label.
upstream_cache = {}
bundled_notices = json.loads((root/'third_party/notices.json').read_text())
def upstream_notices(package, source):
    identifier = f"{package['name']}-{package['version']}"
    if identifier in bundled_notices:
        import hashlib
        result = {}
        for item in bundled_notices[identifier]['files']:
            content = (root/'third_party'/item['file']).read_bytes()
            assert hashlib.sha256(content).hexdigest() == item['sha256'], 'bundled notice checksum mismatch'
            result[item['original_name']] = content
        return result
    info_path = source/'.cargo_vcs_info.json'
    repository = (package.get('repository') or '').removesuffix('.git').rstrip('/')
    if not info_path.exists() or not repository.startswith('https://github.com/'):
        return {}
    info = json.loads(info_path.read_text())
    sha = info.get('git',{}).get('sha1','')
    if not re.fullmatch('[0-9a-f]{40}',sha): return {}
    cache_key = repository, sha
    if cache_key in upstream_cache: return upstream_cache[cache_key]
    result = {}
    for name in ('LICENSE', 'LICENSE.md', 'LICENSE.txt', 'LICENSE-MIT', 'LICENSE-APACHE', 'COPYING', 'COPYRIGHT'):
        url = repository.replace('https://github.com/','https://raw.githubusercontent.com/')+'/'+sha+'/'+name
        response = subprocess.run(['/usr/bin/curl','--proto','=https','--tlsv1.2','-sSL','--max-time','15','--max-filesize','1048576','--write-out','%{http_code}',url],capture_output=True,check=True).stdout
        content, status = response[:-3], response[-3:]
        if status == b'404': continue
        if status != b'200': raise RuntimeError('cannot fetch upstream notice: '+url)
        content.decode('utf-8')
        result['UPSTREAM-'+name] = content
        break
    if not result and package.get('license') in ('Apache-2.0', 'MPL-2.0'):
        license_name = package['license']+'.txt'
        result[license_name] = (root/'third_party/licenses'/license_name).read_bytes()
    upstream_cache[cache_key] = result
    return result
manifest = []
for package in metadata['packages']:
    if package['source'] is None: continue
    source = Path(package['manifest_path']).parent
    identifier = f'{package["name"]}-{package["version"]}'
    dest = notice/identifier
    files = [p for p in source.iterdir() if p.is_file() and any(p.name.lower().startswith(prefix) for prefix in ('license','licence','copying','notice','copyright'))]
    dest.mkdir()
    for path in files: shutil.copyfile(path,dest/path.name)
    if not files:
        for filename, content in upstream_notices(package, source).items(): (dest/filename).write_bytes(content)
        if not list(dest.iterdir()): raise RuntimeError('missing third-party notice: '+identifier)
    manifest.append({'name':package['name'],'version':package['version'],'license':package.get('license'),'repository':package.get('repository'),'source':'https://crates.io/crates/'+package['name']+'/'+package['version'],'notice_files':[p.name for p in dest.iterdir()]})
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
