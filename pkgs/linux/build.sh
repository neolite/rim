set -eu
cd "$SRC"
export KBUILD_BUILD_TIMESTAMP="@${SOURCE_DATE_EPOCH}" KBUILD_BUILD_USER=rim KBUILD_BUILD_HOST=rim
# Архитектура — та, под которой работает тулчейн (./rim --arch ...).
case "$(uname -m)" in
  aarch64) export ARCH=arm64;  FRAG="$RECIPE/virt.config";   TARGET=Image;   BUILT=arch/arm64/boot/Image ;;
  x86_64)  export ARCH=x86_64; FRAG="$RECIPE/x86_64.config"; TARGET=vmlinux; BUILT=vmlinux ;;
  *) echo "linux: неизвестная архитектура $(uname -m)" >&2; exit 1 ;;
esac
make -s allnoconfig KCONFIG_ALLCONFIG="$FRAG"
sh "$RECIPE/check-config.sh" "$FRAG" .config
make -s -j"$JOBS" "$TARGET"
mkdir -p "$OUT/boot"
cp "$BUILT" "$OUT/boot/$TARGET"
cp .config "$OUT/boot/config"
