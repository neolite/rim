//! recipe.toml пакета и ключ кэша: хэш всего каталога рецепта + тулчейна.

use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Rootfs,
    Kernel,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub kind: Kind,
}

impl Recipe {
    pub fn parse(text: &str) -> Result<Recipe, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }
}

pub fn cache_key(files: &[(String, Vec<u8>)], toolchain_id: &str) -> String {
    let mut sorted: Vec<_> = files.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut h = Sha256::new();
    h.update(b"rim-cache-v1\0");
    h.update(toolchain_id.as_bytes());
    h.update([0u8]);
    for (path, data) in sorted {
        h.update((path.len() as u64).to_le_bytes());
        h.update(path.as_bytes());
        h.update((data.len() as u64).to_le_bytes());
        h.update(data);
    }
    h.finalize()[..16].iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_recipe() {
        let r = Recipe::parse(
            "version = \"1.37.0\"\nurl = \"https://x/b.tar.bz2\"\nsha256 = \"ab\"\nkind = \"rootfs\"\n",
        )
        .unwrap();
        assert_eq!(r.kind, Kind::Rootfs);
        assert_eq!(r.version, "1.37.0");
    }

    #[test]
    fn recipe_rejects_unknown_field() {
        assert!(Recipe::parse("version=\"1\"\nurl=\"u\"\nsha256=\"s\"\nkind=\"kernel\"\nextra=1\n").is_err());
    }

    fn f(p: &str, d: &str) -> (String, Vec<u8>) { (p.into(), d.as_bytes().to_vec()) }

    #[test]
    fn cache_key_is_order_independent() {
        let a = cache_key(&[f("build.sh", "x"), f("recipe.toml", "y")], "tc1");
        let b = cache_key(&[f("recipe.toml", "y"), f("build.sh", "x")], "tc1");
        assert_eq!(a, b);
        assert_eq!(a.len(), 32);
    }

    #[test]
    fn cache_key_changes_with_content_name_or_toolchain() {
        let base = cache_key(&[f("build.sh", "x")], "tc1");
        assert_ne!(base, cache_key(&[f("build.sh", "y")], "tc1"));
        assert_ne!(base, cache_key(&[f("build2.sh", "x")], "tc1"));
        assert_ne!(base, cache_key(&[f("build.sh", "x")], "tc2"));
        // граница «путь/содержимое» не должна склеиваться
        assert_ne!(cache_key(&[f("ab", "c")], "t"), cache_key(&[f("a", "bc")], "t"));
    }
}
