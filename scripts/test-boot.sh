#!/usr/bin/env bash
# Грузит тестовый образ в QEMU (HVF) и проверяет систему снаружи, как пользователь VPS.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
"$ROOT/rim" build --manifest tests/rim.test.toml --out out/rim-test.img >/dev/null || { echo "FAIL сборка"; exit 1; }

WORK="$(mktemp -d)"
trap 'kill "$QPID" 2>/dev/null; rm -rf "$WORK"' EXIT
cp "$ROOT/out/rim-test.img" "$WORK/disk.img"
truncate -s 2G "$WORK/disk.img"         # диск VPS больше образа: резервная GPT не в конце
KEY="$WORK/key"; cp "$ROOT/tests/keys/id_ed25519" "$KEY"; chmod 600 "$KEY"
LOG="$WORK/serial.log"; : > "$LOG"
FW="$(brew --prefix qemu)/share/qemu/edk2-aarch64-code.fd"
PORT=2222
FAILED=0

qemu-system-aarch64 -M virt -accel hvf -cpu host -smp 2 -m 512M -bios "$FW" \
  -drive file="$WORK/disk.img",if=virtio,format=raw \
  -device virtio-net-pci,netdev=n0 -netdev user,id=n0,hostfwd=tcp:127.0.0.1:$PORT-:22 \
  -device virtio-rng-pci -display none -serial file:"$LOG" &
QPID=$!

# STRICT=accept-new при первом входе запоминает ключ хоста; STRICT=yes потом требует тот же ключ.
STRICT=accept-new
rsh() {
  ssh -i "$KEY" -p $PORT -o ConnectTimeout=5 -o BatchMode=yes \
    -o UserKnownHostsFile="$WORK/known_hosts" -o StrictHostKeyChecking=$STRICT root@127.0.0.1 "$@"
}
check() { local name="$1"; shift; if "$@" >/dev/null 2>&1; then echo "ok   $name"; else echo "FAIL $name"; FAILED=1; fi; }
ready_count() { grep -c '^rim: ready' "$LOG" || true; }
wait_ready() { for _ in $(seq 1 "$2"); do [ "$(ready_count)" -ge "$1" ] && return 0; sleep 1; done; return 1; }
wait_ssh() { for _ in $(seq 1 20); do rsh true && return 0; sleep 1; done; return 1; }
root_is_readonly() { rsh 'touch /x 2>&1; true' | grep -q 'Read-only'; }
hostname_ok() { [ "$(rsh hostname)" = rim-test ]; }
no_suid() { [ -z "$(rsh 'find / -xdev -perm -4000')" ]; }
password_refused() {
  ! ssh -p $PORT -o BatchMode=yes -o PreferredAuthentications=password -o PubkeyAuthentication=no \
      -o UserKnownHostsFile="$WORK/known_hosts" root@127.0.0.1 true
}
kill_sshd_listener() {
  rsh 'for p in $(pidof dropbear); do [ "$(cut -d" " -f4 /proc/$p/stat)" = 1 ] && kill $p; done; true'
}
fingerprint() { rsh 'dropbearkey -y -f /data/ssh/host_ed25519' | grep Fingerprint; }
qemu_exits() { for _ in $(seq 1 20); do kill -0 "$QPID" 2>/dev/null || return 0; sleep 1; done; return 1; }

check "rim: ready за 30 с"                  wait_ready 1 30
check "ssh по ключу"                        wait_ssh
check "корень только для чтения"            root_is_readonly
check "hostname из манифеста"               hostname_ok
check "SUID-бинарников нет"                 no_suid
check "пароль не принимается"               password_refused

FP1="$(fingerprint)"
kill_sshd_listener >/dev/null 2>&1
sleep 3
check "упавший sshd перезапущен"            wait_ssh

rsh reboot >/dev/null 2>&1
STRICT=yes                                  # дальше ssh требует прежний ключ хоста
check "после reboot снова ready"            wait_ready 2 60
check "ssh после reboot с прежним ключом"   wait_ssh
FP2="$(fingerprint)"
check "отпечаток ключа хоста не изменился"  test -n "$FP1" -a "$FP1" = "$FP2"

rsh poweroff >/dev/null 2>&1
check "poweroff завершает QEMU"             qemu_exits

if [ $FAILED -ne 0 ]; then
  echo "--- serial.log (хвост) ---"; tail -40 "$LOG"
fi
exit $FAILED
