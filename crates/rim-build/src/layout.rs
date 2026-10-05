//! Разметка диска: [esp,] root-A, root-B, data — выровнено по 1 MiB.

use crate::gpt::{ESP_TYPE, LINUX_FS_TYPE, ROOT_ARM64_TYPE, SECTOR};
use crate::ids::{self, Guid};
use rim_manifest::Disk;

pub const ALIGN: u64 = 2048;

#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub label: &'static str,
    pub type_guid: Guid,
    pub first_lba: u64,
    pub last_lba: u64,
}

pub fn plan(disk: &Disk) -> (Vec<Part>, u64) {
    let spec = [
        ("esp", ESP_TYPE, disk.esp.map_or(0, |s| s.0)),
        ("root-A", ROOT_ARM64_TYPE, disk.root.0),
        ("root-B", ROOT_ARM64_TYPE, disk.root.0),
        ("data", LINUX_FS_TYPE, disk.data.0),
    ];
    let mut next = ALIGN;
    let mut parts = Vec::new();
    for (label, ty, bytes) in spec {
        if label == "esp" && disk.esp.is_none() {
            continue; // boot = "direct": ядро даёт гипервизор, ESP не нужен
        }
        let sectors = bytes / SECTOR;
        parts.push(Part { label, type_guid: ids::parse(ty), first_lba: next, last_lba: next + sectors - 1 });
        next += sectors.div_ceil(ALIGN) * ALIGN;
    }
    let total = parts.last().expect("хотя бы 3 раздела").last_lba + 1 + ALIGN;
    (parts, total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rim_manifest::{Disk, Size};

    #[test]
    fn partitions_aligned_and_in_order() {
        let disk = Disk { esp: Some(Size(128 << 20)), root: Size(256 << 20), data: Size(64 << 20) };
        let (parts, total) = plan(&disk);
        let labels: Vec<_> = parts.iter().map(|p| p.label).collect();
        assert_eq!(labels, ["esp", "root-A", "root-B", "data"]);
        assert_eq!(parts[0].first_lba, 2048);
        assert_eq!(parts[0].last_lba, 2048 + (128 << 11) - 1);
        assert_eq!(parts[1].first_lba, parts[0].last_lba + 1);
        for p in &parts {
            assert_eq!(p.first_lba % 2048, 0, "{} не выровнен", p.label);
        }
        assert_eq!(total, parts[3].last_lba + 1 + 2048);
    }

    #[test]
    fn without_esp_starts_with_root_a() {
        let disk = Disk { esp: None, root: Size(64 << 20), data: Size(256 << 20) };
        let (parts, total) = plan(&disk);
        let labels: Vec<_> = parts.iter().map(|p| p.label).collect();
        assert_eq!(labels, ["root-A", "root-B", "data"]);
        assert_eq!(parts[0].first_lba, 2048);
        assert_eq!(parts[0].last_lba, 2048 + (64 << 11) - 1);
        assert_eq!(parts[2].last_lba - parts[2].first_lba + 1, 256 << 11);
        assert_eq!(total, parts[2].last_lba + 1 + 2048);
    }
}
