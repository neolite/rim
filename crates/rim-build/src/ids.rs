//! GUID'ы выводятся из манифеста, а не из генератора случайных чисел: один манифест — один образ.

use sha2::{Digest, Sha256};

pub type Guid = [u8; 16];

pub fn derive(seed: &[u8], label: &str) -> Guid {
    let mut h = Sha256::new();
    h.update(seed);
    h.update([0u8]);
    h.update(label.as_bytes());
    let d = h.finalize();
    let mut g: Guid = d[..16].try_into().expect("sha256 длиннее 16 байт");
    g[6] = (g[6] & 0x0f) | 0x40;
    g[8] = (g[8] & 0x3f) | 0x80;
    g
}

pub fn fat_serial(seed: &[u8], label: &str) -> u32 {
    let g = derive(seed, label);
    u32::from_be_bytes([g[0], g[1], g[2], g[3]])
}

pub fn to_string(g: &Guid) -> String {
    let h: String = g.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// Разбор строки GUID. Только для констант в коде, поэтому паника на мусоре допустима.
pub fn parse(s: &str) -> Guid {
    let hex: String = s.chars().filter(|c| *c != '-').collect();
    assert_eq!(hex.len(), 32, "GUID {s}");
    let mut g = [0u8; 16];
    for (i, b) in g.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).expect("hex в GUID");
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_is_deterministic_and_label_sensitive() {
        assert_eq!(derive(b"m", "root-A"), derive(b"m", "root-A"));
        assert_ne!(derive(b"m", "root-A"), derive(b"m", "root-B"));
        assert_ne!(derive(b"m1", "root-A"), derive(b"m2", "root-A"));
    }

    #[test]
    fn derive_sets_version_and_variant() {
        let g = derive(b"x", "y");
        assert_eq!(g[6] >> 4, 4);
        assert_eq!(g[8] >> 6, 0b10);
    }

    #[test]
    fn string_roundtrip() {
        let s = "c12a7328-f81f-11d2-ba4b-00a0c93ec93b";
        assert_eq!(to_string(&parse(s)), s);
    }
}
