# rim: архитектура x86_64 (amd64) для таргета Firecracker — дизайн

Дата: 2026-10-05. Статус: решения приняты исполнителем по поручению пользователя («делай»).

## Цель

`rim` собирает гостя Firecracker и для x86_64: тот же манифест, тот же init, тот же GPT-диск.
Зачем: Firecracker на x86 работает внутри обычной облачной VM с вложенной виртуализацией
(без bare metal), и большинство хостов Firecracker — x86.

Вне этой итерации: `boot = "uefi-uki"` на x86_64 (VPS на x86 часто грузятся через BIOS —
отдельная задача), кросс-компиляция без эмуляции.

## Факты Firecracker x86_64 (FAQ, rootfs-and-kernel-setup.md)

- Ядро — несжатый ELF `vmlinux` (bzImage — только для совместимости, медленнее).
- `poweroff` гостя процесс firecracker **не** завершает: у гостя нет способа попросить
  выключение. Завершает его перезагрузка с `reboot=k` (сброс через i8042, его перехватывает
  Firecracker).
- Устройства virtio-mmio Firecracker сообщает ядру через `virtio_mmio.device=` в cmdline →
  нужен `VIRTIO_MMIO_CMDLINE_DEVICES`. Консоль — 8250 на 0x3f8 (`ttyS0`). RTC нет, время —
  kvmclock (`KVM_GUEST`).

## Манифест

```toml
[target.fc-x86]
boot = "direct"
arch = "x86_64"          # по умолчанию "aarch64"
kernel = "linux"
disk = { root = "64M", data = "256M" }
net = { ... }
```

`arch` — `aarch64` | `x86_64`. Валидация: `x86_64` только с `boot = "direct"`.

## Сборка

- `./rim --arch x86_64 <команда>`: тот же `docker/Dockerfile`, собранный под `linux/amd64`
  (на Apple Silicon — Rosetta через OrbStack/Docker Desktop), образ `rim-toolchain-x86_64`,
  отдельный том cargo и `CARGO_TARGET_DIR=out/cargo-linux-<arch>`. `CARGO_BUILD_TARGET` и
  `CARGO_TARGET_DIR` задаются в Dockerfile через `ARG RIM_ARCH`/`RIM_CARGO_DIR` (после `RUN`,
  значения по умолчанию прежние — id тулчейна arm64 не меняется).
- `rim-build build` сверяет `target.arch` с архитектурой, под которой запущен
  (`std::env::consts::ARCH`); при несовпадении — ошибка с подсказкой `./rim --arch …`.
- Кэш пакетов уже разделён: в ключ входит id тулчейна, а он у двух платформ разный.
- `pkgs/linux/build.sh` ветвится по `uname -m`: x86_64 → `ARCH=x86_64`, фрагмент
  `x86_64.config`, `make vmlinux`, результат `$OUT/boot/vmlinux`. busybox и dropbear не меняются.
- GPT: тип корневого раздела по архитектуре (x86-64 root `4f68bce3-e8cd-4db1-96e7-fbcaf984b709`).
- Артефакты direct на x86: `<stem>.img`, `<stem>.vmlinux`, `<stem>.firecracker.json`.
- cmdline direct x86_64: `root=PARTUUID=… rootfstype=erofs ro rootwait console=ttyS0 reboot=k
  panic=10 rim.poweroff=reboot [ip=…]`.

## init

`rim.poweroff=reboot` в `/proc/cmdline` → запрошенное выключение выполняется как перезагрузка
(RB_AUTOBOOT). Чистая функция `setup::poweroff_as_reboot(cmdline) -> bool`. Последовательность
остановки сервисов и отмонтирования `/data` не меняется.

## Ядро x86_64 (`pkgs/linux/x86_64.config`)

allnoconfig + 64BIT, SMP, HYPERVISOR_GUEST, PARAVIRT, KVM_GUEST, PVH (чтобы QEMU грузил
`vmlinux`), ACPI (Firecracker >= 1.8 описывает устройства в ACPI, MPTable устарел и уйдёт в 2.0),
X86_MPPARSE (для старых версий), virtio-mmio с cmdline-устройствами, virtio-blk/net, 8250-консоль,
RTC_DRV_CMOS + HCTOSYS (для QEMU), IP_PNP, erofs, ext4, GPT, devtmpfs, tmpfs, seccomp/landlock —
общая часть как у arm64. Без PCI, EFI, VT, USB, DRM. `check-config.sh` как для arm64.

## Проверка

- Юнит: манифест (`arch` по умолчанию aarch64; x86_64 + uefi-uki — ошибка), layout (тип root
  по arch), bootargs (точная cmdline x86), `poweroff_as_reboot`.
- `ARCH=x86_64 scripts/test-boot-fc.sh`: `qemu-system-x86_64 -M microvm,acpi=off,rtc=on -no-reboot -device i8042`
  (путь MPTable; путь ACPI под TCG не грузится — нет источника калибровки TSC — и проверяется
  только на настоящем Firecracker)
  (TCG — без ускорения на Apple Silicon, таймауты больше, время не показательно), те же проверки,
  кроме драйвера OF-8250; `poweroff` → VM завершилась.
- Детерминизм x86 fc (cmp трёх артефактов); регрессия: arm64 `test-boot-fc.sh` 13/13,
  `test-boot.sh` 14/14, `cargo test`.

## Firecracker на x86

`docs/firecracker.md` дополняется x86: хост — Linux x86_64 с `/dev/kvm`, в т.ч. облачная VM с
вложенной виртуализацией; Terraform-прогон с замером — отдельный шаг, платный, только после
явного согласия пользователя и выбора аккаунта.
