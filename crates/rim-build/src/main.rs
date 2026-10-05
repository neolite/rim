mod gpt;
mod ids;
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
        _ => Err("использование: rim pkg <имя> | rim build [--manifest F] [--target T] [--out F]".into()),
    };
    if let Err(e) = res {
        eprintln!("rim: {e}");
        std::process::exit(1);
    }
}
