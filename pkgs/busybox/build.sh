set -eu
cd "$SRC"
make -s defconfig
# Статика; без своего init (PID 1 — rim-init); без tc (не собирается с новыми заголовками ядра);
# без SHA HWACCEL: в 1.37 код для x86 (sha*_shaNI) ломает сборку на arm64.
sed -i \
  -e 's/^# CONFIG_STATIC is not set/CONFIG_STATIC=y/' \
  -e 's/^CONFIG_INIT=y/# CONFIG_INIT is not set/' \
  -e 's/^CONFIG_LINUXRC=y/# CONFIG_LINUXRC is not set/' \
  -e 's/^CONFIG_TC=y/# CONFIG_TC is not set/' \
  -e 's/^CONFIG_SHA1_HWACCEL=y/# CONFIG_SHA1_HWACCEL is not set/' \
  -e 's/^CONFIG_SHA256_HWACCEL=y/# CONFIG_SHA256_HWACCEL is not set/' \
  .config
make -s oldconfig </dev/null >/dev/null
make -s -j"$JOBS"
make -s CONFIG_PREFIX="$OUT" install >/dev/null
rm -f "$OUT/linuxrc" "$OUT/sbin/init"
