# usage: check-config.sh <фрагмент> <.config>
# Падает, если опция из фрагмента не дожила до итогового .config (неудовлетворённая зависимость).
set -eu
missing=0
while IFS= read -r line; do
  case "$line" in
    CONFIG_*=*)
      if ! grep -qx "$line" "$2"; then
        echo "check-config: нет '$line' в итоговом .config" >&2
        missing=1
      fi
      ;;
  esac
done < "$1"
exit $missing
