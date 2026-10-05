//! Манифест → rim.img: корень (erofs), UKI, ESP, data, GPT. Без loop-устройств.

use crate::gpt::{self, PartEntry, SECTOR};
use crate::ids;
use crate::layout;
use crate::pkg;
use crate::recipe::Kind;
use crate::rootfs::{self, Entry};
use crate::sh;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Seek, SeekFrom, Write};
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::Command;

const STUB: &str = "/usr/lib/systemd/boot/efi/linuxaa64.efi.stub";

pub fn build(root: &Path, manifest_path: &Path, target: &str, out: &Path) -> Result<String, String> {
    let bytes = fs::read(manifest_path).map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let text = String::from_utf8(bytes.clone()).map_err(|_| "манифест не UTF-8".to_string())?;
    let m = rim_manifest::parse(&text).map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let t = m.target.get(target).ok_or_else(|| format!("нет [target.{target}]"))?;
    let tc = std::env::var("RIM_TOOLCHAIN_ID").unwrap_or_else(|_| "dev".into());
    // Единственный источник — ENV тулчейна (docker/Dockerfile): он же входит в ключ кэша пакетов.
    let epoch = std::env::var("SOURCE_DATE_EPOCH").map_err(|_| "SOURCE_DATE_EPOCH не задан (запускайте через ./rim)")?;
    let epoch = epoch.as_str();

    let mut trees = Vec::new();
    for name in &m.system.packages {
        let (dir, kind) = pkg::ensure(root, name, &tc)?;
        if kind != Kind::Rootfs {
            return Err(format!("пакет {name}: kind = {kind:?}, в system.packages допустим только rootfs"));
        }
        trees.push(dir);
    }
    let (kdir, kkind) = pkg::ensure(root, &t.kernel, &tc)?;
    if kkind != Kind::Kernel {
        return Err(format!("target.{target}.kernel = {}: пакет не kind = \"kernel\"", t.kernel));
    }

    let work = root.join("out/work/image");
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).map_err(|e| e.to_string())?;

    // 1. Корень
    let rootdir = work.join("rootfs");
    for tree in &trees {
        fs::create_dir_all(&rootdir).map_err(|e| e.to_string())?;
        sh::run(Command::new("cp").arg("-a").arg(format!("{}/.", tree.display())).arg(&rootdir))?;
    }
    let script = fs::read(root.join("files/udhcpc.script")).map_err(|e| format!("files/udhcpc.script: {e}"))?;
    materialize(&rootdir, &rootfs::plan(&bytes, &script))?;
    let init_bin = PathBuf::from(std::env::var("RIM_INIT_BIN").map_err(|_| "RIM_INIT_BIN не задан (запускайте через ./rim)")?);
    fs::copy(&init_bin, rootdir.join("sbin/init")).map_err(|e| format!("{}: {e}", init_bin.display()))?;
    fs::set_permissions(rootdir.join("sbin/init"), fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    sh::run(Command::new("find").arg(&rootdir).args(["-exec", "touch", "-h", "-d", &format!("@{epoch}"), "{}", "+"]))?;

    let root_uuid = ids::to_string(&ids::derive(&bytes, "fs:root"));
    let root_img = work.join("root.erofs");
    sh::run(
        Command::new("mkfs.erofs")
            .arg(format!("-T{epoch}"))
            .arg(format!("-U{root_uuid}"))
            .arg("--all-root")
            // Без xattr: на bind-mount с macOS они не читаются, да и хостовые атрибуты в образе ни к чему.
            .arg("-x-1")
            .arg("-Enoinline_data")
            .arg(&root_img)
            .arg(&rootdir),
    )?;

    // 2. Разметка: нужен PARTUUID root-A для cmdline
    let (parts, total) = layout::plan(&t.disk);
    let part_guid = |label: &str| ids::derive(&bytes, &format!("part:{label}"));
    let cmdline = format!(
        "root=PARTUUID={} rootfstype=erofs ro rootwait console=ttyAMA0 console=tty0 panic=10",
        ids::to_string(&part_guid("root-A"))
    );

    // 3. UKI
    let uki = work.join("rim-A.efi");
    sh::run(
        Command::new("ukify")
            .arg("build")
            .arg(format!("--linux={}", kdir.join("boot/Image").display()))
            .arg(format!("--stub={STUB}"))
            .arg(format!("--cmdline={cmdline}"))
            .arg("--os-release=NAME=rim\nID=rim\nPRETTY_NAME=rim v1\n")
            .arg(format!("--output={}", uki.display())),
    )?;

    // 4. ESP
    let esp_img = work.join("esp.img");
    let esp_sectors = t.disk.esp.0 / SECTOR;
    File::create(&esp_img).and_then(|f| f.set_len(t.disk.esp.0)).map_err(|e| e.to_string())?;
    let mt = |args: &[&str]| -> Result<(), String> {
        sh::run(Command::new(args[0]).args(&args[1..]).env("SOURCE_DATE_EPOCH", epoch).env("MTOOLS_SKIP_CHECK", "1"))
    };
    let esp = esp_img.to_string_lossy().into_owned();
    let serial = format!("{:08x}", ids::fat_serial(&bytes, "fs:esp"));
    mt(&["mformat", "-i", &esp, "-F", "-v", "ESP", "-N", &serial, "-T", &esp_sectors.to_string(), "-h", "64", "-s", "32", "::"])?;
    mt(&["mmd", "-i", &esp, "::/EFI", "::/EFI/BOOT", "::/EFI/rim"])?;
    let uki_s = uki.to_string_lossy().into_owned();
    sh::run(Command::new("touch").args(["-d", &format!("@{epoch}"), &uki_s]))?;
    mt(&["mcopy", "-m", "-i", &esp, &uki_s, "::/EFI/BOOT/BOOTAA64.EFI"])?;
    mt(&["mcopy", "-m", "-i", &esp, &uki_s, "::/EFI/rim/rim-A.efi"])?;

    // 5. data (пустая ext4)
    let data_img = work.join("data.img");
    let empty = work.join("data-empty");
    fs::create_dir_all(&empty).map_err(|e| e.to_string())?;
    let data_uuid = ids::to_string(&ids::derive(&bytes, "fs:data"));
    let seed = ids::to_string(&ids::derive(&bytes, "fs:data-hash"));
    sh::run(
        Command::new("mke2fs")
            .args(["-q", "-F", "-t", "ext4", "-L", "data", "-U", &data_uuid])
            .arg("-E")
            .arg(format!("hash_seed={seed},root_owner=0:0"))
            .arg("-d")
            .arg(&empty)
            .arg(&data_img)
            .arg(format!("{}k", t.disk.data.0 / 1024))
            .env("E2FSPROGS_FAKE_TIME", epoch),
    )?;

    // 6. Склейка
    let entries: Vec<PartEntry> = parts
        .iter()
        .map(|p| PartEntry {
            type_guid: p.type_guid,
            guid: part_guid(p.label),
            first_lba: p.first_lba,
            last_lba: p.last_lba,
            name: p.label.to_string(),
        })
        .collect();
    let (head, tail) = gpt::build(total, ids::derive(&bytes, "disk"), &entries);
    if let Some(dir) = out.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = out.with_extension("img.tmp");
    let mut f = File::create(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    f.set_len(total * SECTOR).map_err(|e| e.to_string())?;
    let mut put = |lba: u64, data: &[u8]| -> Result<(), String> {
        f.seek(SeekFrom::Start(lba * SECTOR)).and_then(|_| f.write_all(data)).map_err(|e| e.to_string())
    };
    put(0, &head)?;
    put(total - 33, &tail)?;
    for (p, img) in parts.iter().zip([Some(&esp_img), Some(&root_img), None, Some(&data_img)]) {
        let Some(img) = img else { continue };
        let data = fs::read(img).map_err(|e| format!("{}: {e}", img.display()))?;
        let cap = (p.last_lba - p.first_lba + 1) * SECTOR;
        if data.len() as u64 > cap {
            return Err(format!("{}: {} байт не влезает в раздел {} ({} байт)", img.display(), data.len(), p.label, cap));
        }
        put(p.first_lba, &data)?;
    }
    drop(f);
    fs::rename(&tmp, out).map_err(|e| e.to_string())?;

    let digest: String =
        Sha256::digest(fs::read(out).map_err(|e| e.to_string())?).iter().map(|b| format!("{b:02x}")).collect();
    Ok(digest)
}

fn materialize(rootdir: &Path, plan: &[Entry]) -> Result<(), String> {
    for e in plan {
        let p = rootdir.join(e.path());
        match e {
            Entry::Dir(_) => fs::create_dir_all(&p).map_err(|er| format!("{}: {er}", p.display()))?,
            Entry::File { data, mode, .. } => {
                fs::write(&p, data).map_err(|er| format!("{}: {er}", p.display()))?;
                fs::set_permissions(&p, fs::Permissions::from_mode(*mode)).map_err(|er| er.to_string())?;
            }
            Entry::Symlink { target, .. } => {
                let _ = fs::remove_file(&p);
                symlink(target, &p).map_err(|er| format!("{}: {er}", p.display()))?;
            }
        }
    }
    Ok(())
}
