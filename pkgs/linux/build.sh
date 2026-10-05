set -eu
cd "$SRC"
export ARCH=arm64
export KBUILD_BUILD_TIMESTAMP="@${SOURCE_DATE_EPOCH}" KBUILD_BUILD_USER=rim KBUILD_BUILD_HOST=rim
make -s allnoconfig KCONFIG_ALLCONFIG="$RECIPE/virt.config"
sh "$RECIPE/check-config.sh" "$RECIPE/virt.config" .config
make -s -j"$JOBS" Image
mkdir -p "$OUT/boot"
cp arch/arm64/boot/Image "$OUT/boot/Image"
cp .config "$OUT/boot/config"
