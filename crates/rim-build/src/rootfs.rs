//! Что лежит в корне помимо пакетов. Чистый план: материализует его image.rs.

#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    Dir(String),
    File { path: String, data: Vec<u8>, mode: u32 },
    Symlink { path: String, target: String },
}

impl Entry {
    pub fn path(&self) -> &str {
        match self {
            Entry::Dir(p) => p,
            Entry::File { path, .. } | Entry::Symlink { path, .. } => path,
        }
    }
}

pub fn plan(manifest: &[u8], udhcpc_script: &[u8]) -> Vec<Entry> {
    let mut v: Vec<Entry> = [
        "bin", "sbin", "etc", "etc/rim", "proc", "sys", "dev", "run", "tmp", "data", "var",
    ]
    .iter()
    .map(|d| Entry::Dir(d.to_string()))
    .collect();
    let file = |p: &str, d: &[u8], mode| Entry::File { path: p.into(), data: d.to_vec(), mode };
    let link = |p: &str, t: &str| Entry::Symlink { path: p.into(), target: t.into() };
    v.extend([
        file("etc/passwd", b"root:x:0:0:root:/data/root:/bin/sh\n", 0o644),
        file("etc/group", b"root:x:0:\n", 0o644),
        file("etc/shells", b"/bin/sh\n", 0o644),
        file("etc/rim/system.toml", manifest, 0o644),
        file("etc/rim/udhcpc.script", udhcpc_script, 0o755),
        link("etc/resolv.conf", "/run/resolv.conf"),
        link("var/run", "/run"),
    ]);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find<'a>(p: &'a [Entry], path: &str) -> Option<&'a Entry> {
        p.iter().find(|e| e.path() == path)
    }

    #[test]
    fn plan_contains_mountpoints_and_config() {
        let p = plan(b"MANIFEST", b"SCRIPT");
        for d in ["proc", "sys", "dev", "run", "tmp", "data", "bin", "sbin", "etc/rim"] {
            assert!(matches!(find(&p, d), Some(Entry::Dir(_))), "нет каталога {d}");
        }
        assert!(matches!(find(&p, "etc/rim/system.toml"), Some(Entry::File { data, .. }) if data == b"MANIFEST"));
        assert!(matches!(find(&p, "etc/rim/udhcpc.script"), Some(Entry::File { mode: 0o755, .. })));
    }

    #[test]
    fn root_home_on_data_and_runtime_links() {
        let p = plan(b"", b"");
        let Some(Entry::File { data, .. }) = find(&p, "etc/passwd") else { panic!() };
        assert_eq!(String::from_utf8_lossy(data), "root:x:0:0:root:/data/root:/bin/sh\n");
        assert_eq!(find(&p, "etc/resolv.conf"), Some(&Entry::Symlink { path: "etc/resolv.conf".into(), target: "/run/resolv.conf".into() }));
        assert_eq!(find(&p, "var/run"), Some(&Entry::Symlink { path: "var/run".into(), target: "/run".into() }));
    }

    #[test]
    fn parents_come_before_children() {
        let p = plan(b"", b"");
        let pos = |x: &str| p.iter().position(|e| e.path() == x).unwrap();
        assert!(pos("etc") < pos("etc/rim"));
        assert!(pos("etc/rim") < pos("etc/rim/system.toml"));
        assert!(pos("var") < pos("var/run"));
    }
}
