# rim: таргет Firecracker (MVP) — план

> Исполнение: superpowers:executing-plans (натив). Шаги — RED → GREEN → коммит.

**Spec:** docs/superpowers/specs/2026-10-05-rim-firecracker-design.md
**Ветка:** feat/firecracker

## Global Constraints
- Таргет `virt` не меняется: ESP, GPT и data побайтно как на main; root-A отличается только
  бинарником init (он линкует rim-manifest) — см. Ruling Task 2 в ledger.
- `scripts/test-boot.sh` — 14/14 ok; `cargo test --workspace` зелёный, без предупреждений.
- Сборка `fc` детерминирована (cmp трёх артефактов).

## Review Focus
1. Манифест `uefi-uki` без `esp` или `direct` с `esp` → понятная ошибка, а не паника в layout.
2. `net.address` с префиксом 0 или >32, мусор вместо IPv4 → ошибка валидации.
3. Порядок разделов без ESP: `root-A` должен остаться с тем же PARTUUID-выводом (по метке).
4. JSON с путями: запуск из каталога out — относительные имена файлов.
5. `/proc/net/pnp` без nameserver (ip= без dns) → resolv.conf не перезаписывается пустым.

---

### Task 1: Манифест — `boot = "direct"`, необязательный `esp`, блок `net`
Files: `crates/rim-manifest/src/lib.rs`.
- RED: тесты `direct_without_esp_ok`, `direct_with_esp_rejected`, `uki_without_esp_rejected`,
  `net_only_for_direct`, `net_bad_address_rejected` (префикс 0, 33, «1.2.3», «a.b.c.d/24»,
  gateway «x»), `net_parsed` (поля и префикс).
- GREEN: `Boot::Direct` (`"direct"`), `Disk.esp: Option<Size>`, `Target.net: Option<Net>`,
  `Net { address: Ipv4Addr, prefix: u8, gateway: Ipv4Addr, dns: Option<Ipv4Addr> }`
  (address в TOML — строка `a.b.c.d/p`, разбирается кастомным Deserialize в `Cidr`).
- Потребители (`layout`, `image`) правятся минимально, чтобы компилировалось.

### Task 2: Разметка без ESP и склейка по метке
Files: `crates/rim-build/src/layout.rs`, `image.rs`.
- RED: `layout::tests::without_esp_starts_with_root_a`.
- GREEN: `plan` пропускает esp при `None`; `image.rs` сопоставляет образы разделам по `label`.
- Проверка: sha256 тестового образа `virt` равен базовому (Global Constraints).

### Task 3: `bootargs.rs`
Files: `crates/rim-build/src/bootargs.rs`, `main.rs`.
- RED: `cmdline_uefi` (точная строка как сейчас), `cmdline_direct_with_net`
  (`ip=10.0.2.15::10.0.2.2:255.255.255.0::eth0:off:10.0.2.3`), `cmdline_direct_without_dns`,
  `netmask_from_prefix` (/30, /24, /32, /1), `firecracker_json_exact`.
- GREEN: `cmdline(&Target, root_partuuid) -> String`, `firecracker_json(kernel, disk, args)`.

### Task 4: `image.rs` — ветка `direct`
- `uefi-uki`: как раньше, cmdline из `bootargs`.
- `direct`: без UKI/ESP; `<stem>.Image` и `<stem>.firecracker.json` рядом с образом.
- Проверка: `./rim build --manifest tests/rim.fc.test.toml --target fc --out out/rim-fc.img`
  выдаёт три файла; `sgdisk -p` — 3 раздела; virt-sha256 прежний.

### Task 5: Ядро
Files: `pkgs/linux/virt.config`.
- `SERIAL_8250`, `SERIAL_8250_CONSOLE`, `SERIAL_OF_PLATFORM`, `ARM_AMBA`, `IP_PNP`.
- Проверка: `./rim pkg linux` проходит check-config. (virt-sha256 после этого меняется —
  это ожидаемо, новая база фиксируется в ledger.)

### Task 6: init — DNS из `/proc/net/pnp`
Files: `crates/rim-init/src/setup.rs`, `effects.rs`.
- RED: `resolv_from_pnp_keeps_nameservers`, `resolv_from_pnp_none_without_nameserver`.
- GREEN: функция + вызов в `prepare` после монтирования /run.

### Task 7: `scripts/test-boot-fc.sh` + `tests/rim.fc.test.toml`
- Проверки из спеки; печать времени до ready и до ssh.
- Детерминизм fc; регрессия `test-boot.sh` 14/14.

### Task 8: `docs/firecracker.md`
- Хост, бинарник firecracker, tap0+NAT, запуск, ожидания; пустая таблица замеров.
