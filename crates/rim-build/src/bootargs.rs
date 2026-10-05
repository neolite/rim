//! Командная строка ядра и конфиг Firecracker — чистые функции.

use rim_manifest::{Arch, Boot, Net};
use std::net::Ipv4Addr;

pub fn netmask(prefix: u8) -> Ipv4Addr {
    Ipv4Addr::from(u32::MAX.checked_shl(32 - u32::from(prefix)).unwrap_or(0))
}

pub fn cmdline(boot: Boot, arch: Arch, net: Option<&Net>, root_partuuid: &str) -> String {
    let base = format!("root=PARTUUID={root_partuuid} rootfstype=erofs ro rootwait");
    let mut s = match (boot, arch) {
        // tty0 — графическая консоль VPS (VNC).
        (Boot::UefiUki, _) => format!("{base} console=ttyAMA0 console=tty0 panic=10"),
        // ttyS0 — 8250 у Firecracker, ttyAMA0 — PL011 в локальном QEMU; несуществующую ядро пропустит.
        (Boot::Direct, Arch::Aarch64) => format!("{base} console=ttyAMA0 console=ttyS0 panic=10"),
        // У Firecracker на x86 гость не может выключить VM, процесс завершает только сброс через
        // i8042 (reboot=k); rim.poweroff=reboot велит init выполнять poweroff как перезагрузку.
        (Boot::Direct, Arch::X86_64) => format!("{base} console=ttyS0 reboot=k panic=10 rim.poweroff=reboot"),
    };
    if let (Boot::Direct, Some(n)) = (boot, net) {
        // ip=<client>:<server>:<gw>:<netmask>:<hostname>:<device>:<autoconf>:<dns0>
        s += &format!(" ip={}::{}:{}::eth0:off", n.address.addr, n.gateway, netmask(n.address.prefix));
        if let Some(dns) = n.dns {
            s += &format!(":{dns}");
        }
    }
    s
}

fn json_str(s: &str) -> String {
    let mut out = String::from('"');
    for c in s.chars() {
        match c {
            '"' => out += "\\\"",
            '\\' => out += "\\\\",
            c if (c as u32) < 0x20 => out += &format!("\\u{:04x}", c as u32),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Конфиг для `firecracker --no-api --config-file`. Пути — как переданы (относительные
/// работают при запуске из каталога с образом).
pub fn firecracker_json(kernel: &str, disk: &str, boot_args: &str) -> String {
    format!(
        r#"{{
  "boot-source": {{
    "kernel_image_path": {},
    "boot_args": {}
  }},
  "drives": [
    {{
      "drive_id": "disk",
      "path_on_host": {},
      "is_root_device": false,
      "is_read_only": false
    }}
  ],
  "network-interfaces": [
    {{
      "iface_id": "eth0",
      "host_dev_name": "tap0"
    }}
  ],
  "machine-config": {{
    "vcpu_count": 1,
    "mem_size_mib": 256
  }}
}}
"#,
        json_str(kernel),
        json_str(boot_args),
        json_str(disk)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rim_manifest::{Arch, Cidr};

    const UUID: &str = "ef486a02-af19-4c99-b83b-c6b7a2c354fe";

    fn net(dns: Option<Ipv4Addr>) -> Net {
        Net {
            address: Cidr { addr: Ipv4Addr::new(10, 0, 2, 15), prefix: 24 },
            gateway: Ipv4Addr::new(10, 0, 2, 2),
            dns,
        }
    }

    #[test]
    fn netmask_from_prefix() {
        assert_eq!(netmask(30), Ipv4Addr::new(255, 255, 255, 252));
        assert_eq!(netmask(24), Ipv4Addr::new(255, 255, 255, 0));
        assert_eq!(netmask(32), Ipv4Addr::new(255, 255, 255, 255));
        assert_eq!(netmask(1), Ipv4Addr::new(128, 0, 0, 0));
    }

    #[test]
    fn cmdline_uefi_unchanged() {
        assert_eq!(
            cmdline(Boot::UefiUki, Arch::Aarch64, None, UUID),
            format!("root=PARTUUID={UUID} rootfstype=erofs ro rootwait console=ttyAMA0 console=tty0 panic=10")
        );
    }

    #[test]
    fn cmdline_direct_with_net() {
        assert_eq!(
            cmdline(Boot::Direct, Arch::Aarch64, Some(&net(Some(Ipv4Addr::new(10, 0, 2, 3)))), UUID),
            format!(
                "root=PARTUUID={UUID} rootfstype=erofs ro rootwait console=ttyAMA0 console=ttyS0 panic=10 \
                 ip=10.0.2.15::10.0.2.2:255.255.255.0::eth0:off:10.0.2.3"
            )
        );
    }

    #[test]
    fn cmdline_direct_without_dns_and_without_net() {
        assert!(cmdline(Boot::Direct, Arch::Aarch64, Some(&net(None)), UUID).ends_with(" ip=10.0.2.15::10.0.2.2:255.255.255.0::eth0:off"));
        assert!(cmdline(Boot::Direct, Arch::Aarch64, None, UUID).ends_with("console=ttyS0 panic=10"));
    }

    #[test]
    fn firecracker_json_exact() {
        let j = firecracker_json("rim-fc.Image", "rim-fc.img", "root=PARTUUID=x ro \"q\"");
        assert_eq!(
            j,
            r#"{
  "boot-source": {
    "kernel_image_path": "rim-fc.Image",
    "boot_args": "root=PARTUUID=x ro \"q\""
  },
  "drives": [
    {
      "drive_id": "disk",
      "path_on_host": "rim-fc.img",
      "is_root_device": false,
      "is_read_only": false
    }
  ],
  "network-interfaces": [
    {
      "iface_id": "eth0",
      "host_dev_name": "tap0"
    }
  ],
  "machine-config": {
    "vcpu_count": 1,
    "mem_size_mib": 256
  }
}
"#
        );
    }

    #[test]
    fn cmdline_direct_x86() {
        assert_eq!(
            cmdline(Boot::Direct, Arch::X86_64, Some(&net(Some(Ipv4Addr::new(10, 0, 2, 3)))), UUID),
            format!(
                "root=PARTUUID={UUID} rootfstype=erofs ro rootwait console=ttyS0 reboot=k panic=10 rim.poweroff=reboot \
                 ip=10.0.2.15::10.0.2.2:255.255.255.0::eth0:off:10.0.2.3"
            )
        );
    }
}
