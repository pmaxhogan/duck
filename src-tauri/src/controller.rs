//! The engine controller: owns the background thread that polls Voicemeeter and
//! drives the duckers, and exposes a small handle plus a live status snapshot to
//! the rest of the app.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::engine::config::{validate_strips, Config};
use crate::engine::layout::{kind_name, strip_count, SILENCE_DB};
use crate::engine::{color_strips_for, read_levels, Ducker, State, VoicemeeterApi};

pub const STATUS_EVENT: &str = "engine:status";

#[derive(Debug, Clone, Serialize, Default)]
pub struct SourceLevel {
    pub strip: u32,
    pub db: f32,
    pub over: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct RuleStatus {
    pub ducked: bool,
    pub sources: Vec<SourceLevel>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct EngineStatus {
    pub connected: bool,
    pub kind: i32,
    pub kind_name: String,
    pub strip_count: u32,
    pub enabled: bool,
    pub rules: Vec<RuleStatus>,
    /// Local-only, for friendlier slider labels. Never leaves this machine.
    pub labels: HashMap<u32, String>,
    pub error: Option<String>,
}

enum EngineCmd {
    Reload(Config),
    SetEnabled(bool),
    Shutdown,
}

pub struct EngineControl {
    tx: Mutex<Sender<EngineCmd>>,
    status: Arc<Mutex<EngineStatus>>,
    enabled: Arc<AtomicBool>,
    preview: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
}

impl EngineControl {
    fn send(&self, cmd: EngineCmd) {
        let _ = self.tx.lock().unwrap().send(cmd);
    }
    pub fn reload(&self, cfg: Config) {
        self.send(EngineCmd::Reload(cfg));
    }
    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::SeqCst);
        self.send(EngineCmd::SetEnabled(on));
    }
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }
    /// Hold-to-preview: force every rule to duck now, ignoring source levels and
    /// the Enabled switch. The engine reads this each poll.
    pub fn set_preview(&self, on: bool) {
        self.preview.store(on, Ordering::SeqCst);
    }
    pub fn status(&self) -> EngineStatus {
        self.status.lock().unwrap().clone()
    }
    /// Ask the thread to restore Voicemeeter and log out, and wait briefly for it.
    pub fn shutdown(&self) {
        self.send(EngineCmd::Shutdown);
        let deadline = Instant::now() + Duration::from_millis(1500);
        while !self.done.load(Ordering::SeqCst) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
    }
}

pub fn spawn(app: AppHandle, config: Config, enabled: bool) -> EngineControl {
    let (tx, rx) = channel();
    let status = Arc::new(Mutex::new(EngineStatus {
        enabled,
        ..Default::default()
    }));
    let enabled_flag = Arc::new(AtomicBool::new(enabled));
    let preview_flag = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));

    let st = status.clone();
    let en = enabled_flag.clone();
    let pv = preview_flag.clone();
    let dn = done.clone();
    thread::spawn(move || run_loop(app, config, rx, st, en, pv, dn));

    EngineControl {
        tx: Mutex::new(tx),
        status,
        enabled: enabled_flag,
        preview: preview_flag,
        done,
    }
}

fn poll_duration(cfg: &Config) -> Duration {
    Duration::from_millis(cfg.poll_ms.max(10.0) as u64)
}

fn build_duckers(cfg: &Config, kind: i32) -> (Vec<Ducker>, Option<String>) {
    if let Err(e) = validate_strips(cfg, kind) {
        return (Vec::new(), Some(e.to_string()));
    }
    let duckers = cfg
        .rules
        .iter()
        .map(|r| Ducker::new(r.clone(), color_strips_for(r, kind)))
        .collect();
    (duckers, None)
}

fn read_labels(api: &VoicemeeterApi, kind: i32) -> HashMap<u32, String> {
    (0..strip_count(kind)).map(|s| (s, api.strip_label(s))).collect()
}

fn restore_all(duckers: &mut [Ducker], api: &VoicemeeterApi) {
    for d in duckers {
        d.restore(api);
    }
}

fn publish(app: &AppHandle, status: &Arc<Mutex<EngineStatus>>, snapshot: EngineStatus) {
    *status.lock().unwrap() = snapshot.clone();
    let _ = app.emit(STATUS_EVENT, &snapshot);
}

fn run_loop(
    app: AppHandle,
    mut cfg: Config,
    rx: std::sync::mpsc::Receiver<EngineCmd>,
    status: Arc<Mutex<EngineStatus>>,
    enabled: Arc<AtomicBool>,
    preview: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
) {
    let mut api: Option<VoicemeeterApi> = None;
    let mut kind: i32 = 0;
    let mut duckers: Vec<Ducker> = Vec::new();
    let mut build_error: Option<String> = None;
    let mut labels: HashMap<u32, String> = HashMap::new();
    let mut poll = poll_duration(&cfg);
    let start = Instant::now();

    loop {
        // Drain any pending control commands.
        loop {
            match rx.try_recv() {
                Ok(EngineCmd::Shutdown) => {
                    if let Some(a) = &api {
                        restore_all(&mut duckers, a);
                        a.logout();
                    }
                    done.store(true, Ordering::SeqCst);
                    return;
                }
                Ok(EngineCmd::Reload(new_cfg)) => {
                    if let Some(a) = &api {
                        restore_all(&mut duckers, a);
                        // restore_all queues ASYNC gain/color writes. Flush them
                        // before the fresh duckers can capture a baseline this same
                        // loop - otherwise a baseline read taken microseconds later
                        // returns the still-ducked values (reads only refresh after
                        // is_dirty), pinning the un-duck level and the low-pass.
                        a.flush();
                    }
                    cfg = new_cfg;
                    poll = poll_duration(&cfg);
                    if kind >= 1 {
                        let (d, e) = build_duckers(&cfg, kind);
                        duckers = d;
                        build_error = e;
                    }
                }
                Ok(EngineCmd::SetEnabled(on)) => {
                    if !on {
                        if let Some(a) = &api {
                            restore_all(&mut duckers, a);
                        }
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if let Some(a) = &api {
                        restore_all(&mut duckers, a);
                        a.logout();
                    }
                    done.store(true, Ordering::SeqCst);
                    return;
                }
            }
        }

        // (Re)connect if needed.
        if api.is_none() {
            match VoicemeeterApi::load() {
                Ok(a) => {
                    if a.login() >= 0 {
                        for _ in 0..5 {
                            a.is_dirty();
                            thread::sleep(Duration::from_millis(20));
                        }
                        let k = a.kind();
                        if k >= 1 {
                            kind = k;
                            let (d, e) = build_duckers(&cfg, kind);
                            duckers = d;
                            build_error = e;
                            labels = read_labels(&a, kind);
                            api = Some(a);
                        }
                    }
                }
                Err(e) => build_error = Some(e),
            }
            if api.is_none() {
                let snapshot = EngineStatus {
                    connected: false,
                    enabled: enabled.load(Ordering::SeqCst),
                    error: build_error.clone(),
                    ..Default::default()
                };
                publish(&app, &status, snapshot);
                thread::sleep(Duration::from_millis(1000));
                continue;
            }
        }

        // Detect Voicemeeter going away.
        if api.as_ref().unwrap().is_dirty() < 0 {
            for d in &mut duckers {
                d.forget();
            }
            api = None;
            kind = 0;
            let snapshot = EngineStatus {
                connected: false,
                enabled: enabled.load(Ordering::SeqCst),
                ..Default::default()
            };
            publish(&app, &status, snapshot);
            continue;
        }

        let a = api.as_ref().unwrap();
        let now = start.elapsed().as_millis() as f64;
        let on = enabled.load(Ordering::SeqCst);
        let force = preview.load(Ordering::SeqCst);
        let mut rule_status = Vec::with_capacity(duckers.len());

        for d in &mut duckers {
            let levels = read_levels(a, kind, d.rule());
            let sources = d
                .rule()
                .sources
                .iter()
                .map(|s| {
                    let db = levels.get(&s.strip).and_then(|o| *o).unwrap_or(SILENCE_DB);
                    SourceLevel {
                        strip: s.strip,
                        db,
                        over: db >= s.threshold_db,
                    }
                })
                .collect();
            // Preview forces a duck even while the Enabled switch is off, so the
            // user can audition settings without arming the engine.
            if on || force {
                d.tick_with(a, now, &levels, force);
            } else if d.state() != State::Idle {
                d.restore(a);
            }
            rule_status.push(RuleStatus {
                ducked: d.state() != State::Idle,
                sources,
            });
        }

        let snapshot = EngineStatus {
            connected: true,
            kind,
            kind_name: kind_name(kind).to_string(),
            strip_count: strip_count(kind),
            enabled: on,
            rules: rule_status,
            labels: labels.clone(),
            error: build_error.clone(),
        };
        publish(&app, &status, snapshot);

        thread::sleep(poll);
    }
}
