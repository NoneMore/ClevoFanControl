use clevo_fan_control::{
    config::{self, Result, Settings},
    control::{ChannelState, StableResponse},
    hardware::{Demo, Hardware, Insyde},
    platform::SingleInstance,
    tuning::{self, ChannelStats, Tuner},
};
use serde::Serialize;
use std::{
    fs::{self, File, OpenOptions},
    io::{BufWriter, Write},
    os::windows::process::CommandExt,
    path::PathBuf,
    process::{Command as ProcessCommand, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::System::Console::*;

static STOP: AtomicBool = AtomicBool::new(false);
static RESTORED: AtomicBool = AtomicBool::new(true);

unsafe extern "system" fn console_control(event: u32) -> i32 {
    if matches!(
        event,
        CTRL_C_EVENT
            | CTRL_BREAK_EVENT
            | CTRL_CLOSE_EVENT
            | CTRL_LOGOFF_EVENT
            | CTRL_SHUTDOWN_EVENT
    ) {
        STOP.store(true, Ordering::SeqCst);
        if matches!(
            event,
            CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT | CTRL_SHUTDOWN_EVENT
        ) {
            for _ in 0..40 {
                if RESTORED.load(Ordering::SeqCst) {
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
        1
    } else {
        0
    }
}

const HELP: &str = "ClevoFanControl CLI
  clevo-fan-cli monitor --out DIR [--duration-secs 14400] [--interval-secs 2]
  clevo-fan-cli tune --out DIR [--duration-secs 14400] [--baseline-secs 600]
                     [--cpu-target 60] [--gpu-target 50] [--interval-secs 2]
  clevo-fan-cli stop --out DIR
  clevo-fan-cli status --out DIR
  clevo-fan-cli load --out DIR [--interval-secs 5]

Additional options: --config FILE, --demo, --help
Close the GUI first: all controllers share the same single-instance lock.
Tune uses 25% CPU / 40% GPU seed duty floors, 85C CPU / 80C GPU session limits.
CPU target has a +5C stability band; GPU target has a +2C band.
15s median + 30s EMA, 2-point duty deadband, normal changes every 30s
(up <=2 points, down <=1 point after 120s). Raw CPU>=80/GPU>=75C bypasses
delays to 100%. Candidate enables the same stable response in the GUI.
Baseline runs firmware automatic; adaptation starts after baseline, every 180s.
Stop with Ctrl+C or the stop command. Exit restores firmware automatic.
DIR must be a new session directory; telemetry.csv, events.jsonl, report.json,
session.json, load.csv and candidate.json are written there. Candidate never replaces
your original config; its takeover and autorun switches remain disabled.
Only two physical fans are supported for tuning. Monitor supports 1-3 fans.
This is a bounded ordinary-workload experiment, not full-load certification.
Force kill / driver hangs cannot guarantee restoration.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Mode {
    Monitor,
    Tune,
    Stop,
    Status,
    Load,
}
impl Mode {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "monitor" => Ok(Self::Monitor),
            "tune" => Ok(Self::Tune),
            "stop" => Ok(Self::Stop),
            "status" => Ok(Self::Status),
            "load" => Ok(Self::Load),
            _ => Err(format!("Unknown command: {value}")),
        }
    }
}

#[derive(Debug)]
struct Options {
    mode: Mode,
    out: PathBuf,
    config: PathBuf,
    duration: Duration,
    interval: Duration,
    baseline: Duration,
    targets: [i32; 2],
    demo: bool,
}
impl Options {
    fn parse(args: &[String]) -> Result<Self> {
        let mode = Mode::parse(args.first().ok_or("Use --help for commands")?)?;
        let mut out = None;
        let mut config = std::env::current_exe()
            .map_err(|e| e.to_string())?
            .with_file_name("ClevoFanControl.x64.json");
        let mut duration = 14400_u64;
        let mut interval = if mode == Mode::Load { 5.0_f64 } else { 2.0_f64 };
        let mut baseline = 600_u64;
        let mut targets = [60, 50];
        let mut demo = false;
        let mut i = 1;
        while i < args.len() {
            let flag = &args[i];
            if flag == "--demo" {
                demo = true;
                i += 1;
                continue;
            }
            let value = args
                .get(i + 1)
                .ok_or_else(|| format!("Missing value for {flag}"))?;
            let number_error = || format!("Invalid value for {flag}: {value}");
            match flag.as_str() {
                "--out" => out = Some(PathBuf::from(value)),
                "--config" => config = PathBuf::from(value),
                "--duration-secs" => duration = value.parse().map_err(|_| number_error())?,
                "--interval-secs" => interval = value.parse().map_err(|_| number_error())?,
                "--baseline-secs" => baseline = value.parse().map_err(|_| number_error())?,
                "--cpu-target" => targets[0] = value.parse().map_err(|_| number_error())?,
                "--gpu-target" => targets[1] = value.parse().map_err(|_| number_error())?,
                _ => return Err(format!("Unknown option: {flag}")),
            }
            i += 2;
        }
        if !(1..=86400).contains(&duration)
            || !(0.2..=10.0).contains(&interval)
            || !(60..=78).contains(&targets[0])
            || !(45..=73).contains(&targets[1])
        {
            return Err(
                "Duration: 1-86400s; interval: 0.2-10s; CPU target: 60-78C; GPU target: 45-73C"
                    .into(),
            );
        }
        if mode == Mode::Tune && (baseline >= duration || (!demo && baseline < 60)) {
            return Err(
                "Tuning needs baseline < duration and at least 60s real-hardware baseline".into(),
            );
        }
        Ok(Self {
            mode,
            out: out.ok_or("--out DIR is required")?,
            config,
            duration: Duration::from_secs(duration),
            interval: Duration::from_secs_f64(interval),
            baseline: Duration::from_secs(baseline),
            targets,
            demo,
        })
    }
}

struct RestoreGuard {
    hardware: Box<dyn Hardware>,
    owned: bool,
}

struct SessionLog {
    csv: BufWriter<File>,
    events: BufWriter<File>,
}
impl RestoreGuard {
    fn write(&mut self, duties: &[i32]) -> Result<()> {
        self.owned = true; // Arm before a possibly partial write.
        RESTORED.store(false, Ordering::SeqCst);
        self.hardware.write(duties)
    }
    fn restore(&mut self) -> Result<()> {
        if !self.owned {
            return Ok(());
        }
        let mut error = String::new();
        for _ in 0..3 {
            match self.hardware.automatic() {
                Ok(()) => {
                    self.owned = false;
                    RESTORED.store(true, Ordering::SeqCst);
                    return Ok(());
                }
                Err(e) => {
                    error = e;
                    thread::sleep(Duration::from_millis(200));
                }
            }
        }
        Err(error)
    }
}
impl Drop for RestoreGuard {
    fn drop(&mut self) {
        if let Err(e) = self.restore() {
            eprintln!("RESTORE FAILED: {e}");
        }
    }
}

#[derive(Serialize)]
struct Report {
    state: String,
    reason: String,
    pid: u32,
    started_unix: u64,
    elapsed_secs: f64,
    requested_duration_secs: u64,
    baseline_secs: u64,
    demo: bool,
    description: String,
    targets: [i32; 2],
    session_limits: [i32; 2],
    baseline: [ChannelStats; 2],
    tuning: [ChannelStats; 2],
    adjustments: u64,
    restored_automatic: Option<bool>,
    scope: &'static str,
}

fn io_error(e: std::io::Error) -> String {
    e.to_string()
}
fn write_report(options: &Options, report: &Report) -> Result<()> {
    config::atomic_write(
        &options.out.join("report.json"),
        &serde_json::to_vec_pretty(report).map_err(|e| e.to_string())?,
    )
}
fn event(
    log: &mut BufWriter<File>,
    elapsed: f64,
    name: &str,
    detail: impl Serialize,
) -> Result<()> {
    serde_json::to_writer(
        &mut *log,
        &serde_json::json!({"elapsed_secs":elapsed,"event":name,"detail":detail}),
    )
    .map_err(|e| e.to_string())?;
    writeln!(log).map_err(io_error)?;
    log.flush().map_err(io_error)
}
fn candidate(options: &Options, settings: &Settings, tuner: &Tuner) -> Result<()> {
    let mut candidate = settings.clone();
    candidate.global = tuner.control.clone();
    candidate.global.take_over = false;
    candidate.auto_run = false;
    candidate.auto_switch = false;
    candidate.history_recording_enabled = false;
    candidate.start_to_tray = false;
    candidate.save(&options.out.join("candidate.json"))
}

fn session(options: &Options) -> Result<()> {
    let _instance = SingleInstance::acquire()?;
    let settings = Settings::load(&options.config)?; // CLI never silently replaces an invalid config.
    fs::create_dir_all(&options.out).map_err(io_error)?;
    let session_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(options.out.join("session.json"))
        .map_err(|e| format!("Use a new output directory: {e}"))?;
    serde_json::to_writer_pretty(
        session_file,
        &serde_json::json!({"pid":std::process::id(),"mode":options.mode,"demo":options.demo}),
    )
    .map_err(|e| e.to_string())?;
    if options.out.join("STOP").exists() {
        return Err("Output directory already has a STOP request".into());
    }
    let mut csv = BufWriter::new(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(options.out.join("telemetry.csv"))
            .map_err(io_error)?,
    );
    let mut events = BufWriter::new(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(options.out.join("events.jsonl"))
            .map_err(io_error)?,
    );
    writeln!(
        csv,
        "unix_ms,elapsed_secs,phase,channel,temperature_c,actual_duty_percent,rpm,next_duty_percent"
    )
    .map_err(io_error)?;
    let hardware: Box<dyn Hardware> = if options.demo {
        Box::new(Demo::default())
    } else {
        Box::new(Insyde::open(&settings.compatibility)?)
    };
    let mut guard = RestoreGuard {
        hardware,
        owned: false,
    };
    let tune = options.mode == Mode::Tune;
    let mut tuner = Tuner::new(options.targets);
    tuner.control.update_interval = options.interval.as_secs_f64();
    let mut report = Report {
        state: "running".into(),
        reason: "".into(),
        pid: std::process::id(),
        started_unix: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        elapsed_secs: 0.0,
        requested_duration_secs: options.duration.as_secs(),
        baseline_secs: options.baseline.as_secs(),
        demo: options.demo,
        description: guard.hardware.description(),
        targets: options.targets,
        session_limits: tuning::LIMITS,
        baseline: Default::default(),
        tuning: Default::default(),
        adjustments: 0,
        restored_automatic: None,
        scope: "Observed ordinary workload only. No acoustic measurement or unsampled high-load validation. Compare baseline and tuning only under comparable workloads.",
    };
    if tune {
        candidate(options, &settings, &tuner)?;
    }
    write_report(options, &report)?;
    event(&mut events, 0.0, "started", &report)?;
    if !options.demo {
        // Independent process: load collection and NVIDIA query timeouts cannot block EC calls.
        let error_log = File::create(options.out.join("load.stderr.log")).map_err(io_error)?;
        let load = ProcessCommand::new(std::env::current_exe().map_err(io_error)?)
            .arg("load")
            .arg("--out")
            .arg(&options.out)
            .args(["--interval-secs", "5"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(error_log)
            .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
            .spawn();
        match load {
            Ok(child) => event(&mut events, 0.0, "load_recorder_started", child.id())?,
            Err(e) => event(&mut events, 0.0, "load_recorder_failed", e.to_string())?,
        }
    }
    let started = Instant::now();
    let mut logs = SessionLog { csv, events };
    let result = run_loop(
        options,
        &mut guard,
        &mut tuner,
        &settings,
        &mut report,
        &mut logs,
        started,
    );
    // Restore before any final file writes, including after telemetry/logging errors.
    let restore = guard.restore();
    report.elapsed_secs = started.elapsed().as_secs_f64();
    report.restored_automatic = tune.then_some(restore.is_ok());
    report.state = if result.is_err() || restore.is_err() {
        "failed"
    } else {
        "finished"
    }
    .into();
    report.reason = match &result {
        Ok(reason) => reason.clone(),
        Err(e) => e.clone(),
    };
    if let Err(e) = &restore {
        report.reason.push_str(&format!("; restore failed: {e}"));
    }
    write_report(options, &report)?;
    event(&mut logs.events, report.elapsed_secs, "finished", &report)?;
    if tune {
        candidate(options, &settings, &tuner)?;
    }
    println!(
        "{}: {}. Firmware restore: {:?}",
        report.state, report.reason, report.restored_automatic
    );
    restore?;
    result.map(|_| ())
}

fn run_loop(
    options: &Options,
    guard: &mut RestoreGuard,
    tuner: &mut Tuner,
    settings: &Settings,
    report: &mut Report,
    logs: &mut SessionLog,
    started: Instant,
) -> Result<String> {
    let SessionLog { csv, events } = logs;
    let tune = options.mode == Mode::Tune;
    // Reset to firmware automatic for a reproducible baseline. Mark the guard first.
    // Keep initialization inside the result path so restore failures reach report.json.
    if tune {
        guard.owned = true;
        RESTORED.store(false, Ordering::SeqCst);
        guard.restore()?;
    }
    let mut states = [ChannelState::default(); 2];
    let mut responses = [StableResponse::default(), StableResponse::default()];
    let mut last_command: Option<Vec<i32>> = None;
    let mut mismatch = [0; 2];
    let mut stalled = [0; 2];
    let mut previous_sample = None;
    let mut previous_temps: Option<[i32; 2]> = None;
    let mut expected_channels = None;
    let mut next_status = Duration::ZERO;
    let mut next_report = Duration::ZERO;
    let mut active_since = None;
    loop {
        if STOP.load(Ordering::SeqCst) || options.out.join("STOP").exists() {
            return Ok("stop requested".into());
        }
        let elapsed = started.elapsed();
        if elapsed >= options.duration {
            return Ok("duration completed".into());
        }
        if previous_sample.is_some_and(|last: Duration| {
            elapsed.saturating_sub(last) > options.interval + Duration::from_secs(5)
        }) {
            return Err(
                "Sampling gap (possible sleep/resume or blocked driver); ending session".into(),
            );
        }
        let sample = guard.hardware.sample()?;
        // A slow driver call must not be followed by a stale write.
        if started.elapsed().saturating_sub(elapsed) > Duration::from_secs(5) {
            return Err("Hardware sample took over 5 seconds".into());
        }
        if STOP.load(Ordering::SeqCst) || options.out.join("STOP").exists() {
            return Ok("stop requested".into());
        }
        if tune {
            tuning::validate_sample(&sample, expected_channels)?;
            tuning::check_temperature(&sample)?;
        }
        if !(1..=3).contains(&sample.channels.len())
            || expected_channels.is_some_and(|n| n != sample.channels.len())
        {
            return Err("Fan channel count changed".into());
        }
        expected_channels = Some(sample.channels.len());
        previous_sample = Some(elapsed);
        let active = tune && elapsed >= options.baseline;
        let phase = if active {
            "tuning"
        } else if tune {
            "baseline"
        } else {
            "monitor"
        };
        if active {
            let temps = [
                sample.channels[0].temperature,
                sample.channels[1].temperature,
            ];
            if previous_temps
                .is_some_and(|old| old.iter().zip(temps).any(|(a, b)| (a - b).abs() > 20))
            {
                return Err(
                    "Temperature jumped over 20C between samples; restore and inspect telemetry"
                        .into(),
                );
            }
            previous_temps = Some(temps);
            let active_start = *active_since.get_or_insert(elapsed);
            for (ch, reading) in sample.channels.iter().enumerate() {
                if let Some(command) = &last_command {
                    mismatch[ch] = if (reading.duty - command[ch]).abs() > 3 {
                        mismatch[ch] + 1
                    } else {
                        0
                    };
                    if mismatch[ch] >= 5 {
                        return Err(format!(
                            "Channel {} duty mismatch on 5 consecutive samples; possible competing controller",
                            ch + 1
                        ));
                    }
                }
                stalled[ch] = if elapsed.saturating_sub(active_start) > Duration::from_secs(15)
                    && reading.rpm.is_none_or(|rpm| rpm == 0)
                {
                    stalled[ch] + 1
                } else {
                    0
                };
                if stalled[ch] >= 3 {
                    return Err(format!(
                        "Channel {} tachometer absent/zero on 3 samples",
                        ch + 1
                    ));
                }
            }
            let filtered = [
                responses[0].temperature(temps[0], sample.at),
                responses[1].temperature(temps[1], sample.at),
            ];
            let changes = tuner.observe(elapsed.saturating_sub(active_start), filtered);
            if !changes.is_empty() {
                report.adjustments += changes.len() as u64;
                candidate(options, settings, tuner)?;
                event(events, elapsed.as_secs_f64(), "curve_adjusted", changes)?;
            }
            let curves = [&tuner.control.cpu_curve, &tuner.control.gpu_curve];
            let duties: Vec<i32> = sample
                .channels
                .iter()
                .enumerate()
                .map(|(ch, reading)| {
                    let target = states[ch].evaluate(curves[ch], filtered[ch], &tuner.control);
                    let current = last_command.as_ref().map_or(reading.duty, |d| d[ch]);
                    responses[ch].duty(current, target, reading.temperature, ch, sample.at)
                })
                .collect();
            // Recheck deadline after logging/config I/O and immediately before hardware mutation.
            if STOP.load(Ordering::SeqCst) || options.out.join("STOP").exists() {
                return Ok("stop requested".into());
            }
            if started.elapsed() >= options.duration {
                return Ok("duration completed".into());
            }
            if sample.at.elapsed() > options.interval + Duration::from_secs(3) {
                return Err("Sample expired before control write".into());
            }
            if last_command.as_ref() != Some(&duties)
                || sample
                    .channels
                    .iter()
                    .zip(&duties)
                    .any(|(r, d)| (r.duty - d).abs() > 2)
            {
                guard.write(&duties)?;
            }
            last_command = Some(duties);
        }
        let unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        for (ch, reading) in sample.channels.iter().enumerate() {
            if ch < 2 {
                if active {
                    report.tuning[ch].add(reading)
                } else {
                    report.baseline[ch].add(reading)
                }
            }
            let next = last_command
                .as_ref()
                .and_then(|d| d.get(ch))
                .map_or(String::new(), ToString::to_string);
            writeln!(
                csv,
                "{unix_ms},{:.3},{phase},{},{},{},{},{next}",
                elapsed.as_secs_f64(),
                ch + 1,
                reading.temperature,
                reading.duty,
                reading.rpm.map_or(String::new(), |r| r.to_string())
            )
            .map_err(io_error)?;
        }
        csv.flush().map_err(io_error)?;
        report.elapsed_secs = elapsed.as_secs_f64();
        if elapsed >= next_report {
            write_report(options, report)?;
            next_report = elapsed + Duration::from_secs(30);
        }
        if elapsed >= next_status {
            println!(
                "{:.0}s {phase}: {}",
                elapsed.as_secs_f64(),
                sample
                    .channels
                    .iter()
                    .enumerate()
                    .map(|(ch, r)| format!(
                        "ch{} {}C {}% {}RPM",
                        ch + 1,
                        r.temperature,
                        r.duty,
                        r.rpm.unwrap_or(0)
                    ))
                    .collect::<Vec<_>>()
                    .join(" | ")
            );
            next_status = elapsed + Duration::from_secs(30);
        }
        let deadline = (elapsed + options.interval).min(options.duration);
        while started.elapsed() < deadline {
            if STOP.load(Ordering::SeqCst) || options.out.join("STOP").exists() {
                return Ok("stop requested".into());
            }
            thread::sleep(
                Duration::from_millis(100).min(deadline.saturating_sub(started.elapsed())),
            );
        }
    }
}

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{HELP}");
        return;
    }
    let result = (|| {
        let options = Options::parse(&args)?;
        if options.mode == Mode::Load {
            return clevo_fan_control::load::record(&options.out, options.interval);
        }
        if options.mode == Mode::Status {
            let report: serde_json::Value = config::read_json(&options.out.join("report.json"))?;
            println!(
                "{}",
                serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
            );
            return Ok(());
        }
        if options.mode == Mode::Stop {
            if !options.out.join("session.json").is_file() {
                return Err("No session.json found in --out directory".into());
            }
            fs::write(options.out.join("STOP"), b"stop requested\n").map_err(io_error)?;
            println!("Stop requested; check report.json for final state and restored_automatic.");
            return Ok(());
        }
        if unsafe { SetConsoleCtrlHandler(Some(console_control), 1) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        session(&options)
    })();
    if let Err(e) = result {
        eprintln!("ERROR: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    struct PartialWrite(Arc<Mutex<usize>>);
    impl Hardware for PartialWrite {
        fn sample(&mut self) -> Result<clevo_fan_control::hardware::Sample> {
            Err("unused".into())
        }
        fn write(&mut self, _: &[i32]) -> Result<()> {
            Err("partial write".into())
        }
        fn automatic(&mut self) -> Result<()> {
            *self.0.lock().unwrap() += 1;
            Ok(())
        }
        fn description(&self) -> String {
            "test".into()
        }
    }
    #[test]
    fn partial_hardware_write_arms_restore_guard() {
        let restores = Arc::new(Mutex::new(0));
        {
            let mut guard = RestoreGuard {
                hardware: Box::new(PartialWrite(restores.clone())),
                owned: false,
            };
            assert!(guard.write(&[40, 40]).is_err());
        }
        assert_eq!(*restores.lock().unwrap(), 1);
    }
    #[test]
    fn rejects_unsafe_options_and_unknown_flags() {
        for extra in [
            "--interval-secs NaN",
            "--duration-secs 0",
            "--cpu-target 85",
            "--gpu-target 80",
            "--baseline-secs 0",
            "--unknown value",
        ] {
            let args = format!("tune --out test {extra}")
                .split_whitespace()
                .map(str::to_string)
                .collect::<Vec<_>>();
            assert!(Options::parse(&args).is_err(), "{extra}");
        }
    }
}
