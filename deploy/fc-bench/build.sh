#!/usr/bin/env bash
# Выполняется на хосте замера под root (его запускает scripts/fc-bench.sh): собирает нативно для
# x86_64 три гостя — rim, rim + lockbox-node, обычный Alpine — и кладёт всё для remote.sh в art/.
# Ожидает рядом rim/ и lockbox/ — распакованные исходники.
set -euo pipefail
B="$(cd "$(dirname "$0")" && pwd)"
A="$B/art"; R="$B/rim"
mkdir -p "$A"
command -v docker >/dev/null || { apt-get update -q >/dev/null; DEBIAN_FRONTEND=noninteractive apt-get install -yq docker.io >/dev/null; }

echo "== lockbox-node (CGO_ENABLED=0, статический)"
docker run --rm -v "$B/lockbox:/src:ro" -v "$A:/out" -w /src -e CGO_ENABLED=0 -e GOFLAGS=-buildvcs=false \
  -e GOTOOLCHAIN=local golang:1.24-alpine \
  go build -trimpath -ldflags="-s -w -buildid=" -o /out/lockbox-node ./cmd/node
file "$A/lockbox-node" | grep -q 'statically linked' || { file "$A/lockbox-node"; echo "lockbox-node не статический"; exit 1; }

# Пакет rim: архив с готовым бинарником, url — file:// внутри контейнера тулчейна (/rim = $R).
mkdir -p "$R/out/fc-bench/lockbox-node-bench" "$R/pkgs/lockbox"
cp "$A/lockbox-node" "$R/out/fc-bench/lockbox-node-bench/"
tar -C "$R/out/fc-bench" --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner -cf - lockbox-node-bench \
  | gzip -n > "$R/out/fc-bench/lockbox-src.tar.gz"
cp "$R/deploy/fc-bench/lockbox-pkg/build.sh" "$R/pkgs/lockbox/build.sh"
cat > "$R/pkgs/lockbox/recipe.toml" <<R
version = "bench"
url = "file:///rim/out/fc-bench/lockbox-src.tar.gz"
sha256 = "$(sha256sum "$R/out/fc-bench/lockbox-src.tar.gz" | cut -d' ' -f1)"
kind = "rootfs"
R

echo "== rim и rim + lockbox"
ROOT="$R"; . "$R/scripts/lib-testkey.sh"
new_test_key
cd "$R"
for g in rim lockbox; do
  m="$(test_manifest "deploy/fc-bench/$g.bench.toml")"
  ./rim --arch x86_64 build --manifest "$m" --target fc-x86 --out "out/fc-bench/$g-fc.img" >/dev/null
  cp "out/fc-bench/$g-fc.img" "out/fc-bench/$g-fc.vmlinux" "out/fc-bench/$g-fc.firecracker.json" "$A/"
done
cp "$TEST_KEY" "$A/key"; cp "$TEST_KEY.pub" "$A/key.pub"; chmod 600 "$A/key"

echo "== Alpine"
docker run --rm -v "$R/deploy/fc-bench:/src:ro" -v "$A:/in:ro" -v "$A:/out" alpine:3.24 sh /src/alpine.sh
cp "$R/deploy/fc-bench/remote.sh" "$A/"
ls -l "$A"
