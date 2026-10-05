//! Всё, что трогает ядро: монтирование, сигналы, процессы, перезагрузка.
//! Логика решений — в core::Supervisor; здесь только перевод событий и действий.

use crate::core::{Action, Event, ShutdownKind, Sig, Supervisor};
use crate::setup;
use nix::mount::{MsFlags, mount, umount};
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use nix::sys::reboot::{RebootMode, reboot};
use nix::sys::signal::{SigSet, SigmaskHow, Signal, kill, sigprocmask};
use nix::sys::signalfd::{SfdFlags, SignalFd};
use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
use nix::unistd::{Pid, sethostname, setsid, sync};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Instant;

const MANIFEST: &str = "/etc/rim/system.toml";

struct Console(Mutex<Vec<File>>);

impl Console {
    fn open() -> Console {
        let active = fs::read_to_string("/sys/class/tty/console/active").unwrap_or_default();
        let files = setup::console_paths(&active)
            .iter()
            .filter_map(|p| OpenOptions::new().write(true).open(p).ok())
            .collect();
        Console(Mutex::new(files))
    }
    fn line(&self, s: &str) {
        if let Ok(mut files) = self.0.lock() {
            for f in files.iter_mut() {
                let _ = writeln!(f, "{s}");
            }
        }
    }
}

pub fn run() -> ! {
    let mount_errors = mount_early();
    let console = Arc::new(Console::open());
    console.line("rim: init");
    for e in &mount_errors {
        console.line(&format!("rim: {e}"));
    }
    match prepare(&console) {
        Ok(manifest) => supervise(manifest, console),
        Err(e) => emergency(&console, &e),
    }
}

/// Каждое монтирование независимо: отказ одного не должен оставить init без /dev и консоли.
fn mount_early() -> Vec<String> {
    let nodev = MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC;
    let mut errors = Vec::new();
    let mut m = |src: &str, dst: &str, fs: &str, flags: MsFlags, data: Option<&str>| {
        if let Err(e) = mount(Some(src), dst, Some(fs), flags, data) {
            errors.push(format!("mount {fs} {dst}: {e}"));
        }
    };
    m("proc", "/proc", "proc", nodev, None);
    m("sysfs", "/sys", "sysfs", nodev, None);
    m("devtmpfs", "/dev", "devtmpfs", MsFlags::MS_NOSUID, Some("mode=0755"));
    let _ = fs::create_dir_all("/dev/pts");
    m("devpts", "/dev/pts", "devpts", MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC, Some("mode=620,ptmxmode=666"));
    m("tmpfs", "/run", "tmpfs", MsFlags::MS_NOSUID | MsFlags::MS_NODEV, Some("mode=0755"));
    m("tmpfs", "/tmp", "tmpfs", MsFlags::MS_NOSUID | MsFlags::MS_NODEV, Some("mode=1777"));
    errors
}

fn prepare(console: &Console) -> Result<rim_manifest::Manifest, String> {
    let text = fs::read_to_string(MANIFEST).map_err(|e| format!("{MANIFEST}: {e}"))?;
    let m = rim_manifest::parse(&text).map_err(|e| format!("{MANIFEST}: {e}"))?;

    let mut uevents = Vec::new();
    for entry in fs::read_dir("/sys/class/block").map_err(|e| format!("/sys/class/block: {e}"))? {
        let Ok(entry) = entry else { continue };
        let dev = entry.file_name().to_string_lossy().into_owned();
        if let Ok(ue) = fs::read_to_string(entry.path().join("uevent")) {
            uevents.push((dev, ue));
        }
    }
    let data = setup::find_partition(&uevents, "data").ok_or("раздел с PARTNAME=data не найден")?;
    mount(Some(data.as_str()), "/data", Some("ext4"), MsFlags::MS_NOSUID | MsFlags::MS_NODEV, None::<&str>)
        .map_err(|e| format!("mount {data} /data: {e}"))?;
    console.line(&format!("rim: /data = {data}"));

    for d in ["/data/ssh", "/data/log", "/data/root/.ssh"] {
        fs::create_dir_all(d).map_err(|e| format!("{d}: {e}"))?;
    }
    fs::set_permissions("/data/root/.ssh", fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;

    let host_key = "/data/ssh/host_ed25519";
    if fs::metadata(host_key).is_err() {
        console.line("rim: первая загрузка, создаю ключ хоста");
        let st = Command::new("/bin/dropbearkey")
            .args(["-t", "ed25519", "-f", host_key])
            .stdout(Stdio::null())
            .status()
            .map_err(|e| format!("dropbearkey: {e}"))?;
        if !st.success() {
            return Err(format!("dropbearkey: {st}"));
        }
    }
    let ak = "/data/root/.ssh/authorized_keys";
    fs::write(ak, setup::authorized_keys(&m.system.ssh_authorized_keys)).map_err(|e| format!("{ak}: {e}"))?;
    fs::set_permissions(ak, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;

    sethostname(&m.system.hostname).map_err(|e| format!("sethostname: {e}"))?;
    let _ = Command::new("/sbin/ip").args(["link", "set", "lo", "up"]).status();
    Ok(m)
}

fn emergency(console: &Console, err: &str) -> ! {
    console.line(&format!("rim: ОШИБКА: {err}"));
    console.line("rim: аварийный shell на консоли (exit — перезапустить shell)");
    loop {
        let shell = (|| -> std::io::Result<std::process::ExitStatus> {
            let c = || OpenOptions::new().read(true).write(true).open("/dev/console");
            Command::new("/bin/sh").stdin(c()?).stdout(c()?).stderr(c()?).status()
        })();
        if shell.is_err() {
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
        while let Ok(WaitStatus::Exited(..) | WaitStatus::Signaled(..)) =
            waitpid(Pid::from_raw(-1), Some(WaitPidFlag::WNOHANG))
        {}
    }
}

fn supervise(m: rim_manifest::Manifest, console: Arc<Console>) -> ! {
    let mut mask = SigSet::empty();
    for s in [Signal::SIGCHLD, Signal::SIGTERM, Signal::SIGUSR1, Signal::SIGUSR2, Signal::SIGINT] {
        mask.add(s);
    }
    if let Err(e) = sigprocmask(SigmaskHow::SIG_BLOCK, Some(&mask), None) {
        emergency(&console, &format!("sigprocmask: {e}"));
    }
    let sfd = match SignalFd::with_flags(&mask, SfdFlags::SFD_NONBLOCK | SfdFlags::SFD_CLOEXEC) {
        Ok(s) => s,
        Err(e) => emergency(&console, &format!("signalfd: {e}")),
    };

    let start = Instant::now();
    let now = || start.elapsed().as_secs();
    let mut sup = Supervisor::new(m.service.iter().map(|(n, s)| (n.clone(), s.restart)));
    let mut queue = vec![Event::Boot { now: now() }];

    loop {
        while let Some(ev) = queue.pop() {
            for action in sup.step(ev) {
                match action {
                    Action::Spawn { name } => queue.push(spawn(&m, &name, &console, now())),
                    Action::Signal { pid, sig } => {
                        let s = if sig == Sig::Term { Signal::SIGTERM } else { Signal::SIGKILL };
                        let _ = kill(Pid::from_raw(pid), s);
                    }
                    Action::Log { line } => console.line(&format!("rim: {line}")),
                    Action::Ready => console.line("rim: ready"),
                    Action::Finish { kind } => finish(kind, &console),
                }
            }
        }

        {
            let mut fds = [PollFd::new(sfd.as_fd(), PollFlags::POLLIN)];
            let _ = poll(&mut fds, PollTimeout::from(1000u16));
        }
        while let Ok(Some(info)) = sfd.read_signal() {
            match Signal::try_from(info.ssi_signo as i32) {
                Ok(Signal::SIGCHLD) => loop {
                    match waitpid(Pid::from_raw(-1), Some(WaitPidFlag::WNOHANG)) {
                        Ok(WaitStatus::Exited(pid, code)) => {
                            queue.push(Event::Exited { pid: pid.as_raw(), status: code, now: now() })
                        }
                        Ok(WaitStatus::Signaled(pid, sig, _)) => {
                            queue.push(Event::Exited { pid: pid.as_raw(), status: 128 + sig as i32, now: now() })
                        }
                        Ok(WaitStatus::StillAlive) | Err(_) => break,
                        Ok(_) => {}
                    }
                },
                Ok(Signal::SIGUSR2) => queue.push(Event::ShutdownRequested { kind: ShutdownKind::Poweroff, now: now() }),
                Ok(Signal::SIGUSR1) => queue.push(Event::ShutdownRequested { kind: ShutdownKind::Halt, now: now() }),
                Ok(Signal::SIGTERM | Signal::SIGINT) => {
                    queue.push(Event::ShutdownRequested { kind: ShutdownKind::Reboot, now: now() })
                }
                _ => {}
            }
        }
        queue.insert(0, Event::Tick { now: now() });
    }
}

fn spawn(m: &rim_manifest::Manifest, name: &str, console: &Arc<Console>, now: u64) -> Event {
    let Some(svc) = m.service.get(name) else {
        return Event::SpawnFailed { name: name.into(), now };
    };
    let mut cmd = Command::new(&svc.exec[0]);
    cmd.args(&svc.exec[1..])
        .env_clear()
        .env("PATH", "/bin:/sbin")
        .env("HOME", "/data/root")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // SAFETY: между fork и exec вызываем только async-signal-safe sigprocmask и setsid.
    unsafe {
        cmd.pre_exec(|| {
            sigprocmask(SigmaskHow::SIG_SETMASK, Some(&SigSet::empty()), None)?;
            setsid()?;
            Ok(())
        });
    }
    match cmd.spawn() {
        Ok(mut child) => {
            let log = OpenOptions::new().create(true).append(true).open(format!("/data/log/{name}.log")).ok();
            let log = Arc::new(Mutex::new(log));
            if let Some(out) = child.stdout.take() {
                pump(out, name, console.clone(), log.clone());
            }
            if let Some(err) = child.stderr.take() {
                pump(err, name, console.clone(), log);
            }
            Event::Spawned { name: name.into(), pid: child.id() as i32, now }
        }
        Err(e) => {
            console.line(&format!("rim: [{name}] {}: {e}", svc.exec[0]));
            Event::SpawnFailed { name: name.into(), now }
        }
    }
}

fn pump(src: impl Read + Send + 'static, name: &str, console: Arc<Console>, log: Arc<Mutex<Option<File>>>) {
    let name = name.to_string();
    std::thread::spawn(move || {
        for line in BufReader::new(src).lines().map_while(Result::ok) {
            console.line(&format!("[{name}] {line}"));
            if let Ok(mut guard) = log.lock() {
                if let Some(f) = guard.as_mut() {
                    let _ = writeln!(f, "{line}");
                }
            }
        }
    });
}

fn finish(kind: ShutdownKind, console: &Console) -> ! {
    console.line("rim: выключение");
    sync();
    let _ = umount("/data");
    sync();
    let mode = match kind {
        ShutdownKind::Poweroff => RebootMode::RB_POWER_OFF,
        ShutdownKind::Reboot => RebootMode::RB_AUTOBOOT,
        ShutdownKind::Halt => RebootMode::RB_HALT_SYSTEM,
    };
    let _ = reboot(mode);
    loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
    }
}
