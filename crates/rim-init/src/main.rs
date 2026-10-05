// Вне Linux core и setup живут только ради тестов: их потребитель (effects) не собирается.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

mod core;
mod setup;
#[cfg(target_os = "linux")]
mod effects;

#[cfg(target_os = "linux")]
fn main() {
    if std::process::id() != 1 {
        eprintln!("rim-init должен работать как PID 1");
        std::process::exit(1);
    }
    effects::run()
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("rim-init работает только как PID 1 под Linux");
    std::process::exit(1);
}
