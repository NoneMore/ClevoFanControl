//! Independent workload recorder. Slow/failed NVIDIA queries never delay EC control.
use crate::config::{self, Result};
use std::{
    fs::OpenOptions,
    io::{BufWriter, Write},
    os::windows::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::{Foundation::*, System::Threading::*};

const GPU_QUERY: &str = concat!(
    "--query-gpu=index,utilization.gpu,utilization.memory,power.draw,",
    "clocks.gr,clocks.mem,temperature.gpu"
);
const CSV_HEADER: &str = concat!(
    "unix_ms,elapsed_secs,cpu_busy_percent,gpu_index,gpu_busy_percent,",
    "gpu_memory_busy_percent,gpu_power_w,gpu_graphics_mhz,gpu_memory_mhz,",
    "gpu_temperature_c,gpu_query_ms,gpu_error"
);

#[derive(Clone, Copy)]
struct CpuTimes {
    idle: u64,
    total: u64,
}
fn cpu_times() -> Result<CpuTimes> {
    let mut idle = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    if unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let ticks = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    Ok(CpuTimes {
        idle: ticks(idle),
        total: ticks(kernel) + ticks(user),
    })
}
fn busy_percent(previous: CpuTimes, current: CpuTimes) -> Option<f64> {
    let total = current.total.checked_sub(previous.total)?;
    let idle = current.idle.checked_sub(previous.idle)?;
    if total == 0 || idle > total {
        return None;
    }
    Some((total - idle) as f64 / total as f64 * 100.0)
}

fn gpu_reading() -> (Vec<String>, &'static str) {
    let child = Command::new("nvidia-smi.exe")
        .args(["--id=0", GPU_QUERY, "--format=csv,noheader,nounits"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
    let Ok(mut child) = child else {
        return (vec![String::new(); 7], "unavailable");
    };
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < Duration::from_secs(3) => {
                thread::sleep(Duration::from_millis(25))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return (vec![String::new(); 7], "timeout_or_wait_failed");
            }
        }
    }
    let Ok(output) = child.wait_with_output() else {
        return (vec![String::new(); 7], "query_failed");
    };
    if !output.status.success() {
        return (vec![String::new(); 7], "query_failed");
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let Some(line) = text.lines().next() else {
        return (vec![String::new(); 7], "empty");
    };
    let mut fields: Vec<_> = line.split(',').map(|v| v.trim().to_string()).collect();
    if fields.len() != 7 {
        return (vec![String::new(); 7], "invalid_format");
    }
    let mut status = "";
    for field in &mut fields {
        if field.parse::<f64>().ok().is_none_or(|v| !v.is_finite()) {
            field.clear();
            status = "unsupported_fields";
        }
    }
    (fields, status)
}

struct ParentHandle(HANDLE);
impl Drop for ParentHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

pub fn record(directory: &Path, interval: Duration) -> Result<()> {
    let session: serde_json::Value = config::read_json(&directory.join("session.json"))?;
    let pid = session["pid"]
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
        .ok_or("Invalid session PID")?;
    let parent = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    if parent.is_null() {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let parent = ParentHandle(parent);
    let mut log = BufWriter::new(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("load.csv"))
            .map_err(|e| e.to_string())?,
    );
    writeln!(log, "{CSV_HEADER}").map_err(|e| e.to_string())?;
    log.flush().map_err(|e| e.to_string())?;
    let mut previous = cpu_times()?;
    let mut next = Instant::now() + interval;
    loop {
        // Wait on the controller handle instead of polling report.json ten times a
        // second. Controller exit still wakes the recorder immediately.
        let wait_ms = next
            .saturating_duration_since(Instant::now())
            .as_millis()
            .clamp(1, u128::from(u32::MAX - 1)) as u32;
        match unsafe { WaitForSingleObject(parent.0, wait_ms) } {
            WAIT_OBJECT_0 => break,
            WAIT_TIMEOUT => {}
            _ => return Err("Unable to check controller liveness".into()),
        }
        let report: serde_json::Value = config::read_json(&directory.join("report.json"))?;
        if report["state"].as_str() != Some("running") {
            break;
        }
        if Instant::now() < next {
            continue;
        }
        let started = Instant::now();
        let unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let current = cpu_times()?;
        let busy = busy_percent(previous, current).map_or(String::new(), |v| format!("{v:.2}"));
        previous = current;
        let (gpu, status) = gpu_reading();
        let query_ms = started.elapsed().as_millis();
        let epoch = report["started_unix"]
            .as_u64()
            .ok_or("Missing session start time")?;
        let elapsed = unix_ms as f64 / 1000.0 - epoch as f64;
        writeln!(
            log,
            "{unix_ms},{elapsed:.3},{busy},{},{query_ms},{status}",
            gpu.join(",")
        )
        .map_err(|e| e.to_string())?;
        log.flush().map_err(|e| e.to_string())?;
        next = started + interval;
    }
    log.flush().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cpu_busy_subtracts_idle_from_kernel_plus_user_time() {
        let previous = CpuTimes {
            idle: 100,
            total: 300,
        };
        let current = CpuTimes {
            idle: 150,
            total: 500,
        };
        assert_eq!(busy_percent(previous, current), Some(75.0));
        assert_eq!(busy_percent(previous, previous), None);
        assert_eq!(busy_percent(current, previous), None);
    }
}
