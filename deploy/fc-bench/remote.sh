#!/usr/bin/env bash
# Выполняется на хосте замера под root из art/ (собран build.sh): ставит Firecracker, поднимает tap0
# и N раз грузит каждого гостя. Выход: results.txt — "<система> <готов, с> <выключение, с>".
# Готов: rim и alpine — sshd прислал баннер; lockbox — gRPC-сервер ответил на HTTP/2-рукопожатие.
set -euo pipefail
cd "$(dirname "$0")"
N="${1:-20}"
[ -e /dev/kvm ] || { echo "нет /dev/kvm — вложенная виртуализация не включена"; exit 1; }

if ! command -v firecracker >/dev/null; then
  VER=$(curl -fsSLI -o /dev/null -w '%{url_effective}' https://github.com/firecracker-microvm/firecracker/releases/latest | sed 's|.*/||')
  curl -fsSL "https://github.com/firecracker-microvm/firecracker/releases/download/$VER/firecracker-$VER-x86_64.tgz" | tar xz
  install "release-$VER-x86_64/firecracker-$VER-x86_64" /usr/local/bin/firecracker
fi
firecracker --version | head -1 > firecracker.version
ip link show tap0 >/dev/null 2>&1 || {
  ip tuntap add tap0 mode tap
  ip addr add 172.16.0.1/30 dev tap0
  ip link set tap0 up
}
# MAC гостя фиксирован (guest_mac), соседа на хосте прописываем заранее: иначе первые ARP-запросы
# уходят, пока у гостя ещё нет сети, запись соседа становится FAILED, и хост повторяет ARP только
# через retrans_time (1 с) — все замеры округлялись бы до ~1 с.
GUEST_MAC=06:00:ac:10:00:02
ip neigh replace 172.16.0.2 lladdr "$GUEST_MAC" dev tap0 nud permanent

now() { python3 -c 'import time; print(f"{time.time():.3f}")'; }
# fcboot.py boot <ssh|grpc> <лог> — засекает время, запускает firecracker (run.json) и опрашивает
# гостя каждые 5 мс в том же процессе: печатает "<pid> <готов, с>" или "<pid> FAIL".
# fcboot.py wait <ssh|grpc> — только ждёт готовности (до 30 с).
# ssh готов — sshd прислал баннер; grpc — сервер ответил на HTTP/2-преамбулу своим SETTINGS.
cat > fcboot.py <<'EOF'
import socket, subprocess, sys, time

def ready(kind):
    port = 22 if kind == "ssh" else 50051
    try:
        s = socket.create_connection(("172.16.0.2", port), 0.05)
        s.settimeout(0.5)
        if kind == "ssh":
            return s.recv(4) == b"SSH-"
        s.sendall(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n" + b"\x00\x00\x00\x04\x00\x00\x00\x00\x00")
        h = s.recv(9)
        return len(h) == 9 and h[3] == 4
    except OSError:
        return False

mode, kind = sys.argv[1:3]
t0 = time.time()
fc = None
if mode == "boot":
    fc = subprocess.Popen(["firecracker", "--no-api", "--config-file", "run.json"],
                          stdout=open(sys.argv[3], "w"), stderr=subprocess.STDOUT, start_new_session=True)
while time.time() - t0 < 30 and (fc is None or fc.poll() is None):
    if ready(kind):
        if fc:
            print(fc.pid, f"{time.time() - t0:.3f}")
        sys.exit(0)
    time.sleep(0.005)
if fc:
    print(fc.pid, "FAIL")
sys.exit(1)
EOF
gsh() { ssh -i key -o BatchMode=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  -o LogLevel=ERROR -o ConnectTimeout=5 root@172.16.0.2 "$@"; }

# config <ядро> <диск> <cmdline> [initrd] — та же форма, что rim-fc.firecracker.json
config() {
  python3 - "$@" <<'EOF'
import json, sys
kernel, disk, args = sys.argv[1:4]
boot = {"kernel_image_path": kernel, "boot_args": args}
if len(sys.argv) > 4:
    boot["initrd_path"] = sys.argv[4]
print(json.dumps({
    "boot-source": boot,
    "drives": [{"drive_id": "disk", "path_on_host": disk, "is_root_device": False, "is_read_only": False}],
    "network-interfaces": [{"iface_id": "eth0", "host_dev_name": "tap0", "guest_mac": "06:00:ac:10:00:02"}],
    "machine-config": {"vcpu_count": 1, "mem_size_mib": 256},
}))
EOF
}

args() { python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["boot-source"]["boot_args"])' "$1"; }
RIM_ARGS="$(args rim-fc.firecracker.json)"
LBX_ARGS="$(args lockbox-fc.firecracker.json)"
ALP_ARGS="console=ttyS0 reboot=k panic=10 root=/dev/vda rootfstype=ext4 modules=virtio_blk,ext4"

run() {  # run <система> <образ> → "<до ssh> <выключение>" или FAIL
  local sys="$1" img="$2"
  case "$sys" in
    rim)     config rim-fc.vmlinux "$img" "$RIM_ARGS" > run.json ;;
    lockbox) config lockbox-fc.vmlinux "$img" "$LBX_ARGS" > run.json ;;
    alpine)  config alpine.vmlinuz "$img" "$ALP_ARGS" alpine.initramfs > run.json ;;
  esac
  local pid tb ts ready=ssh
  [ "$sys" = lockbox ] && ready=grpc
  read -r pid tb < <(python3 fcboot.py boot "$ready" "$sys.console.log" || true)
  # ssh нужен и lockbox-гостю: через него статистика и выключение.
  if [ "${tb:-FAIL}" = FAIL ] || ! python3 fcboot.py wait ssh || ! gsh true; then
    kill "$pid" 2>/dev/null || true; sleep 0.2; echo FAIL; return
  fi
  [ -n "${STATS:-}" ] && gsh "$STATS" > "$sys.stats"
  ts=$(now)
  gsh reboot >/dev/null 2>&1 || true   # на x86 только сброс (reboot=k) завершает firecracker
  for _ in $(seq 1 1500); do kill -0 "$pid" 2>/dev/null || break; sleep 0.02; done
  if kill -0 "$pid" 2>/dev/null; then kill "$pid"; sleep 0.2; echo "$tb HUNG"; return; fi
  python3 -c "print(f'$tb {$(now)-$ts:.3f}')"
}

: > results.txt
for sys in rim lockbox alpine; do
  case "$sys" in alpine) base=alpine.img ;; *) base="$sys-fc.img" ;; esac
  run "$sys" "$base" >/dev/null   # прогрев: первая загрузка (rim создаёт ключ хоста)
  STATS='awk "/MemTotal/{t=\$2}/MemAvailable/{a=\$2}END{print \"ram_used_mib\",int((t-a)/1024)}" /proc/meminfo; df -k / | awk "NR==2{print \"root_used_kib\",\$3}"; uname -r; p=$(pidof lockbox-node) && awk "/VmRSS/{print \"lockbox_rss_mib\",int(\$2/1024)}" /proc/$p/status; echo procs $(ls -d /proc/[0-9]* | wc -l)'
  run "$sys" "$base" >/dev/null; unset STATS
  for i in $(seq 1 "$N"); do
    cp "$base" run.img
    echo "$sys $(run "$sys" run.img)" | tee -a results.txt
  done
done
