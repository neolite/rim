set -eu
cd "$SRC"
./configure --enable-static --disable-zlib --disable-syslog --disable-lastlog \
  --disable-utmp --disable-utmpx --disable-wtmp --disable-wtmpx \
  --disable-pututline --disable-pututxline >/dev/null
make -s -j"$JOBS" PROGRAMS="dropbear dropbearkey"
mkdir -p "$OUT/bin"
install -m 0755 dropbear dropbearkey "$OUT/bin/"
