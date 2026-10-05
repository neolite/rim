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

#[cfg(test)]
mod tests {
    use super::*;

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
}
