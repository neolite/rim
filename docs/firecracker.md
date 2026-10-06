# rim в Firecracker

Таргет `boot = "direct"`: Firecracker сам передаёт гостю ядро и командную строку, UEFI и ESP
не нужны. Диск — тот же GPT-образ (`root-A`, `root-B`, `data`), ядро находит корень по
`root=PARTUUID=` и `/data` по метке раздела.

## Сборка

```toml
# rim.toml (фрагмент)
[target.fc]
boot = "direct"
kernel = "linux"
disk = { root = "64M", data = "256M" }
net = { address = "172.16.0.2/30", gateway = "172.16.0.1", dns = "1.1.1.1" }
```

Сервис `udhcpc` для этого таргета не нужен: DHCP в Firecracker нет, адрес ядро ставит из
`ip=` на командной строке, DNS init берёт из `/proc/net/pnp`.

```sh
./rim build --target fc --out out/fc/rim-fc.img
```

В `out/fc/` появляются три файла:

| Файл | Что это |
|---|---|
| `rim-fc.img` | GPT-диск, бит-в-бит воспроизводимый |
| `rim-fc.Image` | несжатое ядро arm64 (Firecracker не принимает сжатое) |
| `rim-fc.firecracker.json` | конфиг для `--config-file`: ядро, cmdline, диск, `eth0` на `tap0`, 1 vCPU, 256 MiB |

Пути в JSON — имена соседних файлов, поэтому Firecracker запускается из этого каталога.

## x86_64

Тот же манифест, в таргете `arch = "x86_64"` (только с `boot = "direct"`):

```toml
[target.fc-x86]
boot = "direct"
arch = "x86_64"
kernel = "linux"
disk = { root = "64M", data = "256M" }
net = { address = "172.16.0.2/30", gateway = "172.16.0.1", dns = "1.1.1.1" }
```

```sh
./rim --arch x86_64 build --target fc-x86 --out out/fc-x86/rim-fc.img
```

Сборка идёт тем же Dockerfile под `linux/amd64` (на Apple Silicon — Rosetta в OrbStack/Docker
Desktop). Ядро — несжатый ELF `rim-fc.vmlinux`, как рекомендует Firecracker для x86.

Отличие в выключении: на x86 у Firecracker нет управления питанием, `poweroff` гостя процесс
не завершает. Поэтому в cmdline x86 стоят `reboot=k` (сброс через i8042, его Firecracker
перехватывает и завершается) и `rim.poweroff=reboot` — init выполняет poweroff как
перезагрузку. Для пользователя разницы нет: `poweroff` завершает firecracker с `exit_code=0`
(проверено в Firecracker 1.17.0, 2026-10-06).

Ядро x86 собрано с ACPI и PCI: Firecracker с 1.8 описывает устройства таблицами ACPI, а
загрузку по MPTable объявил устаревшей (уберут в 2.0). PCI-устройств в Firecracker нет, но без
`CONFIG_PCI` ACPI в госте не поднимается (`AE_BAD_PARAMETER`), и virtio-драйверы отказывают с
`-22` — диска нет, загрузка висит на `Waiting for root device`. Локальный тест проходит путь
MPTable (под TCG у ACPI-пути нет источника калибровки TSC); путь ACPI и kvmclock проверены в
Firecracker 1.17.0. В логе остаются безвредные `virtio-mmio … error -16`: Firecracker сообщает
устройства и в ACPI, и в cmdline (`virtio_mmio.device=`), второе описание ядро отклоняет.

Хост для x86 не обязан быть bare metal: подойдёт облачная VM с вложенной виртуализацией
(`/dev/kvm` внутри VM). Firecracker официально проверяется на bare metal, во вложенной
виртуализации работает, но медленнее — для замеров это нужно учитывать.

Локально: `ARCH=x86_64 ./scripts/test-boot-fc.sh` (QEMU `-M microvm`; на Apple Silicon
без ускорения — проверка корректности, не скорости).

## Хост

aarch64: нужен Linux aarch64 с `/dev/kvm`. Обычная облачная VM не подойдёт: вложенной виртуализации
на ARM у облаков нет. Подходят bare-metal инстансы на Graviton2 и новее (AWS `m6g.metal`,
`c6g.metal`, `c7g.metal`, `m7g.metal`); Graviton1 (`a1.metal`) Firecracker не поддерживает.
Выделенный ARM-сервер — если его процессор не старше Neoverse N1. Mac на Apple Silicon не подходит (нет KVM); локально таргет
проверяется в QEMU — `scripts/test-boot-fc.sh`.

## Запуск

```sh
# 1. Firecracker (официальный релиз; на x86 — ARCH=x86_64)
ARCH=$(uname -m)
VER=$(curl -fsSLI -o /dev/null -w '%{url_effective}' https://github.com/firecracker-microvm/firecracker/releases/latest | sed 's|.*/||')
curl -fsSL "https://github.com/firecracker-microvm/firecracker/releases/download/$VER/firecracker-$VER-$ARCH.tgz" | tar xz
sudo install release-$VER-$ARCH/firecracker-$VER-$ARCH /usr/local/bin/firecracker
sudo setfacl -m u:"$USER":rw /dev/kvm      # запуск без root

# 2. tap0 + NAT (адреса — из net в манифесте: гость 172.16.0.2, шлюз 172.16.0.1)
sudo ip tuntap add tap0 mode tap user "$USER"
sudo ip addr add 172.16.0.1/30 dev tap0
sudo ip link set tap0 up
sudo sysctl -w net.ipv4.ip_forward=1
UPLINK=$(ip route show default | awk '{print $5; exit}')
sudo iptables -t nat -A POSTROUTING -o "$UPLINK" -s 172.16.0.0/30 -j MASQUERADE
# FORWARD по умолчанию DROP на хостах с Docker — без этих правил у гостя нет выхода наружу
sudo iptables -I FORWARD -i tap0 -o "$UPLINK" -j ACCEPT
sudo iptables -I FORWARD -i "$UPLINK" -o tap0 -m conntrack --ctstate RELATED,ESTABLISHED -j ACCEPT

# 3. Гость (консоль — в этом же терминале). Гость пишет в /data на диске, поэтому
#    запускаем копию: собранный rim-fc.img остаётся бит-в-бит воспроизводимым.
cd out/fc
cp rim-fc.img run.img
sed 's/"rim-fc.img"/"run.img"/' rim-fc.firecracker.json > run.json
firecracker --no-api --config-file run.json

# 4. С хоста, в другом терминале
ssh root@172.16.0.2
```

`poweroff` внутри гостя завершает процесс firecracker. Перезагрузки как таковой у
Firecracker нет: `reboot` тоже завершает процесс, перезапуск — забота того, кто его запустил.

## Замеры

| Где | До `rim: ready` | До ssh | Примечание |
|---|---|---|---|
| QEMU/HVF на M1 Pro (`test-boot-fc.sh`) | 0,90 с | 0,97 с | 1 vCPU, 256 MiB; 2026-10-05 |
| Firecracker 1.17.0, AWS `c8i.xlarge` (вложенная виртуализация) | 0,50 с | 0,48 с* | x86_64, 1 vCPU, 256 MiB; 2026-10-06 |

\* медиана 20 загрузок до баннера sshd, отдельная проба — до строки `rim: ready` в консоли;
время считается от запуска процесса firecracker.

Сравнение в том же Firecracker (`scripts/fc-bench.sh`, сборка и замер на хосте из
`deploy/fc-bench`, 20 загрузок каждой системы, медианы):

| | rim | rim + lockbox-node | Alpine 3.24 (linux-virt, OpenRC) |
|---|---|---|---|
| До готовности | **0,48 с** (0,45–0,50) до ssh | **0,56 с** (0,51–0,57) до ответа gRPC | 3,37 с (2,82–3,92) до ssh |
| Выключение (команда по ssh → выход firecracker) | 0,19 с | 0,18 с | 2,32 с |
| Память занята (MemTotal − MemAvailable) | 11 МиБ | 14 МиБ (lockbox-node RSS 20 МиБ) | 39 МиБ |
| Процессов (с потоками ядра) | 37 | 37 | 60 |
| Занято на корне | 3,7 МБ | 31 МБ (статический lockbox-node 28 МБ) | 80 МБ |

Ядро у всех одной версии (6.18.55). Для честного старта сети MAC гостя фиксирован, а ARP-запись
на хосте прописана заранее: иначе ранние пробы помечают соседа FAILED, хост повторяет ARP раз в
секунду, и все замеры округляются до ~1 с.
