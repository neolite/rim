#!/usr/bin/env bash
# Замер rim, rim + lockbox-node и Alpine (x86_64) в настоящем Firecracker на хосте из
# deploy/fc-bench (Terraform). Сборка и замер идут на хосте — нативно для x86_64, локально нужен
# только git: сюда уходят исходники (git archive HEAD), обратно — результаты.
#   scripts/fc-bench.sh <ip хоста> [N=20]
# LOCKBOX_DIR — исходники lockbox (по умолчанию ~/Apps/lockbox), берётся их HEAD.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IP="${1:?использование: scripts/fc-bench.sh <ip> [N]}"
N="${2:-20}"
LOCKBOX_DIR="${LOCKBOX_DIR:-$HOME/Apps/lockbox}"
B="$ROOT/out/fc-bench"
mkdir -p "$B"
HOST_KEY="${RIM_SSH_KEY:-$HOME/.ssh/id_ed25519_rim}"
SSH_OPTS=(-i "$HOST_KEY" -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile="$B/known_hosts")
hsh() { ssh "${SSH_OPTS[@]}" "ubuntu@$IP" "$@"; }

for d in "$ROOT" "$LOCKBOX_DIR"; do
  [ -z "$(git -C "$d" status --porcelain --untracked-files=no)" ] \
    || echo "ВНИМАНИЕ: в $d незакоммиченные правки — на хост уйдёт HEAD без них" >&2
done
echo "== исходники: rim $(git -C "$ROOT" rev-parse --short HEAD), lockbox $(git -C "$LOCKBOX_DIR" rev-parse --short HEAD)"
git -C "$ROOT" archive --format=tar.gz -o "$B/rim-src.tar.gz" HEAD
git -C "$LOCKBOX_DIR" archive --format=tar.gz -o "$B/lockbox-src.tar.gz" HEAD

echo "== заливка на $IP"
hsh 'rm -rf b && mkdir -p b/rim b/lockbox'
scp -q "${SSH_OPTS[@]}" "$B/rim-src.tar.gz" "$B/lockbox-src.tar.gz" "ubuntu@$IP:b/"
hsh 'tar -xzf b/rim-src.tar.gz -C b/rim && tar -xzf b/lockbox-src.tar.gz -C b/lockbox'

echo "== сборка на хосте"
hsh 'sudo bash b/rim/deploy/fc-bench/build.sh' 2>&1 | tee "$B/build.log" | grep -E '^==|error|ошибк' || true
hsh 'test -f b/art/remote.sh' || { echo "сборка не удалась, лог: $B/build.log"; exit 1; }

echo "== замер (N=$N)"
hsh "sudo bash b/art/remote.sh $N"
scp -q "${SSH_OPTS[@]}" \
  "ubuntu@$IP:b/art/{results.txt,rim.stats,lockbox.stats,alpine.stats,firecracker.version,alpine.mmio-config}" "$B/"
for s in rim lockbox alpine; do scp -q "${SSH_OPTS[@]}" "ubuntu@$IP:b/art/$s.console.log" "$B/" || true; done

python3 - "$B" <<'EOF'
import statistics as st, sys, pathlib
b = pathlib.Path(sys.argv[1])
rows = [l.split() for l in (b / "results.txt").read_text().splitlines() if l.strip()]
print((b / "firecracker.version").read_text().strip())
for sys_ in ("rim", "lockbox", "alpine"):
    r = [x for x in rows if x[0] == sys_]
    ok = [x for x in r if x[1] != "FAIL"]
    boot = [float(x[1]) for x in ok]
    down = [float(x[2]) for x in ok if x[2] != "HUNG"]
    f = b / f"{sys_}.stats"
    stats = " ".join(f.read_text().split()) if f.exists() else ""
    if not boot:
        print(f"{sys_:8s} все {len(r)} прогонов FAIL"); continue
    ready = "до gRPC" if sys_ == "lockbox" else "до ssh"
    dn = f"медиана {st.median(down):.3f} с" if down else "HUNG"
    print(f"{sys_:8s} {ready}: медиана {st.median(boot):.3f} с (мин {min(boot):.3f}, макс {max(boot):.3f})"
          f" | выключение: {dn} | ok {len(ok)}/{len(r)} | {stats}")
EOF
