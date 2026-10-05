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
    /// Статическая сеть через `ip=` ядра; только для `direct` (на VPS сеть даёт DHCP).
    #[serde(default)]
    pub net: Option<Net>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Boot {
    /// UEFI грузит UKI из ESP (VPS, QEMU с прошивкой).
    #[serde(rename = "uefi-uki")]
    UefiUki,
    /// Ядро и cmdline передаёт сам гипервизор (Firecracker): без UEFI и ESP.
    #[serde(rename = "direct")]
    Direct,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Disk {
    /// Обязателен для `uefi-uki`, запрещён для `direct`.
    #[serde(default)]
    pub esp: Option<Size>,
    pub root: Size,
    pub data: Size,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Net {
    pub address: Cidr,
    pub gateway: std::net::Ipv4Addr,
    #[serde(default)]
    pub dns: Option<std::net::Ipv4Addr>,
}

/// IPv4-адрес с префиксом: "172.16.0.2/30". Префикс 1..=32.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cidr {
    pub addr: std::net::Ipv4Addr,
    pub prefix: u8,
}

impl Cidr {
    pub fn parse(s: &str) -> Result<Cidr, String> {
        let (a, p) = s.split_once('/').ok_or_else(|| format!("адрес {s:?}: нужен вид a.b.c.d/префикс"))?;
        let addr = a.parse().map_err(|_| format!("адрес {s:?}: не IPv4"))?;
        let prefix: u8 = p.parse().map_err(|_| format!("адрес {s:?}: префикс не число"))?;
        if !(1..=32).contains(&prefix) {
            return Err(format!("адрес {s:?}: префикс 1..=32"));
        }
        Ok(Cidr { addr, prefix })
    }
}

impl<'de> Deserialize<'de> for Cidr {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Cidr::parse(&s).map_err(serde::de::Error::custom)
    }
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
        match (t.boot, t.disk.esp.is_some()) {
            (Boot::UefiUki, false) => return err(format!("target.{name}.disk.esp: обязателен для boot = \"uefi-uki\"")),
            (Boot::Direct, true) => return err(format!("target.{name}.disk.esp: при boot = \"direct\" ESP не нужен")),
            _ => {}
        }
        if t.boot == Boot::UefiUki && t.net.is_some() {
            return err(format!("target.{name}.net: только для boot = \"direct\" (uefi-uki берёт сеть по DHCP)"));
        }
        let sizes = t.disk.esp.map(|s| ("esp", s)).into_iter().chain([("root", t.disk.root), ("data", t.disk.data)]);
        for (field, size) in sizes {
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
        assert_eq!(t.disk.esp, Some(Size(128 << 20)));
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

    fn with_fc(target: &str) -> String {
        format!("{GOOD}\n[target.fc]\n{target}\n")
    }

    const FC: &str = r#"boot = "direct"
kernel = "linux"
disk = { root = "64M", data = "256M" }"#;

    #[test]
    fn direct_without_esp_ok() {
        let m = parse(&with_fc(FC)).unwrap();
        let t = &m.target["fc"];
        assert_eq!(t.boot, Boot::Direct);
        assert_eq!(t.disk.esp, None);
        assert_eq!(t.net, None);
        assert_eq!(m.target["virt"].disk.esp, Some(Size(128 << 20)));
    }

    #[test]
    fn direct_with_esp_rejected() {
        let bad = with_fc(&FC.replace("{ root", "{ esp = \"128M\", root"));
        let e = parse(&bad).unwrap_err();
        assert!(e.0.contains("target.fc.disk.esp"), "{e}");
    }

    #[test]
    fn uki_without_esp_rejected() {
        let bad = GOOD.replace("esp = \"128M\", ", "");
        let e = parse(&bad).unwrap_err();
        assert!(e.0.contains("target.virt.disk.esp"), "{e}");
    }

    #[test]
    fn net_parsed() {
        let m = parse(&with_fc(&format!(
            "{FC}\nnet = {{ address = \"172.16.0.2/30\", gateway = \"172.16.0.1\", dns = \"1.1.1.1\" }}"
        )))
        .unwrap();
        let n = m.target["fc"].net.clone().unwrap();
        assert_eq!(n.address, Cidr { addr: [172, 16, 0, 2].into(), prefix: 30 });
        assert_eq!(n.gateway, std::net::Ipv4Addr::new(172, 16, 0, 1));
        assert_eq!(n.dns, Some(std::net::Ipv4Addr::new(1, 1, 1, 1)));
    }

    #[test]
    fn net_only_for_direct() {
        let bad = GOOD.replace(
            "data = \"64M\" }",
            "data = \"64M\" }\nnet = { address = \"10.0.0.2/24\", gateway = \"10.0.0.1\" }",
        );
        let e = parse(&bad).unwrap_err();
        assert!(e.0.contains("target.virt.net"), "{e}");
    }

    #[test]
    fn net_bad_address_rejected() {
        for (addr, gw) in [
            ("10.0.0.2/0", "10.0.0.1"),
            ("10.0.0.2/33", "10.0.0.1"),
            ("10.0.0.2", "10.0.0.1"),
            ("1.2.3/24", "10.0.0.1"),
            ("a.b.c.d/24", "10.0.0.1"),
            ("10.0.0.2/24", "x"),
        ] {
            let m = with_fc(&format!("{FC}\nnet = {{ address = \"{addr}\", gateway = \"{gw}\" }}"));
            assert!(parse(&m).is_err(), "{addr} {gw} принят");
        }
    }
}
