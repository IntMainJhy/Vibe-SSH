#!/usr/bin/env python3
"""Generate macOS app icons from the approved transparent brand artwork."""
from pathlib import Path
import shutil
import subprocess
import tempfile


root = Path(__file__).resolve().parents[1]
source = root / 'docs/assets/vibe-ssh-mark.png'
destination = root / 'apps/desktop/src-tauri/icons'


def resize(size: int, output: Path) -> None:
    subprocess.run(
        ['sips', '-z', str(size), str(size), str(source), '--out', str(output)],
        check=True,
        stdout=subprocess.DEVNULL,
    )


destination.mkdir(parents=True, exist_ok=True)
shutil.copyfile(source, destination / 'icon.png')
for size, filename in ((32, '32x32.png'), (128, '128x128.png'), (256, '128x128@2x.png')):
    resize(size, destination / filename)

with tempfile.TemporaryDirectory(prefix='vibe-ssh-icons-') as temporary:
    iconset = Path(temporary) / 'Vibe-SSH.iconset'
    iconset.mkdir()
    for size in (16, 32, 128, 256, 512):
        resize(size, iconset / f'icon_{size}x{size}.png')
        resize(size * 2, iconset / f'icon_{size}x{size}@2x.png')
    subprocess.run(
        ['iconutil', '-c', 'icns', str(iconset), '-o', str(destination / 'icon.icns')],
        check=True,
    )

print('Updated desktop icons from docs/assets/vibe-ssh-mark.png')
