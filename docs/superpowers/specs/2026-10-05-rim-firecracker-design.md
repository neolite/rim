# rim: таргет Firecracker (MVP) — дизайн

Дата: 2026-10-05. Статус: утверждён по частям в чате.

## Цель

rim загружается гостем в microVM Firecracker на aarch64: поднимает сервисы из манифеста,
доступен по сети (ssh), корректно выключается. Агенты, vsock, lockbox, слой с libc —
вне MVP.

Требования Firecracker к гостю (docs/kernel-policy.md, docs/rootfs-and-kernel-setup.md):
несжатый `Image` (PE) на aarch64; device tree вместо ACPI; virtio-mmio; консоль 8250
через `SERIAL_OF_PLATFORM` (`ttyS0`); часы PL031 (`ARM_AMBA`, `RTC_DRV_PL031`);
Linux 6.18 — поддерживаемое гостевое ядро. DHCP Firecracker не даёт.

## Решение: тот же диск, прямая загрузка ядра

Firecracker подключает файл как блочное устройство, ядро разбирает на нём GPT.
Поэтому образ остаётся GPT-диском, `root=PARTUUID=` и поиск `/data` по PARTNAME
работают без изменений; меняется только способ загрузки.

## Манифест

```toml
[target.fc]
boot = "direct"
kernel = "linux"
disk = { root = "64M", data = "256M" }
net = { address = "172.16.0.2/30", gateway = "172.16.0.1", dns = "1.1.1.1" }
```

- `boot = "direct"` — новый вариант `Boot::Direct`.
- `disk.esp` становится необязательным: обязателен для `uefi-uki`, запрещён для `direct`
  (ошибка валидации называет поле).
- `net` — необязательный блок: `address` (IPv4/префикс 1..=32), `gateway` (IPv4),
  `dns` (IPv4, необязательно). Шлюз обязан лежать в подсети адреса и не совпадать с ним
  (поэтому /32 недопустим). Допустим только для `direct` (для `uefi-uki` сеть даёт DHCP).

## Ядро

Один пакет `linux` для обоих таргетов. В `pkgs/linux/virt.config` добавляются
`SERIAL_8250`, `SERIAL_8250_CONSOLE`, `SERIAL_OF_PLATFORM`, `IP_PNP`
(`ARM_AMBA` arm64 включает сам).
`check-config.sh` проверяет, что все они дожили до итогового `.config`.

## init

Если ядро настроило сеть само (`ip=`), DNS лежит в `/proc/net/pnp`. init при старте
переписывает строки `nameserver …` оттуда в `/run/resolv.conf`
(на него указывает `/etc/resolv.conf`). Логика — чистая функция в `setup.rs`
`resolv_from_pnp(&str) -> Option<String>`; `None`, если nameserver нет.
Остальное (сервисы, ключи, выключение) не меняется: `poweroff` через PSCI завершает
процесс firecracker.

## Сборщик

- `layout::plan`: без ESP разделы `root-A`, `root-B`, `data`; выравнивание 1 MiB сохраняется.
- Новый модуль `bootargs.rs` (чистые функции):
  - `cmdline(root_partuuid, net, console_extra) -> String`:
    `root=PARTUUID=<uuid> rootfstype=erofs ro rootwait console=ttyAMA0 console=ttyS0 panic=10`
    плюс `ip=<addr>::<gw>:<mask>::eth0:off[:<dns>]`, если задан `net`.
    Для `uefi-uki` cmdline прежний (консоли ttyAMA0 и tty0).
  - `firecracker_json(kernel_path, disk_path, boot_args) -> String`: boot-source, один диск
    (`drive_id = "disk"`, `is_root_device = false`, `is_read_only = false`), сеть
    `eth0` на `tap0`, `vcpu_count = 1`, `mem_size_mib = 256`. JSON пишется вручную.
- `image.rs`: ветка по `boot`. `uefi-uki` — как раньше. `direct` — без UKI и ESP;
  рядом с образом `<out без .img>.Image` (копия ядра) и `<out без .img>.firecracker.json`
  (пути в JSON — имена файлов рядом, относительные: запускать из каталога out).
- Разделы склеиваются по метке, а не по позиции.

## Проверка

- Юнит-тесты: манифест (direct без esp ок; direct с esp — ошибка; uefi-uki без esp —
  ошибка; net для uefi-uki — ошибка; плохой адрес/префикс — ошибка), layout без ESP,
  bootargs (точные строки cmdline и JSON), `resolv_from_pnp`.
- `scripts/test-boot-fc.sh`: QEMU `-M virt,acpi=off -kernel … -append "<boot_args из JSON>"`,
  диск и сеть через `virtio-blk-device`/`virtio-net-device`, манифест
  `tests/rim.fc.test.toml` (сеть 10.0.2.15/24 через 10.0.2.2, DNS 10.0.2.3, без udhcpc).
  Проверки: JSON валиден и ссылается на собранные файлы; `rim: ready`; ssh по ключу;
  корень ro; hostname; адрес eth0 = 10.0.2.15; `/etc/resolv.conf` содержит 10.0.2.3;
  часы не 1970; poweroff завершает VM; `/data` отмонтирован. Печатает время до
  `rim: ready` и до первого ssh.
- Детерминизм: две сборки `fc` дают побайтно одинаковые `.img`, `.Image`, `.firecracker.json`.
- Регрессия: `scripts/test-boot.sh` (14 проверок) и `cargo test --workspace` зелёные.

## Настоящий Firecracker

`docs/firecracker.md`: хост — Linux aarch64 с `/dev/kvm` (bare metal: AWS a1.metal/c7g.metal,
выделенный ARM); официальный бинарник firecracker; `tap0` + NAT; запуск
`firecracker --no-api --config-file rim-fc.firecracker.json`. Прогон на арендованном
сервере — только с явного согласия пользователя; измерения дописываются в документ.

## Вне MVP

vsock-агент, слой libc/Python, lockbox как сервис, загрузка с отдельных дисков без GPT,
snapshot/restore, x86_64.
