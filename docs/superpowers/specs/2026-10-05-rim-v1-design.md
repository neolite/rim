# rim v1 — дизайн

Дата: 2026-10-05. Статус: на ревью.

## Цель

rim — экспериментальная ОС-витрина. Она показывает три идеи:

- система как функция манифеста;
- неизменяемый корень;
- полностью статический userspace.

Ядро — Linux с минимальным конфигом. Своё у нас всё, что выше ядра: сборщик образа, init и формат манифеста.

**Критерий готовности v1:**

1. На чистой машине `./rim build && ./scripts/test-boot.sh` проходит зелёным.
2. Тот же `rim.img` после `dd` на Hetzner CAX11 пускает по ssh.

### Решения, принятые в обсуждении

| Вопрос | Решение |
|---|---|
| Назначение | эксперимент/витрина (не хост агентов, не замена Ubuntu) |
| Ядро | Linux mainline, минимальный конфиг, без модулей |
| Архитектура | aarch64 |
| Проверка | сначала QEMU локально (M1, HVF), потом Hetzner CAX |
| Подход | своя минимальная система (не IX, не Nix) |
| Язык своих компонентов | Rust |
| Разработка | TDD |
| Переносимость | корень не зависит от железа; железо — «таргеты» |

### Вне рамок v1

Всё это будет отдельными спеками:

- обновления и откат;
- растягивание `data` на весь диск;
- IPv6;
- синхронизация времени;
- таргеты для реального железа;
- агенты и песочницы;
- внешние источники пакетов (IX, SoLo);
- пользователи кроме root.

## 1. Архитектура

Конвейер: `rim.toml` → `rim-build` (в Docker, linux/arm64) → `rim.img` → QEMU → `dd` на VPS.

### Разметка диска (GPT)

| Раздел (PARTLABEL) | Размер | Содержимое |
|---|---|---|
| `esp` (FAT32) | 128M | `EFI/BOOT/BOOTAA64.EFI` = UKI слота A; `EFI/rim/rim-A.efi` |
| `root-A` | 256M | erofs, только чтение |
| `root-B` | 256M | пустой; размечается сразу, чтобы v2 не менял формат диска |
| `data` | 64M в v1 | ext4: ssh-ключи хоста, логи |

### Слот = одна версия системы

**UKI** — systemd-stub, в который зашиты:

- ядро;
- cmdline: `root=PARTUUID=<uuid root-A> rootfstype=erofs ro console=ttyAMA0 console=tty0`.

**Initramfs нет:** ядро с вшитыми erofs и virtio монтирует корень само.

**Корень (erofs):**

- `/sbin/init` (rim-init);
- busybox;
- dropbear;
- `/etc/rim/system.toml` — «замороженный» манифест;
- пустые точки монтирования `/proc /sys /dev /run /tmp /data`.

### Таргеты

Корень (erofs) одинаков для всех таргетов aarch64.

Таргет определяет:

- ядро и его конфиг;
- dtb;
- упаковку загрузки;
- разметку диска.

В v1 есть ровно один таргет — `virt` (QEMU virt + Hetzner CAX: UEFI, virtio).

### Репозиторий

```
rim/
  rim.toml
  rim                     # обёртка: docker run … rim-build
  crates/rim-init/
  crates/rim-build/
  crates/rim-manifest/    # общий тип манифеста + валидация (используют init и build)
  pkgs/<name>/{recipe.toml,build.sh,...}
  kernel/virt.config
  docker/Dockerfile
  scripts/test-boot.sh
```

## 2. Манифест и rim-build

### Манифест

TOML без логики. Неизвестные поля — ошибка (`deny_unknown_fields`).

```toml
[system]
hostname = "rim"
packages = ["busybox", "dropbear"]
ssh_authorized_keys = ["ssh-ed25519 AAAA..."]

[service.network]
exec = ["/bin/udhcpc", "-f", "-i", "eth0", "-s", "/etc/rim/udhcpc.script"]
restart = "always"

[service.sshd]
exec = ["/bin/dropbear", "-F", "-E", "-s", "-g", "-r", "/data/ssh/host_ed25519"]
restart = "always"

[target.virt]
boot = "uefi-uki"
kernel_config = "kernel/virt.config"
disk = { esp = "128M", root = "256M", data = "64M" }
```

### Пакеты

Каждый пакет — `pkgs/<name>/recipe.toml` и `build.sh`:

- в `recipe.toml` указаны `version`, `url` и `sha256` исходников;
- `build.sh` собирает статику на musl в `$OUT`.

Ключ кэша — sha256 от трёх вещей:

- содержимого каталога рецепта;
- sha256 исходников;
- идентификатора образа тулчейна (digest Docker-образа).

Результат кладётся в `cache/<hash>-<name>/`. Пакеты v1:

- `linux`;
- `busybox`;
- `dropbear`.

`rim-init` собирается через `cargo build --release --target aarch64-unknown-linux-musl` и кэшируется так же.

### Шаги rim-build

1. Разобрать и проверить манифест. Пакет без рецепта — ошибка.
2. Собрать недостающие пакеты (с кэшем).
3. Собрать дерево корня, записать `/etc/rim/system.toml`.
4. Собрать `root.erofs` через `mkfs.erofs`. Сортировка, фиксированные mtime и uid/gid — 0.
5. Собрать UKI: systemd-stub + ядро + cmdline, через `objcopy`.
6. Собрать ESP (mtools), data (`mke2fs -d` с фиксированными UUID и hash seed) и пустой root-B.
7. Записать GPT и склеить образ по смещениям. Loop-устройства и root не нужны.

### Детерминизм

Один манифест даёт бит-в-бит одинаковый `rim.img`. Для этого:

- `SOURCE_DATE_EPOCH` выводится из манифеста (константа в v1);
- UUID разделов и ФС — из `sha256(manifest || label)`;
- никаких случайных значений, времени или порядка из файловой системы.

### Где запускается

`rim-build` работает в контейнере `docker/Dockerfile` (alpine, linux/arm64). Версии `erofs-utils`, `mtools`, `e2fsprogs`, `binutils`, systemd-stub и Rust зафиксированы. Обёртка `./rim build` монтирует репозиторий в контейнер.

## 3. rim-init (PID 1)

### Последовательность

1. Смонтировать `/proc`, `/sys`, devtmpfs `/dev`, tmpfs `/run` и `/tmp`.
2. Найти раздел с `PARTNAME=data` по `/sys/class/block/*/uevent` и смонтировать ext4 в `/data`.
3. Если нет `/data/ssh/host_ed25519`, сгенерировать его через `dropbearkey -t ed25519`.
4. Записать ключи из манифеста в `/data/root/.ssh/authorized_keys`, создав каталог с правами 0700. Домашний каталог root в `/etc/passwd` корня — `/data/root`, потому что сам корень только для чтения. Файл перезаписывается при каждой загрузке: источник правды — манифест.
5. Прочитать `/etc/rim/system.toml`, задать hostname, поднять `lo`.
6. Запустить сервисы. Сеть — обычный сервис.
7. Надзирать за сервисами:
   - перезапуск с backoff 1 → 2 → 4 … → 30 с;
   - сброс backoff после 60 с стабильной работы;
   - stdout/stderr → `/data/log/<svc>.log` и в консоль с префиксом `[svc]`.
8. Собирать зомби.
9. Напечатать в консоль `rim: ready`.
10. Выключаться по соглашению busybox: `SIGUSR2` — poweroff, `SIGTERM` — reboot, `SIGUSR1` — halt. Порядок:
    - SIGTERM сервисам;
    - 5 с ожидания;
    - SIGKILL;
    - `sync`;
    - umount `/data`;
    - `reboot(2)`.

### Обработка ошибок

PID 1 не паникует и не делает `unwrap()` в рабочем цикле. Если конфиг не читается или `/data` не монтируется, init пишет ошибку в консоль и запускает аварийный `/bin/sh` на консоли. Сервисы в этом случае не стартуют.

### Внутреннее устройство

Логика init — чистый автомат без системных вызовов:

```
события:   ServiceExited{name, status} | Tick{now} | ShutdownRequested{kind}
состояние: name -> Running{pid, since} | Backoff{until, delay} | Stopped
действия:  Spawn{name} | Signal{pid, sig} | Log{line} | Reboot{kind}
fn step(state, event) -> (state, Vec<Action>)
```

Слой эффектов:

- переводит `signalfd` (SIGCHLD и сигналы выключения) и таймер в события;
- выполняет действия через `fork`/`exec`/`kill`/`reboot`.

Зависимости: `rustix`, `serde`, `toml`. Async-рантайма нет.

### Безопасность v1

- `/` только для чтения;
- SUID-бинарников нет;
- ssh только по ключу (`-s -g`);
- единственный пользователь — root.

## 4. Проверка и выкладка

### Тесты

TDD: каждый слой начинается с падающего теста.

1. **Юнит** (`cargo test` на Mac):
   - автомат rim-init;
   - валидация манифеста;
   - запись GPT (заголовки, CRC32, резервная GPT);
   - детерминированные UUID.
2. **Сборка** (в Docker):
   - две сборки дают одинаковый sha256;
   - `sgdisk -v rim.img` проходит;
   - `fsck.erofs root.erofs` проходит.
3. **Загрузка** (`scripts/test-boot.sh`, `qemu-system-aarch64 -M virt -accel hvf -cpu host`, edk2, virtio, порт 2222→22, serial в файл). Проверки:
   - `rim: ready` за ≤30 с;
   - ssh по тестовому ключу работает;
   - `touch /x` падает;
   - `kill` dropbear — сервис снова слушает порт;
   - `poweroff` — QEMU завершается сам.

### Ядро (`kernel/virt.config`)

Всё вшито, модулей нет:

- EFI stub;
- virtio-pci, virtio-blk, virtio-scsi, virtio-net, virtio-console;
- erofs, ext4, vfat не нужен;
- devtmpfs;
- PL011 (ttyAMA0);
- efifb и virtio-gpu (графическая консоль Hetzner);
- PL031 RTC;
- Landlock и seccomp — включены заранее для v5.

### Выкладка на Hetzner CAX11

1. Создать сервер, включить rescue (arm64), перезагрузиться.
2. `curl <url>/rim.img | dd of=/dev/sda bs=4M conv=fsync`, затем `reboot`.
3. Зайти по ssh на публичный IPv4. Адрес приходит по DHCP.

Создание и удаление сервера — внешние действия на аккаунте пользователя. Они выполняются только после явного подтверждения.

Проверить на месте:

- тип диска (`/dev/sda` virtio-scsi или `/dev/vda`);
- видимость вывода в VNC-консоли.

### Дорожная карта после v1

| Этап | Содержание |
|---|---|
| v2 | A/B-обновления, счётчик попыток загрузки, автооткат |
| v3 | растягивание `data`, IPv6, синхронизация времени |
| v4 | таргеты: Raspberry Pi 4, ТВ-приставка |
| v5 | песочницы агентов: Landlock+seccomp на VPS, microVM при наличии KVM |
| v6 | пакеты из IX, SoLo для GPU |

## Риски

| Риск | Мера |
|---|---|
| systemd-stub в alpine имеет другую версию или отсутствует | зафиксировать пакет; запасной вариант — EFI stub ядра с `CONFIG_CMDLINE` |
| Недетерминизм от mkfs.erofs/mke2fs | тест «две сборки» с первого дня; фиксировать флаги (`-T`, `-U`, `-E hash_seed=`) |
| На Hetzner нет вывода в консоль | efifb + `console=tty0`; сначала QEMU с `-device ramfb` как имитация VNC |
| Docker на Mac не запущен | `./rim` проверяет `docker info` и печатает понятную ошибку |
