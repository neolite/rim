# Пакет rim из готового статического lockbox-node (собран build.sh замера в golang:1.24-alpine).
set -eu
mkdir -p "$OUT/bin"
install -m 0755 "$SRC/lockbox-node" "$OUT/bin/"
