//! Манифест rim: данные без логики. Всё, что не описано здесь, — ошибка.

use serde::{Deserialize, Deserializer};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestError(pub String);

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ManifestError {}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub system: System,
    #[serde(default)]
    pub service: BTreeMap<String, Service>,
    #[serde(default)]
    pub target: BTreeMap<String, Target>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct System {
    pub hostname: String,
    pub packages: Vec<String>,
    #[serde(default)]
    pub ssh_authorized_keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Service {
    pub exec: Vec<String>,
    #[serde(default)]
    pub restart: Restart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Restart {
    #[default]
    Always,
    Never,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub boot: Boot,
    pub kernel: String,
    pub disk: Disk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Boot {
    #[serde(rename = "uefi-uki")]
    UefiUki,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Disk {
    pub esp: Size,
    pub root: Size,
    pub data: Size,
}

/// Размер в байтах. В манифесте записывается как "128M", "1G", "1024K".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Size(pub u64);

impl Size {
    pub fn parse(s: &str) -> Result<Size, String> {
        let (num, unit) = s.split_at(s.len().saturating_sub(1));
        let mult: u64 = match unit {
            "K" => 1 << 10,
            "M" => 1 << 20,
            "G" => 1 << 30,
            _ => return Err(format!("размер {s:?}: нужен суффикс K, M или G")),
        };
        let n: u64 = num.parse().map_err(|_| format!("размер {s:?}: не число"))?;
        n.checked_mul(mult).map(Size).ok_or_else(|| format!("размер {s:?}: переполнение"))
    }
}

impl<'de> Deserialize<'de> for Size {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Size::parse(&s).map_err(serde::de::Error::custom)
    }
}

const MIB: u64 = 1 << 20;

pub fn parse(text: &str) -> Result<Manifest, ManifestError> {
    let m: Manifest = toml::from_str(text).map_err(|e| ManifestError(e.to_string()))?;
    validate(&m)?;
    Ok(m)
}

fn validate(m: &Manifest) -> Result<(), ManifestError> {
    let err = |s: String| Err(ManifestError(s));
    let h = &m.system.hostname;
    let ok_host = !h.is_empty()
        && h.len() <= 63
        && h.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !h.starts_with('-');
    if !ok_host {
        return err(format!("system.hostname {h:?}: только a-z, 0-9 и '-', до 63 символов"));
    }
    if m.system.ssh_authorized_keys.is_empty() {
        return err("system.ssh_authorized_keys пуст: без ключа в систему не войти".into());
    }
    for k in &m.system.ssh_authorized_keys {
        if !(k.starts_with("ssh-") || k.starts_with("ecdsa-")) || k.contains('\n') {
            return err(format!("system.ssh_authorized_keys: {k:?} не похож на публичный ключ"));
        }
    }
    for (name, s) in &m.service {
        match s.exec.first() {
            Some(p) if p.starts_with('/') => {}
            _ => return err(format!("service.{name}.exec: нужен абсолютный путь первым элементом")),
        }
    }
    if m.target.is_empty() {
        return err("нет ни одного [target.*]".into());
    }
    for (name, t) in &m.target {
        for (field, size) in [("esp", t.disk.esp), ("root", t.disk.root), ("data", t.disk.data)] {
            if size.0 < MIB || size.0 % MIB != 0 {
                return err(format!("target.{name}.disk.{field}: нужно кратно 1M и не меньше 1M"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"
[system]
hostname = "rim"
packages = ["busybox", "dropbear"]
ssh_authorized_keys = ["ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExample test"]

[service.sshd]
exec = ["/bin/dropbear", "-F"]

[service.oneshot]
exec = ["/bin/true"]
restart = "never"

[target.virt]
boot = "uefi-uki"
kernel = "linux"
disk = { esp = "128M", root = "256M", data = "64M" }
"#;

    #[test]
    fn parses_good_manifest() {
        let m = parse(GOOD).unwrap();
        assert_eq!(m.system.hostname, "rim");
        assert_eq!(m.service["sshd"].restart, Restart::Always);
        assert_eq!(m.service["oneshot"].restart, Restart::Never);
        let t = &m.target["virt"];
        assert_eq!(t.boot, Boot::UefiUki);
        assert_eq!(t.kernel, "linux");
        assert_eq!(t.disk.esp, Size(128 << 20));
        assert_eq!(t.disk.data, Size(64 << 20));
    }

    #[test]
    fn rejects_unknown_field() {
        let bad = GOOD.replace("hostname = \"rim\"", "hostname = \"rim\"\nhostnme = \"x\"");
        let e = parse(&bad).unwrap_err();
        assert!(e.0.contains("hostnme"), "{e}");
    }

    #[test]
    fn rejects_missing_ssh_keys() {
        let bad = GOOD.replace(
            "ssh_authorized_keys = [\"ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExample test\"]",
            "ssh_authorized_keys = []",
        );
        let e = parse(&bad).unwrap_err();
        assert!(e.0.contains("ssh_authorized_keys"), "{e}");
    }

    #[test]
    fn rejects_bad_key_prefix() {
        let bad = GOOD.replace("ssh-ed25519 AAAA", "rsa AAAA");
        assert!(parse(&bad).unwrap_err().0.contains("ssh_authorized_keys"));
    }

    #[test]
    fn rejects_bad_hostname() {
        let bad = GOOD.replace("hostname = \"rim\"", "hostname = \"Rim_1\"");
        assert!(parse(&bad).unwrap_err().0.contains("hostname"));
    }

    #[test]
    fn rejects_relative_exec_and_empty_exec() {
        let rel = GOOD.replace("\"/bin/true\"", "\"true\"");
        assert!(parse(&rel).unwrap_err().0.contains("oneshot"));
        let empty = GOOD.replace("exec = [\"/bin/true\"]", "exec = []");
        assert!(parse(&empty).unwrap_err().0.contains("oneshot"));
    }

    #[test]
    fn rejects_unaligned_size_and_requires_target() {
        let bad = GOOD.replace("data = \"64M\"", "data = \"1500K\"");
        assert!(parse(&bad).unwrap_err().0.contains("data"));
        let no_target = GOOD.split("[target.virt]").next().unwrap().to_string();
        assert!(parse(&no_target).unwrap_err().0.contains("target"));
    }

    #[test]
    fn size_parsing() {
        assert_eq!(Size::parse("1M"), Ok(Size(1 << 20)));
        assert_eq!(Size::parse("2G"), Ok(Size(2 << 30)));
        assert_eq!(Size::parse("1024K"), Ok(Size(1 << 20)));
        assert!(Size::parse("12").is_err());
        assert!(Size::parse("M").is_err());
    }
}
