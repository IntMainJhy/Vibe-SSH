#!/bin/sh
# A local Debian VM that serves as a real Linux SSH server for verification and demos.
#
# Everything lives in target/vm. It never reads or writes ~/.ssh: the login keys are
# generated here, and the server's host key is generated here too and injected at first
# boot, so its fingerprint is known before the first connection is ever made.
#
#   scripts/linux-vm.sh up      start (first run downloads a ~320 MB Debian image)
#   scripts/linux-vm.sh hosts   print host entries for a Vibe-SSH hosts.json
#   scripts/linux-vm.sh ssh     debugging shell (system ssh, isolated from your SSH config)
#   scripts/linux-vm.sh password  the generated password of the `ops` user
#   scripts/linux-vm.sh down    stop; add --wipe to also delete the disk
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
VM="$ROOT/target/vm"
SSH_PORT=${VIBE_VM_SSH_PORT:-42022}
HTTP_PORT=${VIBE_VM_HTTP_PORT:-42080}
APP_PORT=${VIBE_VM_APP_PORT:-42081}
IMAGE_URL=https://cloud.debian.org/images/cloud/trixie/latest
IMAGE=debian-13-genericcloud-arm64.qcow2
FIRMWARE=${VIBE_VM_FIRMWARE:-/opt/homebrew/share/qemu/edk2-aarch64-code.fd}
# The password for the `ops` user (password login, sudo prompt) and the passphrase of
# id_encrypted. Generated per VM and kept next to its disk, never in the repository.
password() { cat "$VM/ops_password"; }

running() { [ -f "$VM/qemu.pid" ] && kill -0 "$(cat "$VM/qemu.pid")" 2>/dev/null; }
fingerprint() { ssh-keygen -lf "$VM/host_key.pub" | awk '{print $2}'; }
vm_ssh() {
    ssh -F /dev/null -o UserKnownHostsFile="$VM/known_hosts" -o StrictHostKeyChecking=yes \
        -o IdentitiesOnly=yes -o IdentityAgent=none -o ConnectTimeout=5 -o LogLevel=ERROR \
        -i "$VM/id_deploy" -p "$SSH_PORT" deploy@127.0.0.1 "$@"
}

up() {
    [ "$(uname -sm)" = "Darwin arm64" ] || { echo "needs an Apple Silicon Mac" >&2; exit 1; }
    command -v qemu-system-aarch64 >/dev/null || { echo "install QEMU first: brew install qemu" >&2; exit 1; }
    if running; then echo "already running"; return; fi
    mkdir -p "$VM"
    cd "$VM"
    if [ ! -f "$IMAGE" ]; then
        echo "downloading $IMAGE ..."
        curl -fsSL "$IMAGE_URL/SHA512SUMS" -o SHA512SUMS
        curl -fL --retry 3 -o "$IMAGE.part" "$IMAGE_URL/$IMAGE"
        want=$(awk -v f="$IMAGE" '$2 == f {print $1}' SHA512SUMS)
        [ "$want" = "$(shasum -a 512 "$IMAGE.part" | awk '{print $1}')" ] || { echo "checksum mismatch" >&2; exit 1; }
        mv "$IMAGE.part" "$IMAGE"
    fi
    if [ ! -f disk.qcow2 ]; then
        for key in host_key id_deploy; do
            rm -f "$key" "$key.pub"
            ssh-keygen -q -t ed25519 -N "" -C "vibe-ssh-vm-$key" -f "$key"
        done
        (umask 077 && openssl rand -hex 12 > ops_password)
        rm -f id_encrypted id_encrypted.pub
        ssh-keygen -q -t ed25519 -N "$(password)" -C vibe-ssh-vm-encrypted -f id_encrypted
        echo "[127.0.0.1]:$SSH_PORT $(cut -d' ' -f1,2 host_key.pub)" > known_hosts
        rm -rf seed && mkdir seed
        printf 'instance-id: vibe-ssh-vm\nlocal-hostname: vibe-vm\n' > seed/meta-data
        {
            echo "#cloud-config"
            echo "ssh_deletekeys: true"
            echo "ssh_genkeytypes: []"
            echo "ssh_keys:"
            echo "  ed25519_private: |"
            sed 's/^/    /' host_key
            echo "  ed25519_public: $(cat host_key.pub)"
            echo "ssh_pwauth: true"
            echo "users:"
            echo "  - name: deploy"
            echo "    shell: /bin/bash"
            echo "    sudo: 'ALL=(ALL) NOPASSWD:ALL'"
            echo "    ssh_authorized_keys:"
            echo "      - $(cat id_deploy.pub)"
            echo "      - $(cat id_encrypted.pub)"
            echo "  - name: ops"
            echo "    shell: /bin/bash"
            echo "    lock_passwd: false"
            echo "    sudo: 'ALL=(ALL) ALL'"
            echo "    ssh_authorized_keys:"
            echo "      - $(cat id_deploy.pub)"
            echo "chpasswd:"
            echo "  expire: false"
            echo "  users:"
            echo "    - {name: ops, password: $(password), type: text}"
        } > seed/user-data
        rm -f seed.iso
        hdiutil makehybrid -quiet -o seed.iso -iso -joliet -default-volume-name cidata seed
        qemu-img create -q -f qcow2 -F qcow2 -b "$IMAGE" disk.qcow2 16G
        rm -f vars.fd && dd if=/dev/zero of=vars.fd bs=1m count=64 2>/dev/null
    fi
    for port in "$SSH_PORT" "$HTTP_PORT" "$APP_PORT"; do
        if nc -z 127.0.0.1 "$port" 2>/dev/null; then
            echo "port $port is in use; set VIBE_VM_SSH_PORT / VIBE_VM_HTTP_PORT / VIBE_VM_APP_PORT" >&2; exit 1
        fi
    done
    qemu-system-aarch64 -machine virt -accel hvf -cpu host -smp 2 -m 2048 \
        -drive if=pflash,format=raw,readonly=on,file="$FIRMWARE" \
        -drive if=pflash,format=raw,file=vars.fd \
        -drive if=virtio,format=qcow2,file=disk.qcow2 \
        -drive if=virtio,format=raw,readonly=on,file=seed.iso \
        -netdev "user,id=n0,hostfwd=tcp:127.0.0.1:$SSH_PORT-:22,hostfwd=tcp:127.0.0.1:$HTTP_PORT-:80,hostfwd=tcp:127.0.0.1:$APP_PORT-:8080" \
        -device virtio-net-pci,netdev=n0 \
        -display none -serial file:console.log -daemonize -pidfile qemu.pid
    printf 'booting'
    for _ in $(seq 1 90); do
        if vm_ssh true 2>/dev/null; then
            vm_ssh 'cloud-init status --wait >/dev/null 2>&1 || true'
            echo " ready: ssh 127.0.0.1:$SSH_PORT; the server's ports 80 and 8080 are http://127.0.0.1:$HTTP_PORT and http://127.0.0.1:$APP_PORT; host key $(fingerprint)"
            return
        fi
        printf '.'
        sleep 2
    done
    echo " did not come up; see $VM/console.log" >&2
    exit 1
}

hosts() {
    fp=$(fingerprint)
    cat <<JSON
[
  {"id": "vm", "name": "本机 Linux 虚拟机", "host": "127.0.0.1", "port": $SSH_PORT, "username": "deploy",
   "identity_file": "$VM/id_deploy", "host_key_sha256": "$fp", "agent_access": true},
  {"id": "vm-encrypted", "name": "虚拟机（加密私钥）", "host": "127.0.0.1", "port": $SSH_PORT, "username": "deploy",
   "identity_file": "$VM/id_encrypted", "host_key_sha256": "$fp", "agent_access": true},
  {"id": "vm-ops", "name": "虚拟机（sudo 要密码）", "host": "127.0.0.1", "port": $SSH_PORT, "username": "ops",
   "identity_file": "$VM/id_deploy", "host_key_sha256": "$fp", "agent_access": true},
  {"id": "vm-password", "name": "虚拟机（密码登录）", "host": "127.0.0.1", "port": $SSH_PORT, "username": "ops",
   "identity_file": null, "host_key_sha256": "$fp", "agent_access": true}
]
JSON
}

down() {
    if running; then
        kill "$(cat "$VM/qemu.pid")"
        echo "stopped"
    else
        echo "not running"
    fi
    rm -f "$VM/qemu.pid"
    if [ "${1:-}" = "--wipe" ]; then
        rm -rf "$VM/disk.qcow2" "$VM/vars.fd" "$VM/seed" "$VM/seed.iso" "$VM/console.log" "$VM/ops_password"
        echo "disk wiped; the next 'up' starts from a clean server"
    fi
}

case "${1:-}" in
    up) up ;;
    hosts) hosts ;;
    ssh) shift; vm_ssh "$@" ;;
    down) shift; down "$@" ;;
    password) password ;;
    *) sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'; exit 2 ;;
esac
