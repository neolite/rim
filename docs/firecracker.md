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

## Хост

Нужен Linux aarch64 с `/dev/kvm`. Обычная облачная VM не подойдёт: вложенной виртуализации
на ARM у облаков нет. Подходят bare-metal инстансы на Graviton2 и новее (AWS `m6g.metal`,
`c6g.metal`, `c7g.metal`, `m7g.metal`); Graviton1 (`a1.metal`) Firecracker не поддерживает.
Выделенный ARM-сервер — если его процессор не старше Neoverse N1. Mac на Apple Silicon не подходит (нет KVM); локально таргет
проверяется в QEMU — `scripts/test-boot-fc.sh`.

## Запуск

```sh
# 1. Firecracker (официальный релиз, aarch64)
ARCH=aarch64
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
| Firecracker, bare-metal | — | — | ещё не запускался |
