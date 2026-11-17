# vibe-ssh: deploy a static site behind Nginx

Every command here was run through vibe-ssh against Debian 13 with nginx 1.26.
Other distributions differ in package names and config paths: probe first and
adapt. Replace `SITE` with a short name for the site and `RELEASE` with a new
release id such as a UTC timestamp.

Layout on the server:

    /srv/SITE/releases/RELEASE/    one directory per upload, never edited afterwards
    /srv/SITE/current         symlink to the release being served

## 1. Probe before changing anything

```bash
vibe-ssh run --lease <lease> <<'EOF'
. /etc/os-release; echo "os=$ID $VERSION_ID"
echo "user=$(id -un) sudo=$(sudo -n true 2>/dev/null && echo passwordless || echo needs-password)"
echo "nginx=$(command -v nginx || ls /usr/sbin/nginx 2>/dev/null || echo absent)"
echo "port80=$(ss -ltnH 'sport = :80' | wc -l) listeners"
echo "sites=$(ls /etc/nginx/sites-enabled 2>/dev/null | tr '\n' ' ')"
EOF
```

- `sudo=needs-password`: run the sudo step with `--pty`, then hand the space to
  the user (`vibe-ssh space handoff`) so they type the password. Never ask for
  it in chat.
- Something else already listens on port 80, or other sites are enabled: this
  server hosts more than your site. Do not remove or replace their config; add
  a `server_name` for your site and tell the user what you found.
- Installing Nginx changes the server. Do it only if the user's request covers
  it: `sudo DEBIAN_FRONTEND=noninteractive apt-get install -y nginx-light`.

A run is not a login shell: `/usr/sbin` is often missing from `PATH`, so
`nginx -t` reports "command not found" for a normal user. Use `sudo nginx -t`.

## 2. Build locally, upload to a new release directory

```bash
vibe-ssh run --lease <lease> -- 'sudo mkdir -p /srv/SITE/releases && sudo chown -R "$(id -un)": /srv/SITE'
vibe-ssh put --lease <lease> ./dist /srv/SITE/releases/RELEASE --exclude .git --exclude node_modules --exclude '.env*'
```

Upload the build output, not the source tree. A release directory is new each
time, so the version being served is never half-overwritten.

## 3. Write the site config and test it

```bash
vibe-ssh run --lease <lease> <<'EOF'
set -e
sudo tee /etc/nginx/sites-available/SITE >/dev/null <<'CONF'
server {
    listen 80;
    listen [::]:80;
    server_name example.com;
    root /srv/SITE/current;
    index index.html;
    location / {
        try_files $uri $uri/ =404;
    }
}
CONF
sudo ln -sfn /etc/nginx/sites-available/SITE /etc/nginx/sites-enabled/SITE
sudo nginx -t
EOF
```

`set -e` with `nginx -t` last means a broken config stops here, before any
reload. If the test fails, the job is `failed` and Nginx keeps serving what it
served before: read `stderr.text`, fix the file, and test again. Never reload
after a failed test.

For a single-page app, use `try_files $uri $uri/ /index.html;`. Only remove the
distribution's `default` site when this is the only site and the user agreed.

## 4. Switch and reload

```bash
vibe-ssh run --lease <lease> -- 'ln -sfn /srv/SITE/releases/RELEASE /srv/SITE/current.new && mv -T /srv/SITE/current.new /srv/SITE/current && sudo systemctl reload nginx'
```

The rename makes the switch atomic. A reload is only needed when the config
changed, but it is harmless otherwise.

## 5. Verify by content, with a short retry

```bash
vibe-ssh run --lease <lease> <<'EOF'
for i in 1 2 3 4 5 6 7 8 9 10; do
  curl -fsS -H 'Host: example.com' http://127.0.0.1/ | grep -q 'TEXT FROM THE NEW VERSION' && { echo verified; exit 0; }
  sleep 0.5
done
echo "new version is not being served"; exit 1
EOF
```

A status code of 200 is not enough: right after a reload the old workers can
still answer for a moment, and a default page also returns 200. Check for text
that only the new version contains. Then check the public URL the user cares
about from your own machine as well; DNS, firewalls and TLS sit outside the
server and can still be wrong.

## 6. Roll back

```bash
vibe-ssh run --lease <lease> -- 'ls /srv/SITE/releases; readlink /srv/SITE/current'
vibe-ssh run --lease <lease> -- 'ln -sfn /srv/SITE/releases/PREVIOUS_RELEASE /srv/SITE/current.new && mv -T /srv/SITE/current.new /srv/SITE/current'
```

Verify again after rolling back. Keep at least the previous release; ask before
deleting old ones.

HTTPS certificates, DNS records and cloud firewall rules are outside this flow.
Say so instead of improvising them.

[vibe-ssh:skill] end of guide
