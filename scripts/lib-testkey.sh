# Подключается из test-boot*.sh. Ключ для входа в тестовый образ создаётся заново на каждый
# прогон и в репозиторий не попадает: в tests/*.toml вместо него заглушка, с которой не войти.
# test_manifest <tests/x.toml> → печатает путь к копии манифеста с подставленным ключом;
# приватный ключ — в $TEST_KEY.
TEST_KEY_DIR="$ROOT/out/test-key"
TEST_KEY="$TEST_KEY_DIR/id_ed25519"
PLACEHOLDER='ssh-ed25519 TEST-KEY-PLACEHOLDER'

test_manifest() {
  local src="$1" dst="$TEST_KEY_DIR/$(basename "$1")"
  mkdir -p "$TEST_KEY_DIR"
  [ -f "$TEST_KEY" ] || ssh-keygen -q -t ed25519 -N '' -C rim-test -f "$TEST_KEY" || return 1
  grep -qF "\"$PLACEHOLDER\"" "$ROOT/$src" || { echo "в $src нет заглушки ключа" >&2; return 1; }
  local pub; pub="$(cut -d' ' -f1-2 "$TEST_KEY.pub")"
  python3 - "$ROOT/$src" "$dst" "$PLACEHOLDER" "$pub" <<'EOF'
import sys
src, dst, ph, pub = sys.argv[1:]
open(dst, 'w').write(open(src).read().replace(ph, pub))
EOF
  echo "out/test-key/$(basename "$1")"
}

new_test_key() { rm -f "$TEST_KEY" "$TEST_KEY.pub"; }
