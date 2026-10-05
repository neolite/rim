#!/usr/bin/env bash
# Замер rim и Alpine (x86_64) в настоящем Firecracker на хосте из deploy/fc-bench (Terraform).
#   scripts/fc-bench.sh <ip хоста> [N=20]
# Собирает оба образа локально, заливает на хост, гоняет deploy/fc-bench/remote.sh, забирает
# out/fc-bench/results.txt и печатает медианы. Хост создаётся и удаляется отдельно (terraform).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IP="${1:?использование: scripts/fc-bench.sh <ip> [N]}"
N="${2:-20}"
B="$ROOT/out/fc-bench"
mkdir -p "$B"
HOST_KEY="${RIM_SSH_KEY:-$HOME/.ssh/id_ed25519_rim}"
hsh() { ssh -i "$HOST_KEY" -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile="$B/known_hosts" "ubuntu@$IP" "$@"; }

echo "== образы"
. "$ROOT/scripts/lib-testkey.sh"
new_test_key
MANIFEST="$(test_manifest deploy/fc-bench/rim.bench.toml)"
"$ROOT/rim" --arch x86_64 build --manifest "$MANIFEST" --target fc-x86 --out out/fc-bench/rim-fc.img >/dev/null
cp "$TEST_KEY.pub" "$B/key.pub"
docker run --rm --platform linux/amd64 -v "$ROOT/deploy/fc-bench:/src:ro" -v "$B:/in:ro" -v "$B:/out" \
  alpine:3.24 sh /src/alpine.sh
echo "Alpine: $(tr '\n' ' ' < "$B/alpine.mmio-config")"

echo "== заливка на $IP"
hsh 'mkdir -p ~/b'
scp -q -i "$HOST_KEY" -o UserKnownHostsFile="$B/known_hosts" \
  "$ROOT/deploy/fc-bench/remote.sh" "$B/rim-fc.img" "$B/rim-fc.vmlinux" "$B/rim-fc.firecracker.json" \
  "$B/alpine.img" "$B/alpine.vmlinuz" "$B/alpine.initramfs" "ubuntu@$IP:b/"
scp -q -i "$HOST_KEY" -o UserKnownHostsFile="$B/known_hosts" "$TEST_KEY" "ubuntu@$IP:b/key"

echo "== замер (N=$N)"
hsh "chmod 600 ~/b/key && sudo bash ~/b/remote.sh $N"
scp -q -i "$HOST_KEY" -o UserKnownHostsFile="$B/known_hosts" \
  "ubuntu@$IP:b/{results.txt,rim.stats,alpine.stats,firecracker.version}" "$B/"

python3 - "$B" <<'EOF'
import statistics as st, sys, pathlib
b = pathlib.Path(sys.argv[1])
rows = [l.split() for l in (b / "results.txt").read_text().splitlines() if l.strip()]
print((b / "firecracker.version").read_text().strip())
for sys_ in ("rim", "alpine"):
    r = [x for x in rows if x[0] == sys_]
    ok = [x for x in r if x[1] != "FAIL"]
    boot = [float(x[1]) for x in ok]
    down = [float(x[2]) for x in ok if x[2] != "HUNG"]
    stats = (b / f"{sys_}.stats").read_text().split() if (b / f"{sys_}.stats").exists() else []
    if not boot:
        print(f"{sys_:7s} все {len(r)} прогонов FAIL"); continue
    print(f"{sys_:7s} до ssh: медиана {st.median(boot):.3f} с (мин {min(boot):.3f}, макс {max(boot):.3f})"
          f" | выключение: медиана {st.median(down):.3f} с" if down else " | выключение: HUNG",
          f"| ok {len(ok)}/{len(r)} | {' '.join(stats)}")
EOF
