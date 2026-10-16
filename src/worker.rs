use crate::{
    config::{Settings, wildcard},
    control::{ChannelState, StableResponse, cool_complete, ramp},
    hardware::{Hardware, Sample},
    recording::{Context as RecordingContext, Producer},
};
use serde_json::json;
use std::{
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub sample: Option<Sample>,
    pub targets: Vec<i32>,
    pub outputs: Vec<i32>,
    pub levels: Vec<usize>,
    pub active: Option<String>,
    pub mode: String,
    pub error: Option<String>,
    pub force: bool,
    pub description: String,
}
impl Snapshot {
    pub fn same_display(&self, other: &Self) -> bool {
        self.sample.as_ref().map(|s| s.at) == other.sample.as_ref().map(|s| s.at)
            && self.targets == other.targets
            && self.outputs == other.outputs
            && self.levels == other.levels
            && self.active == other.active
            && self.mode == other.mode
            && self.error == other.error
            && self.force == other.force
            && self.description == other.description
    }
}

pub enum Command {
    Settings(Settings),
    Select(Option<String>),
    Takeover(bool),
    Force(bool),
    AutoSwitch(bool),
    Suspend,
    Resume,
    Retry,
    Recording(Option<Producer>),
    Exit,
}
pub enum Event {
    Sample(Option<Sample>),
    Snapshot(Snapshot),
    CoolingDone,
    Stopped(std::result::Result<(), String>),
}
pub struct Worker {
    pub tx: Sender<Command>,
    pub rx: Receiver<Event>,
    join: Option<thread::JoinHandle<()>>,
}
impl Worker {
    pub fn start(settings: Settings, demo: bool) -> Self {
        let (tx, rx) = mpsc::channel();
        let (etx, erx) = mpsc::channel();
        let join = thread::spawn(move || {
            let initial = open(demo, &settings.compatibility);
            run(settings, initial, rx, etx)
        });
        Self {
            tx,
            rx: erx,
            join: Some(join),
        }
    }
    pub fn join(&mut self) {
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}
fn open(
    demo: bool,
    compatibility: &crate::config::CompatibilityConfig,
) -> std::result::Result<Box<dyn Hardware>, String> {
    if demo {
        Ok(Box::new(crate::hardware::Demo::default()))
    } else {
        Ok(Box::new(crate::hardware::Insyde::open(compatibility)?))
    }
}
fn run(
    mut settings: Settings,
    initial: std::result::Result<Box<dyn Hardware>, String>,
    rx: Receiver<Command>,
    tx: Sender<Event>,
) {
    let mut hardware = None;
    let mut error = None;
    let demo = initial
        .as_ref()
        .is_ok_and(|h| h.description().contains("演示模式"));
    match initial {
        Ok(h) => hardware = Some(h),
        Err(e) => error = Some(e),
    }
    let mut active: Option<String> = None;
    let mut cfg = settings.global.clone();
    let mut states = [ChannelState::default(); 2];
    let mut responses = [StableResponse::default(), StableResponse::default()];
    let mut force = false;
    let mut owned = false;
    let mut suspended = false;
    let mut fault = false;
    let mut sample: Option<Sample> = None;
    let mut targets = vec![];
    let mut output = vec![];
    let mut last_written = vec![];
    let mut pending = None;
    let mut next_sample = Instant::now();
    let mut next_scan = Instant::now();
    let mut next_restore = Instant::now();
    let mut last_emit = Instant::now() - Duration::from_secs(1);
    let mut recording: Option<Producer> = None;
    loop {
        if recording.as_ref().is_some_and(|r| !r.active()) {
            recording = None;
        }
        let cmd = match pending
            .take()
            .map_or_else(|| rx.recv_timeout(Duration::from_millis(100)), Ok)
        {
            Ok(c) => Some(c),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(_) => Some(Command::Exit),
        };
        let mut selected = false;
        let mut changed = false;
        if let Some(cmd) = cmd {
            match cmd {
                Command::Exit => {
                    if let Some(r) = &recording {
                        r.event("restore_start", json!({"reason":"exit","applicable":owned}));
                    }
                    let result = if owned {
                        hardware.as_mut().map_or(Ok(()), |h| h.automatic())
                    } else {
                        Ok(())
                    };
                    if let Some(r) = &recording {
                        r.event(
                            "restore_result",
                            json!({"reason":"exit","applicable":owned,"result":result}),
                        );
                    }
                    let _ = tx.send(Event::Stopped(result));
                    return;
                }
                Command::Settings(s) => {
                    settings = s;
                    if active
                        .as_ref()
                        .is_some_and(|a| !settings.presets.iter().any(|p| &p.name == a))
                    {
                        active = None;
                    }
                    selected = true;
                }
                Command::Select(a) => {
                    active = a;
                    selected = true;
                }
                Command::Takeover(v) => {
                    cfg.take_over = v;
                    fault = false;
                    changed = true;
                }
                Command::Force(v) => {
                    force = v;
                    fault = false;
                    changed = true;
                }
                Command::AutoSwitch(v) => {
                    settings.auto_switch = v;
                    next_scan = Instant::now();
                }
                Command::Suspend => {
                    if let Some(r) = &recording {
                        r.event("suspend", json!({"owned":owned}));
                    }
                    suspended = true;
                    sample = None;
                    if owned {
                        if let Some(r) = &recording {
                            r.event("restore_start", json!({"reason":"suspend"}));
                        }
                        let result = hardware.as_mut().unwrap().automatic();
                        if let Some(r) = &recording {
                            r.event(
                                "restore_result",
                                json!({"reason":"suspend","result":result}),
                            );
                        }
                        match result {
                            Ok(()) => owned = false,
                            Err(e) => error = Some(e),
                        }
                    }
                }
                Command::Resume => {
                    if let Some(r) = &recording {
                        r.event("resume", json!({}));
                    }
                    suspended = false;
                    changed = true;
                    sample = None;
                }
                Command::Retry => {
                    if !owned {
                        match open(demo, &settings.compatibility) {
                            Ok(h) => {
                                hardware = Some(h);
                                error = None;
                                fault = false;
                                changed = true
                            }
                            Err(e) => error = Some(e),
                        }
                    }
                }
                Command::Recording(producer) => {
                    recording = producer;
                }
            }
        }
        if settings.auto_switch && Instant::now() >= next_scan && !suspended {
            next_scan = Instant::now() + Duration::from_millis(500);
            match crate::platform::processes() {
                Ok(names) => {
                    let a = settings
                        .presets
                        .iter()
                        .find(|p| names.iter().any(|n| wildcard(&p.process_pattern, n)))
                        .map(|p| p.name.clone());
                    if a != active {
                        active = a;
                        selected = true;
                    }
                }
                Err(e) => error = Some(e),
            }
        }
        if selected {
            let desired = active
                .as_ref()
                .and_then(|a| settings.presets.iter().find(|p| &p.name == a))
                .map(|p| p.control.clone())
                .unwrap_or_else(|| settings.global.clone());
            if desired != cfg {
                cfg = desired;
                changed = true;
                fault = false;
            }
        }
        if changed {
            states = [ChannelState::default(); 2];
            responses = [StableResponse::default(), StableResponse::default()];
            next_sample = Instant::now();
            output.clear();
            last_written.clear();
        }
        if let Some(r) = recording.as_mut().filter(|r| r.active()) {
            r.context(RecordingContext {
                control: cfg.clone(),
                active_preset: active.clone(),
                mode: if suspended {
                    "suspended"
                } else if fault || hardware.is_none() {
                    "fault"
                } else if force {
                    "forced"
                } else if cfg.take_over && responses.iter().any(StableResponse::boosted) {
                    "protection"
                } else if cfg.take_over {
                    "software"
                } else {
                    "firmware"
                }
                .into(),
                hardware: hardware
                    .as_ref()
                    .map(|h| h.description())
                    .unwrap_or_default(),
                simulated: demo,
                curve_duties: targets.clone(),
                requests: if owned { last_written.clone() } else { vec![] },
            });
        }
        if !suspended && hardware.is_none() && Instant::now() >= next_sample {
            next_sample = Instant::now() + Duration::from_secs_f64(cfg.update_interval);
            let _ = tx.send(Event::Sample(None));
            if let Some(r) = recording.as_mut() {
                r.sample_error(error.as_deref().unwrap_or("hardware unavailable"));
            }
        }
        if !suspended && let Some(h) = hardware.as_mut() {
            if Instant::now() >= next_sample {
                next_sample = Instant::now() + Duration::from_secs_f64(cfg.update_interval);
                match h.sample() {
                    Ok(s) => {
                        // A sample jump is retried once before it can drive the next control decision.
                        let jump = sample.as_ref().is_some_and(|old| {
                            old.channels
                                .iter()
                                .zip(&s.channels)
                                .any(|(a, b)| (a.temperature - b.temperature).abs() > 30)
                        });
                        let result = if jump {
                            if let Some(r) = recording.as_mut() {
                                r.sample(&s, "verification_required");
                            }
                            thread::sleep(Duration::from_secs(1));
                            h.sample()
                        } else {
                            Ok(s)
                        };
                        match result {
                            Ok(s) => {
                                if force
                                    && cool_complete(
                                        &s.channels
                                            .iter()
                                            .map(|c| c.temperature)
                                            .collect::<Vec<_>>(),
                                        cfg.force_temp,
                                    )
                                {
                                    force = false;
                                    let _ = tx.send(Event::CoolingDone);
                                }
                                targets = s
                                    .channels
                                    .iter()
                                    .take(2)
                                    .enumerate()
                                    .map(|(i, c)| {
                                        if force {
                                            95
                                        } else {
                                            let idx = i.min(1);
                                            let curve = if idx == 0 {
                                                &cfg.cpu_curve
                                            } else {
                                                &cfg.gpu_curve
                                            };
                                            let temperature = if cfg.stable_response {
                                                responses[idx].temperature(c.temperature, s.at)
                                            } else {
                                                c.temperature
                                            };
                                            states[idx].evaluate(curve, temperature, &cfg)
                                        }
                                    })
                                    .collect();
                                if s.channels.len() == 3 {
                                    targets.push(targets[1]);
                                }
                                if let Some(r) = recording.as_mut() {
                                    r.context(RecordingContext {
                                        control: cfg.clone(),
                                        active_preset: active.clone(),
                                        mode: if fault {
                                            "fault"
                                        } else if force {
                                            "forced"
                                        } else if cfg.take_over
                                            && responses.iter().any(StableResponse::boosted)
                                        {
                                            "protection"
                                        } else if cfg.take_over {
                                            "software"
                                        } else {
                                            "firmware"
                                        }
                                        .into(),
                                        hardware: h.description(),
                                        simulated: demo,
                                        curve_duties: targets.clone(),
                                        requests: if owned { last_written.clone() } else { vec![] },
                                    });
                                    r.sample(&s, "accepted");
                                }
                                if output.len() != targets.len() {
                                    output = s.channels.iter().map(|c| c.duty).collect();
                                }
                                // Readback, rather than only the last target, detects external CC changes.
                                if s.channels.iter().map(|c| c.duty).ne(output.iter().copied()) {
                                    last_written.clear();
                                }
                                sample = Some(s);
                                if !fault {
                                    error = None;
                                }
                            }
                            Err(e) => {
                                if let Some(r) = recording.as_mut() {
                                    r.sample_error(&e);
                                }
                                error = Some(e);
                                sample = None;
                                fault = true;
                            }
                        }
                    }
                    Err(e) => {
                        if let Some(r) = recording.as_mut() {
                            r.sample_error(&e);
                        }
                        error = Some(e);
                        sample = None;
                        fault = true;
                    }
                }
                let _ = tx.send(Event::Sample(sample.clone()));
            }
            // Process newly queued commands before any actuator operation, including restore.
            if let Ok(c) = rx.try_recv() {
                pending = Some(c);
                continue;
            }
            let stale = sample.as_ref().is_none_or(|s| {
                s.at.elapsed() > Duration::from_secs_f64(cfg.update_interval + 3.0)
            });
            if (fault || stale || (!cfg.take_over && !force))
                && owned
                && Instant::now() >= next_restore
            {
                next_restore = Instant::now() + Duration::from_secs(2);
                if let Some(r) = &recording {
                    r.event("restore_start", json!({"reason":"fault_stale_or_cancel"}));
                }
                let result = h.automatic();
                if let Some(r) = &recording {
                    r.event(
                        "restore_result",
                        json!({"reason":"fault_stale_or_cancel","result":result}),
                    );
                }
                match result {
                    Ok(()) => {
                        owned = false;
                        last_written.clear()
                    }
                    Err(e) => {
                        error = Some(e);
                        fault = true;
                    }
                }
            }
            if !fault && !stale && (cfg.take_over || force) {
                for (i, t) in targets.iter().enumerate() {
                    output[i] = if cfg.stable_response && !force {
                        if i == 2 {
                            output[1]
                        } else {
                            responses[i].duty(
                                output[i],
                                *t,
                                sample.as_ref().unwrap().channels[i].temperature,
                                i,
                                Instant::now(),
                            )
                        }
                    } else if cfg.soft_control && !force {
                        ramp(output[i], *t)
                    } else {
                        *t
                    };
                }
                if output != last_written {
                    owned = true;
                    if let Some(r) = &recording {
                        r.event(
                            "write_start",
                            json!({"requested_duties":output,"curve_duties":targets}),
                        );
                    }
                    let result = h.write(&output);
                    if let Some(r) = &recording {
                        r.event(
                            "write_result",
                            json!({"requested_duties":output,"result":result}),
                        );
                    }
                    match result {
                        Ok(()) => last_written = output.clone(),
                        Err(e) => {
                            error = Some(e);
                            fault = true;
                            if let Some(r) = &recording {
                                r.event("restore_start", json!({"reason":"write_failure"}));
                            }
                            let result = h.automatic();
                            if let Some(r) = &recording {
                                r.event(
                                    "restore_result",
                                    json!({"reason":"write_failure","result":result}),
                                );
                            }
                            if let Err(e) = result {
                                error =
                                    Some(format!("{}；{e}", error.as_deref().unwrap_or_default()))
                            } else {
                                owned = false;
                            }
                        }
                    }
                }
            }
        }
        if last_emit.elapsed() >= Duration::from_millis(100) {
            last_emit = Instant::now();
            let mode = if suspended {
                "休眠暂停"
            } else if hardware.is_none() || fault {
                "不可用"
            } else if force {
                "强制冷却 95%"
            } else if cfg.take_over && responses.iter().any(StableResponse::boosted) {
                "高温保护 · 冷却后恢复曲线"
            } else if cfg.take_over {
                "软件曲线"
            } else {
                "固件自动"
            };
            let _ = tx.send(Event::Snapshot(Snapshot {
                sample: sample.clone(),
                targets: targets.clone(),
                outputs: if owned { output.clone() } else { vec![] },
                levels: states.iter().map(|s| s.level + 1).collect(),
                active: active.clone(),
                mode: mode.into(),
                error: error.clone(),
                force,
                description: hardware
                    .as_ref()
                    .map(|h| h.description())
                    .unwrap_or_default(),
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::Reading;
    use std::sync::{Arc, Mutex};
    #[derive(Default)]
    struct State {
        writes: Vec<Vec<i32>>,
        restores: usize,
        temps: Vec<i32>,
        duties: Vec<i32>,
        fail: bool,
        samples: usize,
        restore_failure: bool,
    }
    struct Mock(Arc<Mutex<State>>);
    impl Hardware for Mock {
        fn sample(&mut self) -> Result<Sample, String> {
            let mut s = self.0.lock().unwrap();
            s.samples += 1;
            if s.fail {
                return Err("sample unavailable".into());
            }
            Ok(Sample {
                at: Instant::now(),
                channels: s
                    .temps
                    .iter()
                    .zip(&s.duties)
                    .map(|(t, d)| Reading {
                        temperature: *t,
                        duty: *d,
                        rpm: Some(2000),
                    })
                    .collect(),
            })
        }
        fn write(&mut self, d: &[i32]) -> Result<(), String> {
            let mut s = self.0.lock().unwrap();
            s.duties = d.to_vec();
            s.writes.push(d.to_vec());
            Ok(())
        }
        fn automatic(&mut self) -> Result<(), String> {
            let mut state = self.0.lock().unwrap();
            state.restores += 1;
            if state.restore_failure {
                Err("restore unavailable".into())
            } else {
                Ok(())
            }
        }
        fn description(&self) -> String {
            "test".into()
        }
    }
    fn start(
        mut settings: Settings,
        temps: Vec<i32>,
    ) -> (
        Arc<Mutex<State>>,
        Sender<Command>,
        Receiver<Event>,
        thread::JoinHandle<()>,
    ) {
        settings.global.update_interval = 1.0;
        let state = Arc::new(Mutex::new(State {
            duties: vec![30; temps.len()],
            temps,
            ..State::default()
        }));
        let h = Box::new(Mock(state.clone()));
        let (tx, rx) = mpsc::channel();
        let (etx, erx) = mpsc::channel();
        let join = thread::spawn(move || run(settings, Ok(h), rx, etx));
        (state, tx, erx, join)
    }
    fn until(rx: &Receiver<Event>, predicate: impl Fn(&Event) -> bool) {
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            if let Ok(e) = rx.recv_timeout(Duration::from_millis(200))
                && predicate(&e)
            {
                return;
            }
        }
        panic!("worker condition timed out")
    }
    fn stop(tx: Sender<Command>, rx: Receiver<Event>, join: thread::JoinHandle<()>) {
        tx.send(Command::Exit).unwrap();
        until(&rx, |e| matches!(e, Event::Stopped(Ok(()))));
        join.join().unwrap();
    }
    #[test]
    fn sampling_events_include_failures_but_not_status_refreshes() {
        let (state, tx, rx, j) = start(Settings::default(), vec![50, 60]);
        until(&rx, |e| matches!(e, Event::Sample(Some(_))));
        state.lock().unwrap().fail = true;
        let deadline = Instant::now() + Duration::from_millis(350);
        let mut snapshots = 0;
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Event::Sample(_)) => panic!("status refresh advanced history"),
                Ok(Event::Snapshot(_)) => snapshots += 1,
                _ => {}
            }
        }
        assert!(snapshots > 0);
        until(&rx, |e| matches!(e, Event::Sample(None)));
        state.lock().unwrap().fail = false;
        until(&rx, |e| matches!(e, Event::Sample(Some(_))));
        stop(tx, rx, j);
    }
    #[test]
    fn stable_takeover_and_resume_do_not_ramp_from_firmware_stop_duty() {
        let mut s = Settings::default();
        s.global.stable_response = true;
        let (state, tx, rx, j) = start(s, vec![55, 41, 41]);
        until(&rx, |e| matches!(e, Event::Sample(Some(_))));
        state.lock().unwrap().duties = vec![4; 3];
        tx.send(Command::Takeover(true)).unwrap();
        until(&rx, |_| !state.lock().unwrap().writes.is_empty());
        let takeover = state.lock().unwrap().writes[0].clone();

        tx.send(Command::Suspend).unwrap();
        until(
            &rx,
            |e| matches!(e, Event::Snapshot(s) if s.mode == "休眠暂停"),
        );
        state.lock().unwrap().duties = vec![0; 3];
        let count = state.lock().unwrap().writes.len();
        tx.send(Command::Resume).unwrap();
        until(&rx, |_| state.lock().unwrap().writes.len() > count);
        let resumed = state.lock().unwrap().writes[count].clone();
        stop(tx, rx, j);

        assert_eq!(takeover, vec![40; 3]);
        assert_eq!(resumed, vec![40; 3]);
    }
    #[test]
    fn stable_profile_bypasses_delay_when_raw_temperature_is_high() {
        let mut s = Settings::default();
        s.global.stable_response = true;
        s.global.take_over = true;
        let (state, tx, rx, j) = start(s, vec![81, 50]);
        until(
            &rx,
            |e| matches!(e,Event::Snapshot(s) if s.mode.starts_with("高温保护")),
        );
        assert_eq!(state.lock().unwrap().writes[0][0], 100);
        stop(tx, rx, j);
        assert_eq!(state.lock().unwrap().restores, 1);
    }
    #[test]
    fn third_fan_follows_gpu_and_exit_restores() {
        let mut s = Settings::default();
        s.global.take_over = true;
        for p in &mut s.global.cpu_curve {
            p.duty = 35;
        }
        let (state, tx, rx, j) = start(s, vec![50, 60, 80]);
        until(
            &rx,
            |e| matches!(e,Event::Snapshot(s) if s.mode=="软件曲线"),
        );
        assert_eq!(state.lock().unwrap().writes[0], vec![35, 40, 40]);
        stop(tx, rx, j);
        assert_eq!(state.lock().unwrap().restores, 1);
    }
    #[test]
    fn sample_failure_returns_automatic_and_latches_fault() {
        let mut s = Settings::default();
        s.global.take_over = true;
        let (state, tx, rx, j) = start(s, vec![50, 60]);
        until(
            &rx,
            |e| matches!(e,Event::Snapshot(s) if s.mode=="软件曲线"),
        );
        state.lock().unwrap().fail = true;
        until(
            &rx,
            |e| matches!(e,Event::Snapshot(s) if s.mode=="不可用"&&s.sample.is_none()),
        );
        assert_eq!(state.lock().unwrap().restores, 1);
        let writes = state.lock().unwrap().writes.len();
        state.lock().unwrap().fail = false;
        until(
            &rx,
            |e| matches!(e,Event::Snapshot(s) if s.sample.is_some()),
        );
        assert_eq!(state.lock().unwrap().writes.len(), writes);
        stop(tx, rx, j);
    }
    #[test]
    fn readback_detects_external_override_after_target_is_stable() {
        let mut s = Settings::default();
        s.global.take_over = true;
        s.global.soft_control = true;
        let (state, tx, rx, j) = start(s, vec![60, 60]);
        until(&rx, |_| state.lock().unwrap().duties == vec![40, 40]);
        let count = state.lock().unwrap().writes.len();
        state.lock().unwrap().duties = vec![55, 55];
        until(&rx, |_| state.lock().unwrap().writes.len() > count);
        assert_eq!(state.lock().unwrap().duties, vec![40, 40]);
        stop(tx, rx, j);
    }
    #[test]
    fn forced_cooling_overrides_takeover_and_completes_once() {
        let (state, tx, rx, j) = start(Settings::default(), vec![60, 60]);
        tx.send(Command::Force(true)).unwrap();
        until(&rx, |_| state.lock().unwrap().duties == vec![95, 95]);
        state.lock().unwrap().temps = vec![49, 49];
        until(&rx, |e| matches!(e, Event::CoolingDone));
        until(
            &rx,
            |e| matches!(e,Event::Snapshot(s) if s.mode=="固件自动"),
        );
        assert_eq!(state.lock().unwrap().restores, 1);
        stop(tx, rx, j);
    }
    #[test]
    fn cancel_queued_during_sampling_prevents_stale_write() {
        struct Blocking {
            inner: Mock,
            ready: Option<Sender<()>>,
            release: Receiver<()>,
        }
        impl Hardware for Blocking {
            fn sample(&mut self) -> Result<Sample, String> {
                if let Some(t) = self.ready.take() {
                    t.send(()).unwrap();
                    self.release.recv_timeout(Duration::from_secs(2)).unwrap();
                }
                self.inner.sample()
            }
            fn write(&mut self, d: &[i32]) -> Result<(), String> {
                self.inner.write(d)
            }
            fn automatic(&mut self) -> Result<(), String> {
                self.inner.automatic()
            }
            fn description(&self) -> String {
                "blocking test".into()
            }
        }
        let state = Arc::new(Mutex::new(State {
            temps: vec![60, 60],
            duties: vec![30, 30],
            ..State::default()
        }));
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (tx, rx) = mpsc::channel();
        let (etx, erx) = mpsc::channel();
        let h = Blocking {
            inner: Mock(state.clone()),
            ready: Some(ready_tx),
            release: release_rx,
        };
        let mut settings = Settings::default();
        settings.global.take_over = true;
        let j = thread::spawn(move || run(settings, Ok(Box::new(h)), rx, etx));
        ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        tx.send(Command::Takeover(false)).unwrap();
        release_tx.send(()).unwrap();
        until(
            &erx,
            |e| matches!(e,Event::Snapshot(s) if s.mode=="固件自动"),
        );
        assert!(state.lock().unwrap().writes.is_empty());
        stop(tx, erx, j);
    }

    #[test]
    fn history_preserves_acquisition_before_a_blocked_write_and_exit_result() {
        struct BlockingWrite {
            inner: Mock,
            ready: Sender<()>,
            release: Receiver<()>,
        }
        impl Hardware for BlockingWrite {
            fn sample(&mut self) -> Result<Sample, String> {
                self.inner.sample()
            }
            fn write(&mut self, d: &[i32]) -> Result<(), String> {
                self.ready.send(()).unwrap();
                self.release.recv_timeout(Duration::from_secs(4)).unwrap();
                self.inner.write(d)
            }
            fn automatic(&mut self) -> Result<(), String> {
                self.inner.automatic()
            }
            fn description(&self) -> String {
                "blocking write simulation".into()
            }
        }
        let directory =
            std::env::temp_dir().join(format!("clevo-worker-history-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let mut recorder = crate::recording::Recorder::new(&directory.join("app.exe"), true);
        let producer = recorder.start().unwrap();
        let state = Arc::new(Mutex::new(State {
            temps: vec![50, 60],
            duties: vec![30; 2],
            ..State::default()
        }));
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let hardware = BlockingWrite {
            inner: Mock(state.clone()),
            ready: ready_tx,
            release: release_rx,
        };
        let (tx, rx) = mpsc::channel();
        let (etx, erx) = mpsc::channel();
        let mut settings = Settings::default();
        settings.global.take_over = true;
        settings.global.update_interval = 10.0;
        tx.send(Command::Recording(Some(producer))).unwrap();
        let j = thread::spawn(move || run(settings, Ok(Box::new(hardware)), rx, etx));
        ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let read_events = || {
            std::fs::read_dir(directory.join("history-demo"))
                .unwrap()
                .filter_map(|e| {
                    let p = e.ok()?.path();
                    p.to_string_lossy()
                        .ends_with(".events.jsonl")
                        .then(|| std::fs::read_to_string(p).unwrap_or_default())
                })
                .collect::<String>()
        };
        let mut text = read_events();
        while !text.contains("\"kind\":\"sample\"") && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
            text = read_events();
        }
        assert!(text.contains("\"kind\":\"sample\""));
        assert!(text.contains("\"kind\":\"write_start\""));
        assert!(!text.contains("\"kind\":\"write_result\""));
        tx.send(Command::Exit).unwrap();
        release_tx.send(()).unwrap();
        until(&erx, |e| matches!(e, Event::Stopped(Ok(()))));
        j.join().unwrap();
        assert!(recorder.finish(Duration::from_secs(2)));
        let text = read_events();
        assert!(text.contains("\"kind\":\"restore_result\""));
        assert!(text.contains("\"Ok\":null"));
        let s = state.lock().unwrap();
        assert_eq!(s.samples, 1);
        assert_eq!(s.writes.len(), 1);
        assert_eq!(s.restores, 1);
        drop(s);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn history_toggle_does_not_resample_and_verification_keeps_both_readings() {
        let directory =
            std::env::temp_dir().join(format!("clevo-worker-retry-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let mut recorder = crate::recording::Recorder::new(&directory.join("app.exe"), true);
        let (state, tx, rx, j) = start(Settings::default(), vec![50, 50]);
        until(&rx, |e| matches!(e, Event::Sample(Some(_))));
        let count = state.lock().unwrap().samples;
        tx.send(Command::Recording(Some(recorder.start().unwrap())))
            .unwrap();
        until(&rx, |e| matches!(e, Event::Snapshot(_)));
        assert_eq!(state.lock().unwrap().samples, count);
        state.lock().unwrap().temps = vec![90, 90];
        until(
            &rx,
            |e| matches!(e,Event::Sample(Some(s)) if s.channels[0].temperature==90),
        );
        stop(tx, rx, j);
        assert!(recorder.finish(Duration::from_secs(2)));
        let text = std::fs::read_dir(directory.join("history-demo"))
            .unwrap()
            .filter_map(|e| {
                let p = e.ok()?.path();
                p.to_string_lossy()
                    .ends_with(".events.jsonl")
                    .then(|| std::fs::read_to_string(p).unwrap())
            })
            .collect::<String>();
        assert!(text.contains("verification_required"));
        let events: Vec<serde_json::Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert!(
            events
                .iter()
                .any(|e| e["detail"]["verification_of"].is_u64())
        );
        assert_eq!(state.lock().unwrap().samples, count + 2);
        assert!(state.lock().unwrap().writes.is_empty());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn history_records_protection_fault_suspend_resume_and_failed_restoration() {
        let directory =
            std::env::temp_dir().join(format!("clevo-worker-events-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let mut recorder = crate::recording::Recorder::new(&directory.join("app.exe"), true);
        let mut settings = Settings::default();
        settings.global.take_over = true;
        settings.global.stable_response = true;
        let (state, tx, rx, j) = start(settings, vec![81, 50]);
        tx.send(Command::Recording(Some(recorder.start().unwrap())))
            .unwrap();
        until(
            &rx,
            |e| matches!(e,Event::Snapshot(s) if s.mode.starts_with("高温保护")),
        );
        tx.send(Command::Suspend).unwrap();
        until(
            &rx,
            |e| matches!(e,Event::Snapshot(s) if s.mode=="休眠暂停"),
        );
        tx.send(Command::Resume).unwrap();
        until(
            &rx,
            |e| matches!(e,Event::Snapshot(s) if s.mode.starts_with("高温保护")),
        );
        {
            let mut s = state.lock().unwrap();
            s.fail = true;
            s.restore_failure = true;
        }
        until(&rx, |e| matches!(e,Event::Snapshot(s) if s.mode=="不可用"));
        tx.send(Command::Exit).unwrap();
        until(&rx, |e| matches!(e, Event::Stopped(Err(_))));
        j.join().unwrap();
        assert!(recorder.finish(Duration::from_secs(2)));
        let text = std::fs::read_dir(directory.join("history-demo"))
            .unwrap()
            .filter_map(|e| {
                let p = e.ok()?.path();
                p.to_string_lossy()
                    .ends_with(".events.jsonl")
                    .then(|| std::fs::read_to_string(p).unwrap())
            })
            .collect::<String>();
        for expected in [
            "suspend",
            "resume",
            "protection",
            "sample_error",
            "restore unavailable",
            "restore_result",
        ] {
            assert!(text.contains(expected), "missing {expected}");
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}
