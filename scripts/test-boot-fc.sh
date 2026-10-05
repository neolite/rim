#!/usr/bin/env bash
# Таргет fc (Firecracker) в QEMU, настроенном как Firecracker: device tree без ACPI и UEFI,
# ядро и cmdline снаружи (те же, что в firecracker.json), virtio-mmio, сеть без DHCP.
# ARCH=x86_64 — x86-гость в QEMU -M microvm (на Apple Silicon без ускорения: медленно, время не показательно).
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ARCH="${ARCH:-aarch64}"
case "$ARCH" in
  aarch64) TGT=fc;     OUTREL=out/fc-test;        KERN=rim-fc.Image;   TRIES=150 ;;
  x86_64)  TGT=fc-x86; OUTREL=out/fc-test-x86_64; KERN=rim-fc.vmlinux; TRIES=1500 ;;
  *) echo "ARCH: aarch64 или x86_64"; exit 1 ;;
esac
OUT="$ROOT/$OUTREL"
. "$ROOT/scripts/lib-testkey.sh"
new_test_key
MANIFEST="$(test_manifest tests/rim.fc.test.toml)" || { echo "FAIL тестовый ключ"; exit 1; }
"$ROOT/rim" --arch "$ARCH" build --manifest "$MANIFEST" --target "$TGT" --out "$OUTREL/rim-fc.img" >/dev/null || { echo "FAIL сборка"; exit 1; }

WORK="$(mktemp -d)"
QPID=""
trap 'kill "$QPID" 2>/dev/null; rm -rf "$WORK"' EXIT
KEY="$TEST_KEY"
LOG="$WORK/serial.log"; : > "$LOG"
PORT=2223
FAILED=0
check() { local name="$1"; shift; if "$@" >/dev/null 2>&1; then echo "ok   $name"; else echo "FAIL $name"; FAILED=1; fi; }
now() { perl -MTime::HiRes=time -e 'printf "%.2f", time'; }

# --- Артефакты ---
JSON="$OUT/rim-fc.firecracker.json"
json_get() { python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print(eval(sys.argv[2]))' "$JSON" "$1"; }
json_refs_ok() {
  [ "$(json_get 'd["boot-source"]["kernel_image_path"]')" = "$KERN" ] &&
  [ "$(json_get 'd["drives"][0]["path_on_host"]')" = rim-fc.img ] &&
  [ -s "$OUT/$KERN" ] && [ -s "$OUT/rim-fc.img" ]
}
three_partitions() { [ "$(python3 - "$OUT/rim-fc.img" <<'EOF'
import sys, struct
f = open(sys.argv[1], 'rb'); f.seek(1024)
n = sum(1 for _ in range(128) if f.read(128)[:16] != bytes(16))
print(n)
EOF
)" = 3 ]; }
check "firecracker.json валиден и ссылается на собранные файлы" json_refs_ok
check "в образе 3 раздела (без ESP)"                          three_partitions
BOOT_ARGS="$(json_get 'd["boot-source"]["boot_args"]')"

cp "$OUT/rim-fc.img" "$WORK/disk.img"
T0="$(now)"
if [ "$ARCH" = aarch64 ]; then
  VM=(qemu-system-aarch64 -M virt,acpi=off -accel hvf -cpu host)
else
  # microvm: virtio-mmio без PCI, как Firecracker; i8042 — им Firecracker ловит reboot=k;
  # -no-reboot: сброс завершает QEMU, как процесс firecracker.
  # acpi=off (MPTable): с ACPI под TCG ядру не на чем откалибровать TSC (kvmclock нет, PIT выключен,
  # а pit=on сбрасывает гостя). Путь ACPI Firecracker >= 1.8 проверяется только на настоящем хосте.
  VM=(qemu-system-x86_64 -M microvm,acpi=off,rtc=on -accel tcg -cpu max -no-reboot -device i8042)
fi
"${VM[@]}" -smp 1 -m 256M \
  -kernel "$OUT/$KERN" -append "$BOOT_ARGS" \
  -drive file="$WORK/disk.img",if=none,id=d0,format=raw -device virtio-blk-device,drive=d0 \
  -netdev user,id=n0,hostfwd=tcp:127.0.0.1:$PORT-:22 -device virtio-net-device,netdev=n0 \
  -display none -serial file:"$LOG" &
QPID=$!

rsh() {
  ssh -i "$KEY" -p $PORT -o ConnectTimeout=5 -o BatchMode=yes \
    -o UserKnownHostsFile="$WORK/known_hosts" -o StrictHostKeyChecking=accept-new root@127.0.0.1 "$@"
}
T_READY=""; T_SSH=""
wait_ready() { for _ in $(seq 1 "$TRIES"); do grep -q '^rim: ready' "$LOG" && { T_READY="$(now)"; return 0; }; sleep 0.2; done; return 1; }
wait_ssh() { for _ in $(seq 1 $((TRIES / 4))); do rsh true && { T_SSH="$(now)"; return 0; }; sleep 0.5; done; return 1; }
root_is_readonly() { rsh 'touch /x 2>&1; true' | grep -q 'Read-only'; }
hostname_ok() { [ "$(rsh hostname)" = rim-fc-test ]; }
ip_from_kernel() { rsh 'ip -4 addr show eth0' | grep -q 'inet 10.0.2.15/24'; }
no_dhcp_client() { ! rsh 'pidof udhcpc'; }
resolv_ok() { rsh 'cat /etc/resolv.conf' | grep -qx 'nameserver 10.0.2.3'; }
# 8250 у QEMU virt нет, но драйвер с OF-привязкой должен быть встроен — его консоль ttyS0 у Firecracker.
of_serial_built_in() { rsh 'test -d /sys/bus/platform/drivers/of_serial'; }
no_such_driver() { rsh 'test -d /sys/bus/platform/drivers/no_such_driver'; }
clock_is_current() { [ "$(rsh 'date +%Y')" -ge 2026 ]; }
qemu_exits() { for _ in $(seq 1 $((TRIES / 5))); do kill -0 "$QPID" 2>/dev/null || return 0; sleep 1; done; return 1; }
data_unmounts() { grep -q 'EXT4-fs (vda3): unmounting filesystem' "$LOG"; }
# Выход VM при панике тоже «завершил VM» — требуем штатную строку ядра.
clean_exit() { case "$ARCH" in aarch64) grep -q 'reboot: Power down' "$LOG" ;; *) grep -q 'reboot: Restarting system' "$LOG" ;; esac; }
poweroff_flag() { rsh 'cat /proc/cmdline' | grep -qw 'rim.poweroff=reboot'; }

check "rim: ready"                            wait_ready
check "ssh по ключу"                          wait_ssh
check "корень только для чтения"              root_is_readonly
check "hostname из манифеста"                 hostname_ok
check "eth0 = 10.0.2.15/24 (ip= ядра)"        ip_from_kernel
check "DHCP-клиента нет"                      no_dhcp_client
check "resolv.conf из /proc/net/pnp"          resolv_ok
check "часы из RTC, не 1970"                  clock_is_current
if [ "$ARCH" = aarch64 ]; then
  check "драйвер 8250/OF (ttyS0) встроен"     of_serial_built_in
  no_such_driver && { echo "FAIL контрольная проверка драйвера ложно-зелёная"; FAILED=1; }
  poweroff_flag && { echo "FAIL rim.poweroff=reboot не должен быть на aarch64"; FAILED=1; }
else
  check "rim.poweroff=reboot в cmdline"       poweroff_flag
fi
rsh 'cd /data/root && exec sleep 600' >/dev/null 2>&1 & sleep 1
rsh poweroff >/dev/null 2>&1
check "poweroff завершает VM"                 qemu_exits
check "poweroff при открытой сессии: /data отмонтирован" data_unmounts
check "VM завершилась штатно, не паникой"     clean_exit

[ -n "$T_READY" ] && printf 'время: до rim: ready %.2f с, до ssh %.2f с (QEMU %s, не Firecracker)\n' \
  "$(echo "$T_READY - $T0" | bc)" "$(echo "${T_SSH:-$T_READY} - $T0" | bc)" "$ARCH"

if [ $FAILED -ne 0 ]; then
  echo "--- serial.log (хвост) ---"; tail -40 "$LOG"
fi
exit $FAILED
