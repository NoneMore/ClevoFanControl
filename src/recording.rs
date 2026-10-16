//! Optional GUI history. Producers never perform file I/O or wait for storage.
use crate::{config::ControlConfig, hardware::Sample};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    os::windows::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const CAPACITY: usize = 256;
const FLUSH: Duration = Duration::from_secs(1);
const SEGMENT_BYTES: u64 = 8 * 1024 * 1024;
const TOTAL_BYTES: u64 = 128 * 1024 * 1024;
const HEADER: &str = "format_version,session_id,sequence,sample_unix_ms,elapsed_ms,channel,temperature_c,rpm,actual_duty_percent,curve_duty_percent,requested_duty_percent,reading_status,mode,active_preset,simulated\n";
const STARTING: u8 = 1;
const RECORDING: u8 = 2;
const STOPPING: u8 = 3;
const ERROR: u8 = 4;
const OFF: u8 = 0;
static SESSION_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Context {
    pub control: ControlConfig,
    pub active_preset: Option<String>,
    pub mode: String,
    pub hardware: String,
    pub simulated: bool,
    pub curve_duties: Vec<i32>,
    pub requests: Vec<i32>,
}

struct Shared {
    accepting: AtomicBool,
    submitting: AtomicU64,
    stop: AtomicBool,
    dropped: AtomicU64,
    sequence: AtomicU64,
    state: AtomicU8,
    error: Mutex<String>,
    started: Instant,
}

#[derive(Clone)]
pub struct Producer {
    tx: SyncSender<Record>,
    shared: Arc<Shared>,
    context: Option<Arc<Context>>,
    observation: Option<u64>,
    verification: Option<u64>,
}

struct Record {
    sequence: u64,
    observation: Option<u64>,
    unix_ms: u64,
    elapsed_ms: u128,
    context: Arc<Context>,
    kind: &'static str,
    detail: Value,
    sample: Option<Sample>,
}

pub fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

impl Producer {
    pub fn context(&mut self, context: Context) {
        if self.context.as_deref() != Some(&context) {
            let first = self.context.is_none();
            self.context = Some(Arc::new(context));
            self.event(if first { "session_start" } else { "state" }, json!({}));
        }
    }
    pub fn active(&self) -> bool {
        self.shared.accepting.load(Ordering::SeqCst)
    }
    fn submit(&self, kind: &'static str, detail: Value, sample: Option<Sample>, sequence: u64) {
        struct Submission<'a>(&'a AtomicU64);
        impl Drop for Submission<'_> {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        self.shared.submitting.fetch_add(1, Ordering::SeqCst);
        let _submission = Submission(&self.shared.submitting);
        if !self.active() {
            return;
        }
        let Some(context) = self.context.as_ref() else {
            return;
        };
        let record = Record {
            sequence,
            observation: self.observation,
            unix_ms: unix_ms(),
            elapsed_ms: self.shared.started.elapsed().as_millis(),
            context: context.clone(),
            kind,
            detail,
            sample,
        };
        match self.tx.try_send(record) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                self.shared.dropped.fetch_add(1, Ordering::Relaxed);
            }
            Err(TrySendError::Disconnected(_)) => {
                self.shared.accepting.store(false, Ordering::Release);
            }
        }
    }
    pub fn event(&self, kind: &'static str, detail: Value) {
        let sequence = self.shared.sequence.fetch_add(1, Ordering::Relaxed);
        self.submit(kind, detail, None, sequence);
    }
    pub fn sample(&mut self, sample: &Sample, status: &'static str) {
        let sequence = self.shared.sequence.fetch_add(1, Ordering::Relaxed);
        self.observation = Some(sequence);
        let verification_of = self.verification.take();
        self.submit(
            "sample",
            json!({"reading_status":status,"verification_of":verification_of}),
            Some(sample.clone()),
            sequence,
        );
        if status == "verification_required" {
            self.verification = Some(sequence);
        }
    }
    pub fn sample_error(&mut self, error: &str) {
        let sequence = self.shared.sequence.fetch_add(1, Ordering::Relaxed);
        self.observation = Some(sequence);
        let verification_of = self.verification.take();
        self.submit(
            "sample_error",
            json!({"error":error,"verification_of":verification_of}),
            None,
            sequence,
        );
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    pub state: &'static str,
    pub dropped: u64,
    pub error: String,
    pub directory: PathBuf,
}

pub struct Recorder {
    directory: PathBuf,
    session: Option<Producer>,
    join: Option<JoinHandle<()>>,
}

impl Recorder {
    pub fn ready(&self) -> bool {
        self.join.as_ref().is_none_or(|j| j.is_finished())
    }
    pub fn new(executable: &Path, simulated: bool) -> Self {
        Self {
            directory: executable
                .parent()
                .unwrap_or(Path::new("."))
                .join(if simulated { "history-demo" } else { "history" }),
            session: None,
            join: None,
        }
    }
    pub fn start(&mut self) -> Result<Producer, String> {
        if self.join.as_ref().is_some_and(|j| !j.is_finished()) {
            return Err("The previous history writer is still running or stopping".into());
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
        let (tx, rx) = mpsc::sync_channel(CAPACITY);
        let shared = Arc::new(Shared {
            accepting: AtomicBool::new(true),
            submitting: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            dropped: AtomicU64::new(0),
            sequence: AtomicU64::new(1),
            state: AtomicU8::new(STARTING),
            error: Mutex::new(String::new()),
            started: Instant::now(),
        });
        let producer = Producer {
            tx,
            shared: shared.clone(),
            context: None,
            observation: None,
            verification: None,
        };
        let directory = self.directory.clone();
        self.join = Some(thread::spawn(move || {
            let result = FileStore::new(&directory, Limits::default())
                .and_then(|store| writer(rx, &shared, store));
            shared.accepting.store(false, Ordering::Release);
            if let Err(error) = result {
                *shared.error.lock().unwrap_or_else(|p| p.into_inner()) = error.to_string();
                shared.state.store(ERROR, Ordering::Release);
            } else {
                shared.state.store(OFF, Ordering::Release);
            }
        }));
        self.session = Some(producer.clone());
        Ok(producer)
    }
    pub fn stop(&self) {
        if let Some(producer) = &self.session {
            producer.shared.accepting.store(false, Ordering::SeqCst);
            producer.shared.stop.store(true, Ordering::Release);
            let _ = producer.shared.state.compare_exchange(
                RECORDING,
                STOPPING,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
            let _ = producer.shared.state.compare_exchange(
                STARTING,
                STOPPING,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        }
    }
    pub fn status(&self) -> Status {
        let mut status = Status {
            state: "off",
            dropped: 0,
            error: String::new(),
            directory: self.directory.clone(),
        };
        if let Some(producer) = &self.session {
            status.dropped = producer.shared.dropped.load(Ordering::Relaxed);
            status.state = match producer.shared.state.load(Ordering::Acquire) {
                STARTING => "starting",
                RECORDING if status.dropped > 0 => "degraded",
                RECORDING => "recording",
                STOPPING => "stopping",
                ERROR => "error",
                _ => "off",
            };
            if let Ok(error) = producer.shared.error.try_lock() {
                status.error = error.clone();
            }
        }
        status
    }
    /// Call only after worker restoration. Never joins a thread still doing I/O.
    pub fn finish(&mut self, timeout: Duration) -> bool {
        self.stop();
        let end = Instant::now() + timeout;
        while self.join.as_ref().is_some_and(|j| !j.is_finished()) {
            if Instant::now() >= end {
                return false;
            }
            thread::sleep(Duration::from_millis(10));
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
        true
    }
}
impl Drop for Recorder {
    fn drop(&mut self) {
        self.stop();
    } // Dropping JoinHandle detaches; never block control shutdown.
}

trait HistoryStorage {
    fn record(&mut self, record: &Record, dropped: u64) -> io::Result<()>;
    fn flush(&mut self) -> io::Result<()>;
}

fn writer(rx: Receiver<Record>, shared: &Shared, mut store: impl HistoryStorage) -> io::Result<()> {
    let mut flushed = Instant::now();
    let mut last = None;
    loop {
        let record = if shared.stop.load(Ordering::Acquire) {
            match rx.try_recv() {
                Ok(r) => Some(r),
                Err(_) if shared.submitting.load(Ordering::SeqCst) == 0 => break,
                Err(_) => {
                    thread::sleep(Duration::from_millis(1));
                    continue;
                }
            }
        } else {
            match rx.recv_timeout(
                FLUSH
                    .saturating_sub(flushed.elapsed())
                    .min(Duration::from_millis(50)),
            ) {
                Ok(r) => Some(r),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        };
        if let Some(record) = record {
            store.record(&record, shared.dropped.load(Ordering::Relaxed))?;
            last = Some(record);
            if !shared.stop.load(Ordering::Acquire) {
                shared.state.store(RECORDING, Ordering::Release);
            }
        }
        if flushed.elapsed() >= FLUSH {
            store.flush()?;
            flushed = Instant::now();
        }
    }
    if let Some(mut record) = last {
        record.sequence = shared.sequence.fetch_add(1, Ordering::Relaxed);
        record.kind = "session_end";
        record.sample = None;
        record.unix_ms = unix_ms();
        record.elapsed_ms = shared.started.elapsed().as_millis();
        record.detail = json!({"dropped_records":shared.dropped.load(Ordering::Relaxed)});
        store.record(&record, shared.dropped.load(Ordering::Relaxed))?;
    }
    store.flush()
}

#[derive(Clone, Copy)]
struct Limits {
    segment: u64,
    total: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            segment: SEGMENT_BYTES,
            total: TOTAL_BYTES,
        }
    }
}
struct Managed {
    path: PathBuf,
    date: String,
    bytes: u64,
}
struct Pair {
    csv: BufWriter<File>,
    events: BufWriter<File>,
    paths: [PathBuf; 2],
    bytes: u64,
    date: String,
}
struct FileStore {
    directory: PathBuf,
    session: String,
    segment: u32,
    limits: Limits,
    closed: Vec<Managed>,
    pair: Option<Pair>,
    total: u64,
    context: Option<Arc<Context>>,
    dropped: u64,
}
static MANAGED_NAME: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"^cfc-history-v1-([0-9]{8})-[0-9]{1,20}-[0-9]{1,10}-[0-9]{1,20}-[0-9]{6}\.(csv|events\.jsonl)$").unwrap()
});
fn regular(metadata: &fs::Metadata) -> bool {
    metadata.is_file() && metadata.file_attributes() & 0x400 == 0
}
fn directory_safe(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.file_attributes() & 0x400 != 0 {
        return Err(io::Error::other(
            "History directory must not be a reparse point",
        ));
    }
    Ok(())
}

impl FileStore {
    fn new(directory: &Path, limits: Limits) -> io::Result<Self> {
        match fs::symlink_metadata(directory) {
            Ok(_) => directory_safe(directory)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => fs::create_dir(directory)?,
            Err(e) => return Err(e),
        }
        directory_safe(directory)?;
        let directory = directory.canonicalize()?;
        let mut closed = vec![];
        for entry in fs::read_dir(&directory)? {
            let path = entry?.path();
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if let Some(captures) = MANAGED_NAME.captures(&name) {
                let meta = fs::symlink_metadata(&path)?;
                if !regular(&meta) {
                    return Err(io::Error::other("Unsafe managed history entry"));
                }
                closed.push(Managed {
                    path: path.clone(),
                    date: captures[1].into(),
                    bytes: meta.len(),
                });
            }
        }
        closed.sort_by(|a, b| a.path.cmp(&b.path));
        let total = closed.iter().map(|m| m.bytes).sum();
        Ok(Self {
            directory,
            session: format!(
                "{}-{}-{}",
                unix_ms(),
                std::process::id(),
                SESSION_COUNTER.fetch_add(1, Ordering::Relaxed)
            ),
            segment: 0,
            limits,
            closed,
            pair: None,
            total,
            context: None,
            dropped: 0,
        })
    }
    fn cleanup(&mut self, day: u64, reserve: u64) -> io::Result<()> {
        let date = utc_date(day);
        let cutoff = utc_date(day.saturating_sub(6));
        let dates: BTreeSet<_> = self
            .closed
            .iter()
            .map(|m| m.date.clone())
            .chain([date.clone()])
            .collect();
        let keep: BTreeSet<_> = dates
            .iter()
            .filter(|d| **d != date)
            .rev()
            .take(6)
            .cloned()
            .chain([date.clone()])
            .collect();
        let mut i = 0;
        while i < self.closed.len() {
            let m = &self.closed[i];
            let expired = m.date < cutoff || !keep.contains(&m.date);
            if expired || self.total.saturating_add(reserve) > self.limits.total {
                let stem = managed_stem(&m.path);
                let group: Vec<_> = self
                    .closed
                    .iter()
                    .enumerate()
                    .filter(|(_, entry)| managed_stem(&entry.path) == stem)
                    .map(|(index, _)| index)
                    .collect();
                for index in group.into_iter().rev() {
                    let entry = &self.closed[index];
                    directory_safe(&self.directory)?;
                    let meta = fs::symlink_metadata(&entry.path)?;
                    if entry.path.parent() != Some(self.directory.as_path())
                        || !regular(&meta)
                        || !MANAGED_NAME
                            .is_match(&entry.path.file_name().unwrap_or_default().to_string_lossy())
                    {
                        return Err(io::Error::other("Unsafe history cleanup target"));
                    }
                    fs::remove_file(&entry.path)?;
                    self.total = self.total.saturating_sub(entry.bytes);
                    self.closed.remove(index);
                }
                i = 0;
            } else {
                i += 1;
            }
        }
        if self.total.saturating_add(reserve) > self.limits.total {
            return Err(io::Error::other("History storage budget exhausted"));
        }
        Ok(())
    }
    fn close_pair(&mut self) -> io::Result<()> {
        if let Some(mut pair) = self.pair.take() {
            pair.csv.flush()?;
            pair.events.flush()?;
            drop(pair.csv);
            drop(pair.events);
            for path in pair.paths {
                self.closed.push(Managed {
                    bytes: fs::metadata(&path)?.len(),
                    path,
                    date: pair.date.clone(),
                });
            }
            self.closed.sort_by(|a, b| a.path.cmp(&b.path));
        }
        Ok(())
    }
    fn open_pair(&mut self, day: u64) -> io::Result<()> {
        self.close_pair()?;
        directory_safe(&self.directory)?;
        let date = utc_date(day);
        loop {
            if self.segment > 999999 {
                return Err(io::Error::other("History segment count exhausted"));
            }
            let base = format!("cfc-history-v1-{date}-{}-{:06}", self.session, self.segment);
            self.segment += 1;
            let paths = [
                self.directory.join(format!("{base}.csv")),
                self.directory.join(format!("{base}.events.jsonl")),
            ];
            let csv = match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&paths[0])
            {
                Ok(file) => file,
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            };
            // A failed second open leaves an identifiable orphan, never removes an existing file.
            let events = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&paths[1])?;
            self.pair = Some(Pair {
                csv: BufWriter::new(csv),
                events: BufWriter::new(events),
                paths,
                bytes: 0,
                date,
            });
            self.context = None;
            return Ok(());
        }
    }
}

fn managed_stem(path: &Path) -> String {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    name.strip_suffix(".events.jsonl")
        .or_else(|| name.strip_suffix(".csv"))
        .unwrap_or(&name)
        .into()
}

impl HistoryStorage for FileStore {
    fn record(&mut self, record: &Record, dropped: u64) -> io::Result<()> {
        let day = record.unix_ms / 86_400_000;
        let (csv, event) = encode(record, &self.session)?;
        let checkpoint = format!(
            "{}\n",
            json!({"format_version":1,"session_id":self.session,"kind":"checkpoint","sequence":record.sequence,"unix_ms":record.unix_ms,"elapsed_ms":record.elapsed_ms,"context":record.context.as_ref(),"application_version":env!("CARGO_PKG_VERSION"),"dropped_records":dropped})
        );
        let mut prefix = if self.context.as_deref() != Some(record.context.as_ref())
            || self.dropped != dropped
        {
            checkpoint.clone()
        } else {
            String::new()
        };
        let base = (csv.len() + event.len()) as u64;
        let rotate = self.pair.as_ref().is_none_or(|p| {
            p.date != utc_date(day) || p.bytes + base + prefix.len() as u64 > self.limits.segment
        });
        if rotate {
            self.close_pair()?;
            prefix = checkpoint;
        }
        let header = if self.pair.is_none() {
            HEADER.len() as u64
        } else {
            0
        };
        let reserve = base + prefix.len() as u64 + header;
        if reserve > self.limits.segment {
            return Err(io::Error::other(
                "History observation exceeds segment budget",
            ));
        }
        self.cleanup(day, reserve)?;
        if self.pair.is_none() {
            self.open_pair(day)?;
        }
        let pair = self.pair.as_mut().unwrap();
        if pair.bytes == 0 {
            pair.csv.write_all(HEADER.as_bytes())?;
        }
        pair.csv.write_all(csv.as_bytes())?;
        pair.events.write_all(prefix.as_bytes())?;
        pair.events.write_all(event.as_bytes())?;
        pair.bytes += reserve;
        self.total += reserve;
        self.context = Some(record.context.clone());
        self.dropped = dropped;
        Ok(())
    }
    fn flush(&mut self) -> io::Result<()> {
        if let Some(pair) = self.pair.as_mut() {
            pair.csv.flush()?;
            pair.events.flush()?;
        }
        Ok(())
    }
}

fn cell(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.into()
    }
}
fn encode(record: &Record, session: &str) -> io::Result<(String, String)> {
    let mut csv = String::new();
    if let Some(sample) = &record.sample {
        let verified = record.detail["reading_status"].as_str() == Some("accepted");
        for (ch, r) in sample.channels.iter().enumerate() {
            let curve = if verified {
                record
                    .context
                    .curve_duties
                    .get(ch)
                    .map(ToString::to_string)
                    .unwrap_or_default()
            } else {
                String::new()
            };
            let request = record
                .context
                .requests
                .get(ch)
                .map(ToString::to_string)
                .unwrap_or_default();
            csv.push_str(&format!(
                "1,{session},{},{},{},{},{},{},{},{curve},{request},{},{},{},{}\n",
                record.sequence,
                record.unix_ms,
                record.elapsed_ms,
                ch + 1,
                r.temperature,
                r.rpm.map(|v| v.to_string()).unwrap_or_default(),
                r.duty,
                record.detail["reading_status"].as_str().unwrap_or(""),
                cell(&record.context.mode),
                cell(record.context.active_preset.as_deref().unwrap_or("")),
                record.context.simulated
            ));
        }
    }
    let event = serde_json::to_string(
        &json!({"format_version":1,"session_id":session,"sequence":record.sequence,"observation":record.observation,"unix_ms":record.unix_ms,"elapsed_ms":record.elapsed_ms,"kind":record.kind,"detail":record.detail}),
    )?;
    Ok((csv, format!("{event}\n")))
}

// Gregorian UTC date from an epoch-day number; independent of local DST/time zone.
fn utc_date(day: u64) -> String {
    let z = day as i64 + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(m <= 2);
    format!("{year:04}{m:02}{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::Reading;
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "clevo-history-test-{}-{}",
                std::process::id(),
                SESSION_COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn context() -> Context {
        Context {
            control: ControlConfig::default(),
            active_preset: Some("test,\"quoted\"\nname".into()),
            mode: "software".into(),
            hardware: "simulated test".into(),
            simulated: true,
            curve_duties: vec![40; 3],
            requests: vec![35; 3],
        }
    }
    fn record(sequence: u64, day: u64) -> Record {
        Record {
            sequence,
            observation: Some(sequence),
            unix_ms: day * 86_400_000,
            elapsed_ms: sequence as u128,
            context: Arc::new(context()),
            kind: "sample",
            detail: json!({"reading_status":"accepted"}),
            sample: Some(Sample {
                at: Instant::now(),
                channels: vec![
                    Reading {
                        temperature: 50,
                        duty: 35,
                        rpm: None,
                    },
                    Reading {
                        temperature: 40,
                        duty: 0,
                        rpm: Some(0),
                    },
                    Reading {
                        temperature: 41,
                        duty: 35,
                        rpm: Some(2200),
                    },
                ],
            }),
        }
    }
    fn events(path: &Path) -> Vec<Value> {
        let mut paths: Vec<_> = fs::read_dir(path)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.to_string_lossy().ends_with(".events.jsonl"))
            .collect();
        paths.sort();
        paths
            .into_iter()
            .flat_map(|p| {
                fs::read_to_string(p)
                    .unwrap()
                    .lines()
                    .map(|l| serde_json::from_str::<Value>(l).unwrap())
                    .collect::<Vec<_>>()
            })
            .collect()
    }
    fn shared() -> Arc<Shared> {
        Arc::new(Shared {
            accepting: AtomicBool::new(true),
            submitting: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            dropped: AtomicU64::new(0),
            sequence: AtomicU64::new(1),
            state: AtomicU8::new(STARTING),
            error: Mutex::new(String::new()),
            started: Instant::now(),
        })
    }
    fn producer(capacity: usize) -> (Producer, Receiver<Record>) {
        let (tx, rx) = mpsc::sync_channel(capacity);
        (
            Producer {
                tx,
                shared: shared(),
                context: Some(Arc::new(context())),
                observation: None,
                verification: None,
            },
            rx,
        )
    }
    #[test]
    fn disabled_recorder_does_not_touch_disk_and_errors_are_visible() {
        let temp = Temp::new();
        let mut recorder = Recorder::new(&temp.0.join("app.exe"), true);
        assert_eq!(recorder.status().state, "off");
        assert!(!temp.0.join("history-demo").exists());
        fs::write(temp.0.join("history-demo"), "blocked").unwrap();
        let _ = recorder.start().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !recorder.ready() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(recorder.status().state, "error");
        assert!(!recorder.status().error.is_empty());
    }
    #[test]
    fn queue_saturation_is_nonblocking_and_tail_loss_is_finalized() {
        let (producer, rx) = producer(2);
        for _ in 0..8 {
            producer.event("event", json!({}));
        }
        assert_eq!(producer.shared.dropped.load(Ordering::Relaxed), 6);
        producer.shared.stop.store(true, Ordering::Release);
        let temp = Temp::new();
        let store = FileStore::new(&temp.0, Limits::default()).unwrap();
        writer(rx, &producer.shared, store).unwrap();
        let output = events(&temp.0);
        assert_eq!(output.last().unwrap()["detail"]["dropped_records"], 6);
        assert_eq!(output[0]["kind"], "checkpoint");
    }
    #[test]
    fn stalled_writer_cannot_accumulate_threads_or_block_exit() {
        let temp = Temp::new();
        let (producer, _rx) = producer(1);
        let (release_tx, release_rx) = mpsc::channel();
        let mut recorder = Recorder::new(&temp.0.join("app.exe"), false);
        recorder.session = Some(producer);
        recorder.join = Some(thread::spawn(move || {
            let _ = release_rx.recv();
        }));
        assert!(recorder.start().is_err());
        let began = Instant::now();
        assert!(!recorder.finish(Duration::from_millis(30)));
        assert!(began.elapsed() < Duration::from_secs(1));
        release_tx.send(()).unwrap();
        assert!(recorder.finish(Duration::from_secs(1)));
    }
    #[test]
    fn records_preserve_readings_missing_values_and_verification_links() {
        let (mut p, rx) = producer(10);
        let sample = record(1, 20000).sample.unwrap();
        p.sample(&sample, "verification_required");
        p.sample(&sample, "accepted");
        let first = rx.recv().unwrap();
        let second = rx.recv().unwrap();
        assert_eq!(second.detail["verification_of"], first.sequence);
        let (csv, json) = encode(&second, "session").unwrap();
        assert!(csv.contains(",50,,35,40,35,accepted,"));
        assert!(csv.contains(",40,0,0,40,35,accepted,"));
        assert!(csv.contains("\"test,\"\"quoted\"\"\nname\""));
        assert_eq!(
            serde_json::from_str::<Value>(&json).unwrap()["observation"],
            second.sequence
        );
        assert_eq!(csv.matches(",accepted,").count(), 3);
        p.sample_error("failed retry");
        let error = rx.recv().unwrap();
        assert!(error.sample.is_none());
        assert_eq!(utc_date(0), "19700101");
        assert_eq!(utc_date(20000), "20241004");
    }
    #[test]
    fn rotations_and_drop_recovery_have_self_contained_checkpoints() {
        let temp = Temp::new();
        let mut store = FileStore::new(
            &temp.0,
            Limits {
                segment: 4000,
                total: 12000,
            },
        )
        .unwrap();
        fs::write(temp.0.join("unrelated.csv"), "keep").unwrap();
        for seq in 1..20 {
            store
                .record(&record(seq, 20000 + seq / 3), seq / 2)
                .unwrap();
        }
        store.flush().unwrap();
        assert!(fs::metadata(temp.0.join("unrelated.csv")).is_ok());
        let mut total = 0;
        for entry in fs::read_dir(&temp.0).unwrap() {
            let path = entry.unwrap().path();
            if MANAGED_NAME.is_match(&path.file_name().unwrap().to_string_lossy()) {
                total += fs::metadata(&path).unwrap().len();
                if path.to_string_lossy().ends_with(".events.jsonl") {
                    let text = fs::read_to_string(&path).unwrap();
                    let first: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
                    assert_eq!(first["kind"], "checkpoint");
                    assert_eq!(first["context"]["control"]["updateInterval"], 2.0);
                }
            }
        }
        assert!(total <= 12000);
        let output = events(&temp.0);
        assert!(
            output
                .iter()
                .any(|v| v["dropped_records"].as_u64().is_some_and(|n| n > 0))
        );
        // Backward wall-clock movement is another segment, never an overwrite.
        store.record(&record(20, 19990), 10).unwrap();
        store.flush().unwrap();
    }
    #[test]
    fn retention_handles_orphans_and_rejects_unsafe_managed_entries() {
        let temp = Temp::new();
        let orphan = temp.0.join("cfc-history-v1-19700101-1-1-1-000000.csv");
        fs::write(&orphan, "old").unwrap();
        let mut store = FileStore::new(&temp.0, Limits::default()).unwrap();
        store.record(&record(1, 20000), 0).unwrap();
        assert!(!orphan.exists());
        let unsafe_path = temp.0.join("cfc-history-v1-20241004-1-1-1-000000.csv");
        fs::create_dir(&unsafe_path).unwrap();
        assert!(FileStore::new(&temp.0, Limits::default()).is_err());
        fs::remove_dir(&unsafe_path).unwrap();
        let target = temp.0.join("unrelated-target");
        fs::write(&target, "keep").unwrap();
        let link = temp.0.join("cfc-history-v1-20241004-2-1-1-000000.csv");
        match std::os::windows::fs::symlink_file(&target, &link) {
            Ok(()) => assert!(FileStore::new(&temp.0, Limits::default()).is_err()),
            Err(e) => eprintln!("Managed-link test unavailable: {e}"),
        }
        assert_eq!(fs::read_to_string(target).unwrap(), "keep");
    }
    #[test]
    fn writer_propagates_injected_write_and_flush_failures() {
        struct Failing(bool);
        impl HistoryStorage for Failing {
            fn record(&mut self, _: &Record, _: u64) -> io::Result<()> {
                if self.0 {
                    Err(io::Error::other("write failed"))
                } else {
                    Ok(())
                }
            }
            fn flush(&mut self) -> io::Result<()> {
                Err(io::Error::other("flush failed"))
            }
        }
        for fail_write in [true, false] {
            let (p, rx) = producer(2);
            p.event("event", json!({}));
            p.shared.stop.store(true, Ordering::Release);
            assert!(writer(rx, &p.shared, Failing(fail_write)).is_err());
        }
    }

    #[test]
    fn periodic_flush_runs_with_continuous_input_and_collisions_preserve_files() {
        struct Counting(Arc<AtomicU64>);
        impl HistoryStorage for Counting {
            fn record(&mut self, _: &Record, _: u64) -> io::Result<()> {
                Ok(())
            }
            fn flush(&mut self) -> io::Result<()> {
                self.0.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
        }
        let (p, rx) = producer(CAPACITY);
        let flushes = Arc::new(AtomicU64::new(0));
        let count = flushes.clone();
        let s = p.shared.clone();
        let j = thread::spawn(move || writer(rx, &s, Counting(count)).unwrap());
        let end = Instant::now() + Duration::from_millis(1150);
        while Instant::now() < end {
            p.event("event", json!({}));
            thread::sleep(Duration::from_millis(1));
        }
        assert!(flushes.load(Ordering::Relaxed) > 0);
        p.shared.stop.store(true, Ordering::Release);
        j.join().unwrap();
        let temp = Temp::new();
        let collision = temp.0.join("cfc-history-v1-20241004-1-1-1-000000.csv");
        fs::write(&collision, "original").unwrap();
        let mut store = FileStore::new(&temp.0, Limits::default()).unwrap();
        store.session = "1-1-1".into();
        store.record(&record(1, 20000), 0).unwrap();
        store.flush().unwrap();
        assert_eq!(fs::read_to_string(&collision).unwrap(), "original");
        assert!(
            temp.0
                .join("cfc-history-v1-20241004-1-1-1-000001.csv")
                .exists()
        );
    }

    #[test]
    fn required_cleanup_failure_and_oversized_batch_stop_growth() {
        use std::os::windows::fs::OpenOptionsExt;
        let temp = Temp::new();
        let old = temp.0.join("cfc-history-v1-19700101-1-1-1-000000.csv");
        fs::write(&old, "original").unwrap();
        let lease = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&old)
            .unwrap();
        let mut store = FileStore::new(&temp.0, Limits::default()).unwrap();
        assert!(store.record(&record(1, 20000), 0).is_err());
        assert_eq!(fs::read_to_string(&old).unwrap(), "original");
        drop(lease);
        let mut tiny = FileStore::new(
            &temp.0,
            Limits {
                segment: 100,
                total: 200,
            },
        )
        .unwrap();
        assert!(tiny.record(&record(1, 20000), 0).is_err());
        assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 1);
    }

    #[test]
    fn stop_waits_for_an_already_admitted_submission_before_finalizing() {
        let temp = Temp::new();
        let (p, rx) = producer(1);
        let state = p.shared.clone();
        state.submitting.store(1, Ordering::SeqCst);
        state.accepting.store(false, Ordering::SeqCst);
        state.stop.store(true, Ordering::Release);
        let store = FileStore::new(&temp.0, Limits::default()).unwrap();
        let j = thread::spawn(move || writer(rx, &state, store).unwrap());
        thread::sleep(Duration::from_millis(10));
        assert!(!j.is_finished());
        p.tx.try_send(record(1, unix_ms() / 86_400_000))
            .unwrap_or_else(|_| panic!("Writer finalized before admission completed"));
        p.shared.submitting.fetch_sub(1, Ordering::SeqCst);
        j.join().unwrap();
        let rows = events(&temp.0);
        assert!(rows.iter().any(|e| e["kind"] == "sample"));
        assert_eq!(rows.last().unwrap()["kind"], "session_end");
    }
}
