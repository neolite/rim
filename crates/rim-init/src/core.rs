//! Логика PID 1 без системных вызовов: событие → новое состояние + действия.

use rim_manifest::Restart;
use std::collections::BTreeMap;

pub const BACKOFF_MIN: u64 = 1;
pub const BACKOFF_MAX: u64 = 30;
pub const STABLE_AFTER: u64 = 60;
pub const GRACE: u64 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownKind { Poweroff, Reboot, Halt }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sig { Term, Kill }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SvcState {
    Starting,
    Running { pid: i32, since: u64 },
    Backoff { until: u64 },
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Boot { now: u64 },
    Spawned { name: String, pid: i32, now: u64 },
    SpawnFailed { name: String, now: u64 },
    Exited { pid: i32, status: i32, now: u64 },
    Tick { now: u64 },
    ShutdownRequested { kind: ShutdownKind, now: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Spawn { name: String },
    Signal { pid: i32, sig: Sig },
    Log { line: String },
    Ready,
    Finish { kind: ShutdownKind },
}

struct Svc {
    restart: Restart,
    state: SvcState,
    next_delay: u64,
}

struct Shutdown {
    kind: ShutdownKind,
    deadline: u64,
    killed: bool,
    finished: bool,
}

pub struct Supervisor {
    svcs: BTreeMap<String, Svc>,
    ready: bool,
    shutdown: Option<Shutdown>,
}

impl Supervisor {
    pub fn new(specs: impl IntoIterator<Item = (String, Restart)>) -> Self {
        let svcs = specs
            .into_iter()
            .map(|(n, restart)| (n, Svc { restart, state: SvcState::Stopped, next_delay: BACKOFF_MIN }))
            .collect();
        Supervisor { svcs, ready: false, shutdown: None }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn state(&self, name: &str) -> Option<&SvcState> {
        self.svcs.get(name).map(|s| &s.state)
    }

    pub fn step(&mut self, ev: Event) -> Vec<Action> {
        let mut out = Vec::new();
        match ev {
            Event::Boot { .. } => {
                for (name, svc) in self.svcs.iter_mut() {
                    svc.state = SvcState::Starting;
                    out.push(Action::Spawn { name: name.clone() });
                }
                self.check_ready(&mut out);
            }
            Event::Spawned { name, pid, now } => {
                if let Some(svc) = self.svcs.get_mut(&name) {
                    svc.state = SvcState::Running { pid, since: now };
                    if self.shutdown.is_some() {
                        out.push(Action::Signal { pid, sig: Sig::Term });
                    }
                }
                self.check_ready(&mut out);
            }
            Event::SpawnFailed { name, now } => {
                out.push(Action::Log { line: format!("[{name}] не удалось запустить") });
                if self.shutdown.is_some() {
                    if let Some(svc) = self.svcs.get_mut(&name) {
                        svc.state = SvcState::Stopped;
                    }
                    self.check_finish(&mut out);
                } else {
                    self.schedule_restart(&name, now, 0);
                }
                self.check_ready(&mut out);
            }
            Event::Exited { pid, status, now } => {
                let Some(name) = self.name_by_pid(pid) else { return out };
                out.push(Action::Log { line: format!("[{name}] завершился, статус {status}") });
                if self.shutdown.is_some() {
                    if let Some(svc) = self.svcs.get_mut(&name) {
                        svc.state = SvcState::Stopped;
                    }
                    self.check_finish(&mut out);
                    return out;
                }
                let (restart, since) = match self.svcs.get(&name) {
                    Some(Svc { restart, state: SvcState::Running { since, .. }, .. }) => (*restart, *since),
                    _ => return out,
                };
                match restart {
                    Restart::Never => {
                        if let Some(svc) = self.svcs.get_mut(&name) {
                            svc.state = SvcState::Stopped;
                        }
                    }
                    Restart::Always => self.schedule_restart(&name, now, now.saturating_sub(since)),
                }
            }
            Event::Tick { now } => {
                if let Some(sd) = self.shutdown.as_mut() {
                    if sd.finished {
                        return out;
                    }
                    if !sd.killed && now >= sd.deadline {
                        sd.killed = true;
                        for svc in self.svcs.values() {
                            if let SvcState::Running { pid, .. } = svc.state {
                                out.push(Action::Signal { pid, sig: Sig::Kill });
                            }
                        }
                    } else if sd.killed && now >= sd.deadline + GRACE {
                        sd.finished = true;
                        out.push(Action::Finish { kind: sd.kind });
                    }
                    return out;
                }
                for (name, svc) in self.svcs.iter_mut() {
                    if let SvcState::Backoff { until } = svc.state {
                        if now >= until {
                            svc.state = SvcState::Starting;
                            out.push(Action::Spawn { name: name.clone() });
                        }
                    }
                }
            }
            Event::ShutdownRequested { kind, now } => {
                if self.shutdown.is_some() {
                    return out;
                }
                self.shutdown = Some(Shutdown { kind, deadline: now + GRACE, killed: false, finished: false });
                for svc in self.svcs.values_mut() {
                    match svc.state {
                        SvcState::Running { pid, .. } => out.push(Action::Signal { pid, sig: Sig::Term }),
                        SvcState::Starting => {}
                        _ => svc.state = SvcState::Stopped,
                    }
                }
                self.check_finish(&mut out);
            }
        }
        out
    }

    fn name_by_pid(&self, pid: i32) -> Option<String> {
        self.svcs.iter().find_map(|(n, s)| match s.state {
            SvcState::Running { pid: p, .. } if p == pid => Some(n.clone()),
            _ => None,
        })
    }

    fn schedule_restart(&mut self, name: &str, now: u64, ran: u64) {
        let Some(svc) = self.svcs.get_mut(name) else { return };
        if ran >= STABLE_AFTER {
            svc.next_delay = BACKOFF_MIN;
        }
        svc.state = SvcState::Backoff { until: now + svc.next_delay };
        svc.next_delay = (svc.next_delay * 2).min(BACKOFF_MAX);
    }

    fn check_ready(&mut self, out: &mut Vec<Action>) {
        if !self.ready && self.svcs.values().all(|s| s.state != SvcState::Starting) {
            self.ready = true;
            out.push(Action::Ready);
        }
    }

    fn check_finish(&mut self, out: &mut Vec<Action>) {
        let all_down = self
            .svcs
            .values()
            .all(|s| !matches!(s.state, SvcState::Running { .. } | SvcState::Starting));
        if let Some(sd) = self.shutdown.as_mut() {
            if all_down && !sd.finished {
                sd.finished = true;
                out.push(Action::Finish { kind: sd.kind });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rim_manifest::Restart;

    fn sup(specs: &[(&str, Restart)]) -> Supervisor {
        Supervisor::new(specs.iter().map(|(n, r)| (n.to_string(), *r)))
    }
    fn spawn(n: &str) -> Action { Action::Spawn { name: n.into() } }
    fn spawned(n: &str, pid: i32, now: u64) -> Event { Event::Spawned { name: n.into(), pid, now } }
    fn exited(pid: i32, now: u64) -> Event { Event::Exited { pid, status: 1, now } }
    fn spawns(a: &[Action]) -> Vec<&Action> { a.iter().filter(|x| matches!(x, Action::Spawn { .. })).collect() }

    #[test]
    fn boot_spawns_all_services_in_name_order() {
        let mut s = sup(&[("sshd", Restart::Always), ("network", Restart::Always)]);
        assert_eq!(s.step(Event::Boot { now: 0 }), vec![spawn("network"), spawn("sshd")]);
    }

    #[test]
    fn ready_emitted_once_after_all_started() {
        let mut s = sup(&[("a", Restart::Always), ("b", Restart::Always)]);
        s.step(Event::Boot { now: 0 });
        assert!(!s.step(spawned("a", 10, 0)).contains(&Action::Ready));
        assert!(s.step(spawned("b", 11, 0)).contains(&Action::Ready));
        s.step(exited(10, 1));
        let again = s.step(Event::Tick { now: 2 });
        let again2 = s.step(spawned("a", 12, 2));
        assert!(!again.contains(&Action::Ready) && !again2.contains(&Action::Ready));
    }

    #[test]
    fn crashed_service_respawns_after_backoff() {
        let mut s = sup(&[("a", Restart::Always)]);
        s.step(Event::Boot { now: 0 });
        s.step(spawned("a", 10, 0));
        s.step(exited(10, 5));
        assert_eq!(s.state("a"), Some(&SvcState::Backoff { until: 6 }));
        assert!(spawns(&s.step(Event::Tick { now: 5 })).is_empty());
        assert_eq!(spawns(&s.step(Event::Tick { now: 6 })), vec![&spawn("a")]);
    }

    #[test]
    fn backoff_doubles_up_to_thirty() {
        let mut s = sup(&[("a", Restart::Always)]);
        s.step(Event::Boot { now: 0 });
        let mut now = 0;
        let mut waits = vec![];
        for pid in 1..=7 {
            s.step(spawned("a", pid, now));
            s.step(exited(pid, now));
            let SvcState::Backoff { until } = *s.state("a").unwrap() else { panic!() };
            waits.push(until - now);
            now = until;
            s.step(Event::Tick { now });
        }
        assert_eq!(waits, vec![1, 2, 4, 8, 16, 30, 30]);
    }

    #[test]
    fn stable_run_resets_backoff() {
        let mut s = sup(&[("a", Restart::Always)]);
        s.step(Event::Boot { now: 0 });
        s.step(spawned("a", 1, 0));
        s.step(exited(1, 0)); // ждём 1
        s.step(Event::Tick { now: 1 });
        s.step(spawned("a", 2, 1));
        s.step(exited(2, 1)); // ждём 2
        s.step(Event::Tick { now: 3 });
        s.step(spawned("a", 3, 3));
        s.step(exited(3, 63)); // проработал 60 с
        assert_eq!(s.state("a"), Some(&SvcState::Backoff { until: 64 }));
    }

    #[test]
    fn never_restart_service_stops() {
        let mut s = sup(&[("once", Restart::Never)]);
        s.step(Event::Boot { now: 0 });
        s.step(spawned("once", 5, 0));
        s.step(exited(5, 1));
        assert_eq!(s.state("once"), Some(&SvcState::Stopped));
        assert!(spawns(&s.step(Event::Tick { now: 100 })).is_empty());
    }

    #[test]
    fn spawn_failure_goes_to_backoff_without_hot_loop() {
        let mut s = sup(&[("a", Restart::Always)]);
        s.step(Event::Boot { now: 0 });
        let acts = s.step(Event::SpawnFailed { name: "a".into(), now: 0 });
        assert!(spawns(&acts).is_empty());
        assert!(acts.iter().any(|a| matches!(a, Action::Log { .. })));
        assert_eq!(s.state("a"), Some(&SvcState::Backoff { until: 1 }));
        assert!(acts.contains(&Action::Ready), "сбой запуска не должен блокировать готовность");
    }

    #[test]
    fn unknown_pid_exit_is_ignored() {
        let mut s = sup(&[("a", Restart::Always)]);
        s.step(Event::Boot { now: 0 });
        s.step(spawned("a", 10, 0));
        assert!(s.step(exited(999, 1)).is_empty());
        assert_eq!(s.state("a"), Some(&SvcState::Running { pid: 10, since: 0 }));
    }

    #[test]
    fn shutdown_terms_running_and_finishes_when_all_exit() {
        let mut s = sup(&[("a", Restart::Always), ("b", Restart::Always)]);
        s.step(Event::Boot { now: 0 });
        s.step(spawned("a", 10, 0));
        s.step(spawned("b", 11, 0));
        let acts = s.step(Event::ShutdownRequested { kind: ShutdownKind::Poweroff, now: 5 });
        assert!(acts.contains(&Action::Signal { pid: 10, sig: Sig::Term }));
        assert!(acts.contains(&Action::Signal { pid: 11, sig: Sig::Term }));
        assert!(!s.step(exited(10, 6)).contains(&Action::Finish { kind: ShutdownKind::Poweroff }));
        assert!(s.step(exited(11, 6)).contains(&Action::Finish { kind: ShutdownKind::Poweroff }));
    }

    #[test]
    fn shutdown_kills_after_grace_and_gives_up_after_second_grace() {
        let mut s = sup(&[("a", Restart::Always)]);
        s.step(Event::Boot { now: 0 });
        s.step(spawned("a", 10, 0));
        s.step(Event::ShutdownRequested { kind: ShutdownKind::Reboot, now: 0 });
        assert!(s.step(Event::Tick { now: 4 }).is_empty());
        assert_eq!(s.step(Event::Tick { now: 5 }), vec![Action::Signal { pid: 10, sig: Sig::Kill }]);
        assert!(s.step(Event::Tick { now: 10 }).contains(&Action::Finish { kind: ShutdownKind::Reboot }));
    }

    #[test]
    fn no_respawn_during_shutdown() {
        let mut s = sup(&[("a", Restart::Always)]);
        s.step(Event::Boot { now: 0 });
        s.step(spawned("a", 10, 0));
        s.step(exited(10, 1)); // в backoff
        let acts = s.step(Event::ShutdownRequested { kind: ShutdownKind::Halt, now: 1 });
        assert!(acts.contains(&Action::Finish { kind: ShutdownKind::Halt }));
        assert!(spawns(&s.step(Event::Tick { now: 100 })).is_empty());
    }

    #[test]
    fn no_services_ready_and_finish_immediately() {
        let mut s = sup(&[]);
        assert_eq!(s.step(Event::Boot { now: 0 }), vec![Action::Ready]);
        assert_eq!(
            s.step(Event::ShutdownRequested { kind: ShutdownKind::Poweroff, now: 0 }),
            vec![Action::Finish { kind: ShutdownKind::Poweroff }]
        );
    }

    #[test]
    fn second_shutdown_request_is_ignored() {
        let mut s = sup(&[("a", Restart::Always)]);
        s.step(Event::Boot { now: 0 });
        s.step(spawned("a", 10, 0));
        s.step(Event::ShutdownRequested { kind: ShutdownKind::Poweroff, now: 0 });
        assert!(s.step(Event::ShutdownRequested { kind: ShutdownKind::Reboot, now: 1 }).is_empty());
    }
}
