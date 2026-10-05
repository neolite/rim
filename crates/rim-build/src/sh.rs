use std::process::Command;

pub fn run(cmd: &mut Command) -> Result<(), String> {
    let shown = format!("{cmd:?}");
    eprintln!("+ {shown}");
    let st = cmd.status().map_err(|e| format!("{shown}: {e}"))?;
    if st.success() { Ok(()) } else { Err(format!("{shown}: {st}")) }
}
