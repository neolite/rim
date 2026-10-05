mod core;

fn main() {
    eprintln!("rim-init работает только как PID 1 под Linux");
    std::process::exit(1);
}
