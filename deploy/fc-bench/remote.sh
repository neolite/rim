#!/usr/bin/env bash
# Выполняется на хосте замера под root (его запускает scripts/fc-bench.sh): ставит Firecracker,
# поднимает tap0 и N раз грузит каждого гостя. Выход: results.txt — "<система> <до ssh, с> <выключение, с>".
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

now() { python3 -c 'import time; print(f"{time.time():.3f}")'; }
banner() { python3 - <<'EOF'
import socket, sys
try:
    s = socket.create_connection(("172.16.0.2", 22), 0.2); s.settimeout(0.3)
    sys.exit(0 if s.recv(4) == b"SSH-" else 1)
except Exception:
    sys.exit(1)
EOF
}
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
    "network-interfaces": [{"iface_id": "eth0", "host_dev_name": "tap0"}],
    "machine-config": {"vcpu_count": 1, "mem_size_mib": 256},
}))
EOF
}

RIM_ARGS="$(python3 -c 'import json; print(json.load(open("rim-fc.firecracker.json"))["boot-source"]["boot_args"])')"
ALP_ARGS="console=ttyS0 reboot=k panic=10 root=/dev/vda rootfstype=ext4 modules=virtio_blk,ext4"

run() {  # run <система> <образ> → "<до ssh> <выключение>" или FAIL
  local sys="$1" img="$2"
  case "$sys" in
    rim)    config rim-fc.vmlinux "$img" "$RIM_ARGS" > run.json ;;
    alpine) config alpine.vmlinuz "$img" "$ALP_ARGS" alpine.initramfs > run.json ;;
  esac
  local t0 tb="" ts
  t0=$(now)
  firecracker --no-api --config-file run.json > "$sys.console.log" 2>&1 &
  local fc=$!
  for _ in $(seq 1 600); do banner && { tb=$(now); break; }; sleep 0.05; done
  if [ -z "$tb" ] || ! gsh true; then kill "$fc" 2>/dev/null; wait "$fc" 2>/dev/null; echo FAIL; return; fi
  [ -n "${STATS:-}" ] && gsh "$STATS" > "$sys.stats"
  ts=$(now)
  gsh reboot >/dev/null 2>&1 || true   # на x86 только сброс (reboot=k) завершает firecracker
  for _ in $(seq 1 600); do kill -0 "$fc" 2>/dev/null || break; sleep 0.05; done
  if kill -0 "$fc" 2>/dev/null; then kill "$fc"; wait "$fc" 2>/dev/null; echo "$(python3 -c "print(f'{$tb-$t0:.3f}')") HUNG"; return; fi
  wait "$fc" 2>/dev/null || true
  python3 -c "print(f'{$tb-$t0:.3f} {$(now)-$ts:.3f}')"
}

: > results.txt
for sys in rim alpine; do
  base="$sys.img"
  run "$sys" "$base" >/dev/null   # прогрев: первая загрузка (rim создаёт ключ хоста)
  STATS='awk "/MemTotal/{t=\$2}/MemAvailable/{a=\$2}END{print \"ram_used_mib\",int((t-a)/1024)}" /proc/meminfo; df -k / | awk "NR==2{print \"root_used_kib\",\$3}"; uname -r'
  run "$sys" "$base" >/dev/null; unset STATS
  for i in $(seq 1 "$N"); do
    cp "$base" run.img
    echo "$sys $(run "$sys" run.img)" | tee -a results.txt
  done
done
