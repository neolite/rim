//! Protective MBR + первичная и резервная GPT. Только байты, без файлов.

use crate::ids::Guid;

pub const SECTOR: u64 = 512;
pub const ESP_TYPE: &str = "c12a7328-f81f-11d2-ba4b-00a0c93ec93b";
pub const ROOT_ARM64_TYPE: &str = "b921b045-1df0-41c3-af44-4c6f280d3fae";
pub const LINUX_FS_TYPE: &str = "0fc63daf-8483-4772-8e79-3d69d8477de4";

const ENTRIES: usize = 128;
const ENTRY_SIZE: usize = 128;
const ENTRIES_BYTES: usize = ENTRIES * ENTRY_SIZE; // 16384 = 32 сектора

#[derive(Debug, Clone)]
pub struct PartEntry {
    pub type_guid: Guid,
    pub guid: Guid,
    pub first_lba: u64,
    pub last_lba: u64,
    pub name: String,
}

/// GUID в GPT хранится «смешанно»: первые три поля little-endian.
fn on_disk(g: &Guid) -> [u8; 16] {
    let mut o = *g;
    o[0..4].reverse();
    o[4..6].reverse();
    o[6..8].reverse();
    o
}

fn entries(parts: &[PartEntry]) -> Vec<u8> {
    let mut buf = vec![0u8; ENTRIES_BYTES];
    for (i, p) in parts.iter().enumerate() {
        let e = &mut buf[i * ENTRY_SIZE..(i + 1) * ENTRY_SIZE];
        e[0..16].copy_from_slice(&on_disk(&p.type_guid));
        e[16..32].copy_from_slice(&on_disk(&p.guid));
        e[32..40].copy_from_slice(&p.first_lba.to_le_bytes());
        e[40..48].copy_from_slice(&p.last_lba.to_le_bytes());
        for (j, u) in p.name.encode_utf16().take(36).enumerate() {
            e[56 + 2 * j..58 + 2 * j].copy_from_slice(&u.to_le_bytes());
        }
    }
    buf
}

fn header(my: u64, alt: u64, entries_lba: u64, total: u64, disk_guid: &Guid, entries_crc: u32) -> [u8; 512] {
    let mut h = [0u8; 512];
    h[0..8].copy_from_slice(b"EFI PART");
    h[8..12].copy_from_slice(&0x0001_0000u32.to_le_bytes());
    h[12..16].copy_from_slice(&92u32.to_le_bytes());
    h[24..32].copy_from_slice(&my.to_le_bytes());
    h[32..40].copy_from_slice(&alt.to_le_bytes());
    h[40..48].copy_from_slice(&34u64.to_le_bytes());
    h[48..56].copy_from_slice(&(total - 34).to_le_bytes());
    h[56..72].copy_from_slice(&on_disk(disk_guid));
    h[72..80].copy_from_slice(&entries_lba.to_le_bytes());
    h[80..84].copy_from_slice(&(ENTRIES as u32).to_le_bytes());
    h[84..88].copy_from_slice(&(ENTRY_SIZE as u32).to_le_bytes());
    h[88..92].copy_from_slice(&entries_crc.to_le_bytes());
    let crc = crc32fast::hash(&h[..92]);
    h[16..20].copy_from_slice(&crc.to_le_bytes());
    h
}

pub fn build(total: u64, disk_guid: Guid, parts: &[PartEntry]) -> (Vec<u8>, Vec<u8>) {
    let ents = entries(parts);
    let ents_crc = crc32fast::hash(&ents);
    let last = total - 1;

    let mut head = vec![0u8; 34 * SECTOR as usize];
    let mbr = &mut head[446..462];
    mbr[0..8].copy_from_slice(&[0x00, 0x00, 0x02, 0x00, 0xEE, 0xFF, 0xFF, 0xFF]);
    mbr[8..12].copy_from_slice(&1u32.to_le_bytes());
    mbr[12..16].copy_from_slice(&((total - 1).min(u32::MAX as u64) as u32).to_le_bytes());
    head[510] = 0x55;
    head[511] = 0xAA;
    head[512..1024].copy_from_slice(&header(1, last, 2, total, &disk_guid, ents_crc));
    head[1024..1024 + ENTRIES_BYTES].copy_from_slice(&ents);

    let mut tail = vec![0u8; 33 * SECTOR as usize];
    tail[..ENTRIES_BYTES].copy_from_slice(&ents);
    tail[ENTRIES_BYTES..].copy_from_slice(&header(last, 1, last - 32, total, &disk_guid, ents_crc));
    (head, tail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids;

    fn sample() -> (Vec<u8>, Vec<u8>, u64) {
        let total = 10_000;
        let parts = vec![
            PartEntry { type_guid: ids::parse(ESP_TYPE), guid: ids::derive(b"t", "esp"), first_lba: 2048, last_lba: 4095, name: "esp".into() },
            PartEntry { type_guid: ids::parse(LINUX_FS_TYPE), guid: ids::derive(b"t", "data"), first_lba: 4096, last_lba: 6143, name: "data".into() },
        ];
        let (head, tail) = build(total, ids::derive(b"t", "disk"), &parts);
        (head, tail, total)
    }

    fn u32_at(b: &[u8], o: usize) -> u32 { u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) }
    fn u64_at(b: &[u8], o: usize) -> u64 { u64::from_le_bytes(b[o..o + 8].try_into().unwrap()) }

    #[test]
    fn sizes() {
        let (head, tail, _) = sample();
        assert_eq!(head.len(), 34 * 512);
        assert_eq!(tail.len(), 33 * 512);
    }

    #[test]
    fn protective_mbr() {
        let (head, _, total) = sample();
        assert_eq!(&head[510..512], &[0x55, 0xAA]);
        assert_eq!(head[446 + 4], 0xEE);
        assert_eq!(u32_at(&head, 446 + 8), 1);
        assert_eq!(u32_at(&head, 446 + 12) as u64, total - 1);
    }

    #[test]
    fn primary_header_fields_and_crc() {
        let (head, _, total) = sample();
        let h = &head[512..1024];
        assert_eq!(&h[0..8], b"EFI PART");
        assert_eq!(u32_at(h, 12), 92);
        assert_eq!(u64_at(h, 24), 1);
        assert_eq!(u64_at(h, 32), total - 1);
        assert_eq!(u64_at(h, 40), 34);
        assert_eq!(u64_at(h, 48), total - 34);
        assert_eq!(u64_at(h, 72), 2);
        let mut copy = h[..92].to_vec();
        copy[16..20].fill(0);
        assert_eq!(u32_at(h, 16), crc32fast::hash(&copy));
        assert_eq!(u32_at(h, 88), crc32fast::hash(&head[1024..1024 + 16384]));
    }

    #[test]
    fn backup_header_mirrors_primary() {
        let (head, tail, total) = sample();
        let b = &tail[32 * 512..33 * 512];
        assert_eq!(&b[0..8], b"EFI PART");
        assert_eq!(u64_at(b, 24), total - 1);
        assert_eq!(u64_at(b, 32), 1);
        assert_eq!(u64_at(b, 72), total - 33);
        assert_eq!(&tail[..16384], &head[1024..1024 + 16384]);
        let mut copy = b[..92].to_vec();
        copy[16..20].fill(0);
        assert_eq!(u32_at(b, 16), crc32fast::hash(&copy));
    }

    #[test]
    fn entry_layout_mixed_endian_guid_and_utf16_name() {
        let (head, _, _) = sample();
        let e = &head[1024..1024 + 128];
        // ESP C12A7328-F81F-11D2-...: первые три поля little-endian.
        assert_eq!(&e[0..4], &[0x28, 0x73, 0x2A, 0xC1]);
        assert_eq!(&e[4..6], &[0x1F, 0xF8]);
        assert_eq!(&e[6..8], &[0xD2, 0x11]);
        assert_eq!(&e[8..10], &[0xBA, 0x4B]);
        assert_eq!(u64_at(e, 32), 2048);
        assert_eq!(u64_at(e, 40), 4095);
        assert_eq!(&e[56..62], &[b'e', 0, b's', 0, b'p', 0]);
        assert!(head[1024 + 2 * 128..1024 + 3 * 128].iter().all(|&x| x == 0));
    }
}
