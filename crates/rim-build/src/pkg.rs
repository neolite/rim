//! Пакет = recipe.toml + build.sh. Результат кэшируется по хэшу рецепта и тулчейна.

use crate::recipe::{Kind, Recipe, cache_key};
use crate::sh;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn read_dir_files(dir: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).map_err(|e| format!("{}: {e}", d.display()))? {
            let p = e.map_err(|e| e.to_string())?.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let rel = p.strip_prefix(dir).expect("внутри dir").to_string_lossy().into_owned();
                out.push((rel, fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?));
            }
        }
    }
    Ok(out)
}

fn sha256_file(p: &Path) -> Result<String, String> {
    let data = fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(Sha256::digest(&data).iter().map(|b| format!("{b:02x}")).collect())
}

fn fetch(root: &Path, r: &Recipe) -> Result<PathBuf, String> {
    let dir = root.join("out/cache/src");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dst = dir.join(&r.sha256);
    if dst.exists() {
        return Ok(dst);
    }
    let tmp = dir.join(format!("{}.part", r.sha256));
    sh::run(Command::new("curl").args(["-fsSL", "--retry", "5", "--retry-all-errors", "-o"]).arg(&tmp).arg(&r.url))?;
    let got = sha256_file(&tmp)?;
    if got != r.sha256 {
        let _ = fs::remove_file(&tmp);
        return Err(format!("{}: sha256 {got}, ожидался {}", r.url, r.sha256));
    }
    fs::rename(&tmp, &dst).map_err(|e| e.to_string())?;
    Ok(dst)
}

pub fn ensure(root: &Path, name: &str, toolchain_id: &str) -> Result<(PathBuf, Kind), String> {
    let rdir = root.join("pkgs").join(name);
    let text = fs::read_to_string(rdir.join("recipe.toml"))
        .map_err(|e| format!("пакет {name}: нет pkgs/{name}/recipe.toml ({e})"))?;
    let recipe = Recipe::parse(&text).map_err(|e| format!("pkgs/{name}/recipe.toml: {e}"))?;
    let key = cache_key(&read_dir_files(&rdir)?, toolchain_id);
    let out = root.join(format!("out/cache/{key}-{name}"));
    if out.exists() {
        return Ok((out, recipe.kind));
    }

    let src_archive = fetch(root, &recipe)?;
    let work = root.join(format!("out/work/src-{name}"));
    let tmp_out = root.join(format!("out/cache/{key}-{name}.tmp"));
    for d in [&work, &tmp_out] {
        let _ = fs::remove_dir_all(d);
        fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    sh::run(Command::new("tar").arg("xf").arg(&src_archive).arg("-C").arg(&work).arg("--strip-components=1"))?;
    let jobs = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    sh::run(
        Command::new("sh")
            .arg(rdir.join("build.sh"))
            .env("SRC", &work)
            .env("OUT", &tmp_out)
            .env("RECIPE", &rdir)
            .env("JOBS", jobs.to_string()),
    )
    .map_err(|e| format!("сборка пакета {name}: {e}"))?;
    let empty = fs::read_dir(&tmp_out).map(|mut d| d.next().is_none()).unwrap_or(true);
    if empty {
        return Err(format!("сборка пакета {name}: build.sh ничего не положил в $OUT"));
    }
    fs::rename(&tmp_out, &out).map_err(|e| e.to_string())?;
    Ok((out, recipe.kind))
}
