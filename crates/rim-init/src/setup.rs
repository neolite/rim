//! Чистые функции для установки системы: тестируются без Linux.

/// `/sys/class/tty/console/active` → пути консолей, куда init пишет свои строки.
pub fn console_paths(active: &str) -> Vec<String> {
    let v: Vec<String> = active.split_whitespace().map(|t| format!("/dev/{t}")).collect();
    if v.is_empty() { vec!["/dev/console".into()] } else { v }
}

/// (имя блочного устройства, содержимое его uevent) → `/dev/<имя>` раздела с нужным PARTNAME.
pub fn find_partition(uevents: &[(String, String)], partname: &str) -> Option<String> {
    uevents.iter().find_map(|(dev, ue)| {
        ue.lines()
            .any(|l| l.strip_prefix("PARTNAME=") == Some(partname))
            .then(|| format!("/dev/{dev}"))
    })
}

pub fn authorized_keys(keys: &[String]) -> String {
    keys.iter().map(|k| format!("{k}\n")).collect()
}

/// Построчно читает вывод сервиса до EOF и отдаёт строки в `f`.
/// Байты не-UTF-8 заменяются на U+FFFD: `lines()` на них остановился бы,
/// поток закрыл бы трубу, и сервис умер бы от SIGPIPE на следующей записи.
pub fn for_each_line(mut src: impl std::io::BufRead, mut f: impl FnMut(&str)) {
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match src.read_until(b'\n', &mut buf) {
            Ok(0) => return,
            Ok(_) => {
                if buf.last() == Some(&b'\n') {
                    buf.pop();
                }
                f(&String::from_utf8_lossy(&buf));
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return,
        }
    }
}

/// Лог сервиса в файл с потолком: при превышении `cap` байт текущий файл
/// становится `<path>.1` (прежний `.1` теряется), запись идёт в новый.
pub struct CappedLog {
    path: std::path::PathBuf,
    cap: u64,
    file: Option<std::fs::File>,
    size: u64,
}

impl CappedLog {
    pub fn open(path: impl Into<std::path::PathBuf>, cap: u64) -> CappedLog {
        let path = path.into();
        let file = std::fs::OpenOptions::new().create(true).append(true).open(&path).ok();
        let size = file.as_ref().and_then(|f| f.metadata().ok()).map_or(0, |m| m.len());
        CappedLog { path, cap, file, size }
    }

    pub fn line(&mut self, line: &str) {
        use std::io::Write;
        let add = line.len() as u64 + 1;
        if self.size > 0 && self.size + add > self.cap {
            let mut old = self.path.clone().into_os_string();
            old.push(".1");
            let _ = std::fs::rename(&self.path, &old);
            self.file = std::fs::OpenOptions::new().create(true).append(true).open(&self.path).ok();
            self.size = 0;
        }
        if let Some(f) = self.file.as_mut() {
            if writeln!(f, "{line}").is_ok() {
                self.size += line.len() as u64 + 1;
            }
        }
    }
}

/// Строки DNS из /proc/net/pnp (ядро настроило сеть по `ip=`) в формате resolv.conf.
/// `None`, если nameserver нет: тогда resolv.conf не трогаем (его может писать udhcpc).
pub fn resolv_from_pnp(pnp: &str) -> Option<String> {
    let mut out = String::new();
    let mut has_ns = false;
    for l in pnp.lines() {
        if let Some(ns) = l.strip_prefix("nameserver ") {
            if ns.trim() == "0.0.0.0" {
                continue;
            }
            has_ns = true;
        } else if !l.starts_with("domain ") {
            continue; // #MANUAL, #PROTO, bootserver — не для resolv.conf
        }
        out += l;
        out.push('\n');
    }
    has_ns.then_some(out)
}

/// Firecracker на x86: гость не может выключить VM, процесс завершает только перезагрузка
/// (reboot=k). Сборщик ставит `rim.poweroff=reboot` — тогда poweroff выполняется как reboot.
pub fn poweroff_as_reboot(cmdline: &str) -> bool {
    cmdline.split_whitespace().any(|w| w == "rim.poweroff=reboot")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("rim-setup-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn invalid_utf8_line_does_not_stop_reading() {
        let input: &[u8] = b"first\n\xff\xfe broken\nthird\n";
        let mut got = Vec::new();
        for_each_line(input, |l| got.push(l.to_string()));
        assert_eq!(got.len(), 3, "{got:?}");
        assert_eq!(got[0], "first");
        assert!(got[1].ends_with(" broken"), "{got:?}");
        assert_eq!(got[2], "third");
    }

    #[test]
    fn last_line_without_newline_is_kept() {
        let mut got = Vec::new();
        for_each_line(&b"a\nb"[..], |l| got.push(l.to_string()));
        assert_eq!(got, vec!["a", "b"]);
    }

    #[test]
    fn capped_log_rotates_once_over_cap() {
        let d = tmpdir("rotate");
        let p = d.join("svc.log");
        let mut log = CappedLog::open(&p, 100);
        for i in 0..50 {
            log.line(&format!("line {i:03}")); // 8 байт + \n
        }
        let cur = std::fs::metadata(&p).unwrap().len();
        let old = std::fs::metadata(d.join("svc.log.1")).map(|m| m.len()).unwrap_or(0);
        assert!(cur <= 100, "текущий лог {cur} байт при потолке 100");
        assert!(old > 0 && old <= 110, "svc.log.1 = {old} байт");
        let tail = std::fs::read_to_string(&p).unwrap();
        assert!(tail.ends_with("line 049\n"), "{tail:?}");
    }

    #[test]
    fn capped_log_counts_existing_size_after_reboot() {
        let d = tmpdir("reopen");
        let p = d.join("svc.log");
        std::fs::write(&p, "x".repeat(95)).unwrap();
        let mut log = CappedLog::open(&p, 100);
        log.line("0123456789");
        assert!(d.join("svc.log.1").exists(), "размер прошлых загрузок не учтён");
    }

    #[test]
    fn console_paths_from_active_list() {
        assert_eq!(console_paths("tty0 ttyAMA0\n"), vec!["/dev/tty0", "/dev/ttyAMA0"]);
        assert_eq!(console_paths(""), vec!["/dev/console"]);
    }

    #[test]
    fn finds_partition_by_partname() {
        let ue = vec![
            ("vda".to_string(), "MAJOR=254\nDEVNAME=vda\nDEVTYPE=disk\n".to_string()),
            ("vda2".to_string(), "DEVNAME=vda2\nDEVTYPE=partition\nPARTNAME=root-A\n".to_string()),
            ("vda4".to_string(), "DEVNAME=vda4\nDEVTYPE=partition\nPARTNAME=data\n".to_string()),
        ];
        assert_eq!(find_partition(&ue, "data"), Some("/dev/vda4".to_string()));
        assert_eq!(find_partition(&ue, "root-B"), None);
    }

    #[test]
    fn partname_must_match_exactly() {
        let ue = vec![("sda5".to_string(), "DEVNAME=sda5\nPARTNAME=data-old\n".to_string())];
        assert_eq!(find_partition(&ue, "data"), None);
    }

    #[test]
    fn authorized_keys_one_per_line() {
        let k = vec!["ssh-ed25519 AAA a".to_string(), "ssh-ed25519 BBB b".to_string()];
        assert_eq!(authorized_keys(&k), "ssh-ed25519 AAA a\nssh-ed25519 BBB b\n");
    }

    #[test]
    fn resolv_from_pnp_keeps_nameservers_and_domain() {
        let pnp = "#MANUAL\ndomain example.org\nnameserver 10.0.2.3\nnameserver 1.1.1.1\nbootserver 10.0.2.2\n";
        assert_eq!(
            resolv_from_pnp(pnp).as_deref(),
            Some("domain example.org\nnameserver 10.0.2.3\nnameserver 1.1.1.1\n")
        );
    }

    #[test]
    fn resolv_from_pnp_none_without_nameserver() {
        assert_eq!(resolv_from_pnp("#MANUAL\n"), None);
        assert_eq!(resolv_from_pnp(""), None);
        assert_eq!(resolv_from_pnp("#MANUAL\nnameserver 0.0.0.0\n"), None);
    }

    #[test]
    fn poweroff_as_reboot_only_with_exact_flag() {
        assert!(poweroff_as_reboot("root=PARTUUID=x reboot=k rim.poweroff=reboot ip=1.2.3.4\n"));
        assert!(!poweroff_as_reboot("root=PARTUUID=x console=ttyS0 panic=10\n"));
        assert!(!poweroff_as_reboot("rim.poweroff=rebootX"));
        assert!(!poweroff_as_reboot("xrim.poweroff=reboot"));
        assert!(!poweroff_as_reboot(""));
    }
}
