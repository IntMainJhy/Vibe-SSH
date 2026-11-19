# vibe-ssh: deploy a static site behind Nginx

For a built static site on Debian with Nginx already installed. List authorized
hosts with `vibe-ssh hosts` and select the user's intended host. If missing,
ask the user to enable access. Adapt the plan for existing or unfamiliar sites.

## One deployment call

Give `vibe-ssh batch --host HOST --name 'Publish SITE' --actor 'AGENT'` the
JSON below via a quoted heredoc. Use the selected authorized host ID, the
site's actual DNS name and a fresh release directory. Change the example URLs
and expected page text. Inspect the local build and list hosts in the same
read-only call if needed. Put prepared steps in one batch, including checks.
Every step becomes a visible job in the same space.
Use `batch --lease LEASE` to add work to an existing goal.
The batch returns a `deployment` ID. Read it with `vibe-ssh history get` to revisit the plan and its individual task records.

```json
{
  "steps": [
    {
      "type": "run", "name": "Check server and prepare release",
      "command": "set -eu\nsudo -n nginx -t\ntest ! -e /etc/nginx/sites-available/site.example && test ! -L /etc/nginx/sites-available/site.example || { echo 'site already configured' >&2; exit 1; }\nsudo -n install -d -m 0755 /srv/site.example/releases\nsudo -n mkdir -m 0755 /srv/site.example/releases/release-1\nsudo -n chown \"$(id -u):$(id -g)\" /srv/site.example/releases/release-1"
    },
    {
      "type": "put", "name": "Upload built files",
      "local": "./dist", "remote_dir": "/srv/site.example/releases/release-1",
      "exclude": [".git", "node_modules", ".env*"]
    },
    {
      "type": "run", "name": "Configure, publish and verify",
      "command": "set -eu\nrelease=/srv/site.example/releases/release-1\ntest -z \"$(find \"$release\" ! -type d ! -type f -print -quit)\"\nchmod -R a+rX \"$release\"\nsudo -n tee /etc/nginx/sites-available/site.example >/dev/null <<'CONF'\nserver {\n  listen 80;\n  server_name site.example;\n  root /srv/site.example/current;\n  index index.html;\n  location / { try_files $uri $uri/ =404; }\n}\nCONF\nsudo -n ln -s /etc/nginx/sites-available/site.example /etc/nginx/sites-enabled/site.example\nsudo -n nginx -t\nsudo -n ln -s \"$release\" /srv/site.example/current.new\nsudo -n mv -T /srv/site.example/current.new /srv/site.example/current\nsudo -n systemctl reload nginx\nfor i in 1 2 3 4 5; do curl -fsS -H 'Host: site.example' http://127.0.0.1/ | grep -F 'EXPECTED PAGE TEXT' && exit 0; sleep 0.5; done\nexit 1"
    }
  ]
}
```

Upload only approved public build artifacts. Tar can carry restrictive source
permissions, so grant read and directory traversal on the new release before
activating it. The example rejects symlinks and special files. For an existing
site, inspect its config and stage a new release. Keep the old release and
other sites intact. Only install Nginx or change DNS/TLS when authorized.
Check the user-facing URL and page text from the client after the batch, in the
same shell call if possible. The client's forwarded port can differ from the
server's Nginx listener.

If the user explicitly requests a fault-and-recovery exercise, append two
ordered run steps to the same plan after the first HTTP check. First insert the
fault in this site's config and run `nginx -t` in an `if`. Require the expected
nonzero result and diagnostic, print them, and check the working page. Then
restore only that change, run a successful `nginx -t` before reloading and
verify page content again. Never reload an invalid configuration.

A `stopped` batch reports the completed, failed, running or unknown step and
the count of unsubmitted steps. Wait on a running job with its `cursor`.
Check remote state after a missing or unknown result. Never re-run the full
batch to guess what happened. Stop on takeover or a stale lease. For a password
prompt, use a focused PTY run and hand control to the user. Keep or hand off the
space if the user needs to inspect it.

For rollback, inspect `readlink /srv/SITE/current` and the retained releases in
the same task space. After confirming the intended release, point a new
symlink at it and rename it over `current` atomically, then verify the live
content. Use `space get SPACE` to check control and retrieve the current lease
for individually authorized follow-up steps. For details use `job get JOB`.

[vibe-ssh:skill] end of guide
