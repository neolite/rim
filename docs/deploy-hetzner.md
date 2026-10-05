# Выкладка rim на Hetzner Cloud (CAX, arm64)

Нужно: `hcloud` CLI (`brew install hcloud`) с токеном проекта (`hcloud context create rim`)
и ssh-ключ, загруженный в проект (`hcloud ssh-key list`).

0. Собрать образ и сжать его для передачи (740 МБ → ~8 МБ, почти всё нули):
   ```
   ./rim build                     # rim.toml → out/rim.img
   zstd -19 -f out/rim.img -o out/rim.img.zst
   ```
1. Создать сервер (любой arm64-образ, он будет перезаписан):
   `hcloud server create --name rim --type cax11 --image debian-12 --location fsn1 --ssh-key <имя ключа>`
2. Включить rescue и перезагрузить:
   `hcloud server enable-rescue rim --type linux64 --ssh-key <имя ключа> && hcloud server reboot rim`
3. Залить образ (из каталога репозитория):
   `zstd -dc out/rim.img.zst | ssh root@<IP> 'dd of=/dev/sda bs=4M conv=fsync status=progress'`
   В rescue диск — `/dev/sda`; проверить заранее: `ssh root@<IP> lsblk`.
4. Перезагрузить в систему: `ssh root@<IP> reboot`
5. Через ~30 с: `ssh root@<IP> 'uname -a; cat /proc/cmdline; mount | grep " / "'`
   Ожидается `rootfstype=erofs` в cmdline и `ro` у корня.

Ключ хоста у rim свой (генерируется при первой загрузке на `/data`), а rescue имел другой:
перед шагом 5 убрать старую запись — `ssh-keygen -R <IP>`.

Если ssh не отвечает: открыть консоль в Hetzner Console (VNC).
init печатает туда `rim: ...`, при ошибке там же аварийный shell.

Удалить сервер, чтобы не платить: `hcloud server delete rim`
