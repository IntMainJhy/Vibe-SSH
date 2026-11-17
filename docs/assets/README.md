# Repository visuals

## Promotional artwork

The README uses [vibe-ssh-hero.png](vibe-ssh-hero.png), exported at 1600 × 840 from its editable [SVG source](vibe-ssh-hero.svg) on 2026-11-17. The matching standalone connection mark is available as [SVG](vibe-ssh-mark.svg) and a transparent [1024 × 1024 PNG](vibe-ssh-mark.png).

The white tile, graphite connection segments, cobalt light and subtle terminal/server outlines are promotional artwork, not an application screenshot or evidence of deployment performance. The standalone mark is also the approved application icon, shared by the macOS bundle, Dock, navigation brand and browser favicon. Generate the desktop PNG/ICNS sizes with `python3 scripts/generate-icons.py` on macOS from `vibe-ssh-mark.png`. There are no external fonts, linked images, scripts or third-party logos in the SVG files. These project-owned assets use the repository's MIT license.

To export again, load the SVG in a browser and render it to a PNG canvas at the dimensions above. Use system sans-serif fonts with PingFang SC or Microsoft YaHei for Chinese. Check text, shadows and clipping in the PNG, then keep the README image paths aligned in both languages.

## Platform badge

[platform-macos.svg](platform-macos.svg) is a project-owned static badge for the supported desktop platform. Keeping it in the repository avoids third-party image-proxy failures for this fixed value. Release and CI badges remain live.

## Application screenshots

`workspace.png` and `session.png` show the application frontend at source revision `6db2997249d1043088ee02571e72ad40e182dcf5`, captured on 2026-11-17 through its browser development preview.

They use the existing `dev-mock.ts` sample hosts, addresses and task output. They illustrate the interface, not a live server, completed deployment, native Keychain operation or performance result.

To refresh them, run the frontend in development mode, use sample data and capture the overview and a session. Keep README captions and the source revision accurate. Never capture personal server details or credentials for public documentation.
