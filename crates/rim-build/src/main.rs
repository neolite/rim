mod gpt;
mod ids;
mod image;
mod layout;
mod pkg;
mod recipe;
mod rootfs;
mod sh;

use std::path::PathBuf;

fn toolchain_id() -> String {
    std::env::var("RIM_TOOLCHAIN_ID").unwrap_or_else(|_| "dev".into())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Абсолютный: build.sh делает cd "$SRC", и относительный $OUT уехал бы внутрь исходников.
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/rim"));
    let res = match args.first().map(String::as_str) {
        Some("pkg") if args.len() == 2 => {
            pkg::ensure(&root, &args[1], &toolchain_id()).map(|(p, _)| println!("{}", p.display()))
        }
        Some("build") => {
            let mut manifest = PathBuf::from("rim.toml");
            let mut target = "virt".to_string();
            let mut out = PathBuf::from("out/rim.img");
            let mut it = args.iter().skip(1);
            let mut bad = None;
            while let Some(a) = it.next() {
                match (a.as_str(), it.next()) {
                    ("--manifest", Some(v)) => manifest = v.into(),
                    ("--target", Some(v)) => target = v.clone(),
                    ("--out", Some(v)) => out = v.into(),
                    _ => bad = Some(a.clone()),
                }
            }
            match bad {
                Some(a) => Err(format!("неизвестный аргумент {a}")),
                None => image::build(&root, &root.join(&manifest), &target, &root.join(&out))
                    .map(|h| println!("{}  {}", h, out.display())),
            }
        }
        _ => Err("использование: rim pkg <имя> | rim build [--manifest F] [--target T] [--out F]".into()),
    };
    if let Err(e) = res {
        eprintln!("rim: {e}");
        std::process::exit(1);
    }
}
