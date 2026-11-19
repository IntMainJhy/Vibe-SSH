# vibe-ssh: deploy a Docker Compose project

Every command here was run through vibe-ssh against Debian 13 with Docker 26
and Compose 2.26. Replace `APP` with a short project name and `RELEASE` with a new
release id such as a UTC timestamp.

Layout on the server:

    /srv/APP/releases/RELEASE/    one directory per upload
    /srv/APP/current         symlink to the last release that became healthy

Always pass the same project name (`-p APP`). Without it Compose names the
project after the directory, and every release would start a second copy of
the app instead of replacing the first.

After probing, put the known upload, validation, start and verification steps
in a `vibe-ssh batch --lease LEASE` plan to keep one deployment record. Use
`history get` with the returned deployment ID to find each task and its logs.
If a build outlasts the wait, follow the existing job before arranging the
remaining steps.

## 1. Probe before changing anything

```bash
vibe-ssh run --lease <lease> <<'EOF'
. /etc/os-release; echo "os=$ID $VERSION_ID"
echo "user=$(id -un) sudo=$(sudo -n true 2>/dev/null && echo passwordless || echo needs-password)"
echo "docker=$(sudo -n docker version --format '{{.Server.Version}}' 2>/dev/null || echo absent)"
echo "compose=$(sudo -n docker compose version --short 2>/dev/null || echo absent)"
echo "projects=$(sudo -n docker compose ls -q 2>/dev/null | tr '\n' ' ')"
echo "listening=$(ss -ltnH | awk '{print $4}' | sort -u | tr '\n' ' ')"
EOF
```

- Installing Docker changes the server; do it only if the user's request covers
  it. On Debian 13: `sudo DEBIAN_FRONTEND=noninteractive apt-get install -y docker.io docker-compose`.
  It takes a while: use `--wait 120` and follow with `vibe-ssh job wait`.
- Check that the ports your compose file publishes are free, and that no other
  project already uses the name `APP`.
- The compose file needs a `healthcheck` for the service that matters.
  Without one, "started" is the only thing Compose can confirm.

## 2. Upload a new release

```bash
vibe-ssh run --lease <lease> -- 'sudo mkdir -p /srv/APP/releases && sudo chown -R "$(id -un)": /srv/APP'
vibe-ssh put --lease <lease> . /srv/APP/releases/RELEASE --exclude .git --exclude node_modules --exclude '.env*'
```

Secrets are excluded on purpose. If the app needs an env file on the server,
the user decides how it gets there; do not upload local secret files unasked.

## 3. Validate, build, start, and wait for health

```bash
vibe-ssh run --lease <lease> --cwd /srv/APP/releases/RELEASE --wait 120 --tail 2000 <<'EOF'
sudo docker compose -p APP config -q || { echo "compose file is invalid"; exit 1; }
if sudo docker compose --ansi never -p APP up -d --build --wait >/tmp/APP-up.log 2>&1; then
  ln -sfn /srv/APP/releases/RELEASE /srv/APP/current.new && mv -T /srv/APP/current.new /srv/APP/current
  echo "release RELEASE is healthy and current"
else
  echo "release RELEASE failed:"; tail -5 /tmp/APP-up.log
  sudo docker compose -p APP ps --format '{{.Service}} {{.State}} {{.Health}}'
  for c in $(sudo docker compose -p APP ps -aq); do sudo docker logs --tail 30 "$c" 2>&1; done
  exit 1
fi
EOF
```

The logs are read with `docker logs` per container because this Compose
version puts a terminal escape code in front of every `compose logs` line.

`up --wait` returns an error when a container does not become healthy. A first
build pulls images and can outlast the wait: the job is then `running`, which
is fine. Follow it with `vibe-ssh job wait <cursor>`; never start it twice.

Running this again with nothing changed leaves the containers untouched.

## 4. Verify

```bash
vibe-ssh run --lease <lease> -- 'curl -fsS http://127.0.0.1:PORT/ | head -c 300'
```

Check a response that only the new version gives, then the public URL from
your own machine.

## 5. When the new release is unhealthy

`up` has already replaced the old containers: the broken version is what is
running now, and the previous one is not coming back by itself. `current`
still points at the last release that was healthy, so bring that one back:

```bash
vibe-ssh run --lease <lease> --wait 120 -- 'cd "$(readlink -f /srv/APP/current)" && sudo docker compose --ansi never -p APP up -d --build --wait && sudo docker compose -p APP ps --format "{{.Service}} {{.State}} {{.Health}}"'
```

Then verify again and tell the user: what failed, the log lines that show why,
and that the previous release is serving again. Leave the failed release
directory in place for inspection.

This restores the application version only. Database migrations and data in
volumes are not rolled back by it; if the new release migrated a database, say
so and let the user decide.

## Rules of thumb

- `docker compose down` stops the service and `down -v` deletes its data.
  Neither is part of a deploy; run them only when the user asked.
- Never `docker system prune` on a server that hosts other things.
- Long-running log tails (`logs -f`) end when you close the space.

[vibe-ssh:skill] end of guide
