# rim

Минимальная неизменяемая ОС для aarch64 (и x86_64 — гостем Firecracker): ядро Linux, свой PID 1 на Rust, busybox и dropbear.
Вся система описывается одним файлом `rim.toml`, из которого собирается бит-в-бит
воспроизводимый образ диска.

> **0.0.1-alpha** — эксперимент/витрина. Грузится в QEMU и проверяется тестом;
> на реальном VPS пока не проверялась.

## Что внутри

- **Образ:** GPT (`esp` / `root-A` / `root-B` / `data`), UKI (systemd-stub + ядро + cmdline),
  без initramfs; корень — erofs только для чтения, данные — ext4 на `/data`.
- **Сборка:** в Docker-контейнере, без root и loop-устройств; GPT пишется своим кодом.
  Все идентификаторы выводятся из хэша манифеста, время — из `SOURCE_DATE_EPOCH`.
- **init:** чистая машина состояний (`crates/rim-init/src/core.rs`), тестируемая без Linux:
  запуск сервисов, перезапуск с backoff, корректное выключение.
- **Ядро:** Linux 6.18 LTS, `allnoconfig` + фрагмент; сборка падает, если опция молча выпала.

## Быстрый старт

Нужны Docker (на macOS — OrbStack/Docker Desktop) и, для теста, QEMU (`brew install qemu`).

```sh
cp rim.example.toml rim.toml        # rim.toml в .gitignore: вставьте свой ssh-ключ
./rim build                         # rim.toml → out/rim.img
./scripts/test-boot.sh              # загрузка в QEMU и 14 проверок снаружи
cargo test --workspace              # юнит-тесты
```

Манифест:

```toml
[system]
hostname = "rim"
packages = ["busybox", "dropbear"]
ssh_authorized_keys = ["ssh-ed25519 AAAA..."]

[service.sshd]
exec = ["/bin/dropbear", "-F", "-s", "-g", "-p", "22", "-r", "/data/ssh/host_ed25519"]

[target.virt]
boot = "uefi-uki"
kernel = "linux"
disk = { esp = "128M", root = "256M", data = "64M" }
```

## Что можно запускать

Только статические бинарники aarch64 (Go с `CGO_ENABLED=0`, Rust `*-musl`, C с `-static`):
libc и пакетного менеджера в системе нет. Нет контейнеров (cgroups), IPv6, iptables, модулей ядра,
корневых сертификатов TLS.

## Требования к машине

arm64, UEFI, диск virtio-blk/virtio-scsi, сеть virtio-net с DHCP. Выкладка на Hetzner CAX —
[docs/deploy-hetzner.md](docs/deploy-hetzner.md).

Гостем microVM Firecracker (без UEFI, статическая сеть; aarch64 и x86_64 через `arch = "x86_64"`
и `./rim --arch x86_64`) — таргет `boot = "direct"`,
см. [docs/firecracker.md](docs/firecracker.md); локальная проверка — `./scripts/test-boot-fc.sh`.

Тесты создают ssh-ключ заново на каждый прогон (`out/test-key/`, вне git); в `tests/*.toml` вместо
ключа заглушка, так что собранный из них образ не пускает никого.

## Лицензия

MIT — см. [LICENSE](LICENSE).
