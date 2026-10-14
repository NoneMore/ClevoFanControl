use clevo_fan_control::{control::{to_percent, to_raw}, hardware::decode_rpm, platform};
use libloading::Library;
use sha2::{Digest, Sha256};
use std::{path::PathBuf, thread, time::Duration};

type GetBuffer = unsafe extern "system" fn(u32, *mut u8) -> i32;
type SetData = unsafe extern "system" fn(u32, *const u8, u32) -> i32;

const EXPECTED_SHA256: &str =
    "38aea56871649d1aec8d732efcfadb52142eae337f360e010d516b8457b763a6";
const DEFAULT_DLL: &str = r"C:\Program Files (x86)\ControlCenter\DCHU\InsydeDCHU.dll";
const TEST_DUTY_PERCENT: i32 = 70;
const HOLD_SECONDS: u64 = 2;
const CONFIRM_FLAG: &str = "--confirm-write-test";

#[derive(Clone, Debug)]
struct Reading {
    temperature: u8,
    raw_duty: u8,
    duty_percent: i32,
    tach_counter: u16,
    rpm: Option<u32>,
}

struct Tester {
    get: GetBuffer,
    set: SetData,
    _library: Library,
    count: usize,
    restore_armed: bool,
}

impl Tester {
    fn open(dll_path: PathBuf) -> Result<Self, String> {
        if !dll_path.is_absolute() || !dll_path.is_file() {
            return Err(format!("DLL not found: {}", dll_path.display()));
        }

        let bytes = std::fs::read(&dll_path).map_err(|e| e.to_string())?;
        let hash = format!("{:x}", Sha256::digest(&bytes));
        if hash != EXPECTED_SHA256 {
            return Err(format!(
                "refusing unverified DLL: {}\nexpected SHA256 {EXPECTED_SHA256}\nactual   SHA256 {hash}",
                dll_path.display()
            ));
        }

        let library = unsafe {
            libloading::os::windows::Library::load_with_flags(&dll_path, 0x100 | 0x800)
        }
        .map_err(|e| e.to_string())?;
        let library: Library = library.into();
        let get = unsafe {
            *library
                .get::<GetBuffer>(b"GetDCHU_Data_Buffer\0")
                .map_err(|e| e.to_string())?
        };
        let set = unsafe {
            *library
                .get::<SetData>(b"SetDCHU_Data\0")
                .map_err(|e| e.to_string())?
        };

        let mut tester = Self {
            get,
            set,
            _library: library,
            count: 0,
            restore_armed: false,
        };

        let table = tester.read_buffer(0x0d)?;
        tester.count = table[12] as usize;
        if tester.count != 2 {
            return Err(format!(
                "refusing write test: expected exactly 2 fans, package 0x0D reported {}",
                tester.count
            ));
        }

        // Confirm the live package is sane before any write is possible.
        tester.sample()?;
        Ok(tester)
    }

    fn read_buffer(&mut self, command: u32) -> Result<[u8; 4096], String> {
        let mut buffer = [0xa5; 4096];
        let ret = unsafe { (self.get)(command, buffer.as_mut_ptr()) };
        if ret != command as i32 || buffer[..256].iter().all(|byte| *byte == 0xa5) {
            return Err(format!(
                "GetDCHU_Data_Buffer({command:#04x}) failed: return={ret:#x}"
            ));
        }
        Ok(buffer)
    }

    fn sample(&mut self) -> Result<Vec<Reading>, String> {
        let live = self.read_buffer(0x0c)?;
        let mut readings = Vec::with_capacity(self.count);
        for i in 0..self.count {
            let temperature = live[18 + i * 3];
            if !(1..=125).contains(&temperature) {
                return Err(format!(
                    "channel {} reported invalid temperature: {temperature}",
                    i + 1
                ));
            }
            let raw_duty = live[16 + i * 3];
            let tach_counter = u16::from_be_bytes([live[2 + i * 2], live[3 + i * 2]]);
            readings.push(Reading {
                temperature,
                raw_duty,
                duty_percent: to_percent(raw_duty),
                tach_counter,
                rpm: decode_rpm(tach_counter),
            });
        }
        Ok(readings)
    }

    fn set_channel(&mut self, channel: usize, duty_percent: i32) -> Result<(), String> {
        if channel >= self.count {
            return Err(format!("invalid channel index: {channel}"));
        }
        if duty_percent != TEST_DUTY_PERCENT {
            return Err(format!(
                "this guarded build only permits the fixed {TEST_DUTY_PERCENT}% test duty"
            ));
        }

        let mut buffer = [0u8; 256];
        buffer[0] = 1;
        buffer[1] = (channel + 1) as u8;
        buffer[2] = to_raw(duty_percent);
        buffer[6] = 0xc1;

        // Arm rollback before making the vendor call: even a surprising return code
        // should cause the normal error path / Drop handler to attempt Auto mode.
        self.restore_armed = true;
        let ret = unsafe { (self.set)(4, buffer.as_ptr(), buffer.len() as u32) };
        if ret != 4 {
            return Err(format!(
                "SetDCHU_Data(4) for channel {} failed: return={ret:#x}",
                channel + 1
            ));
        }
        Ok(())
    }

    fn restore_auto(&mut self) -> Result<(), String> {
        let mask = ((1u32 << self.count) - 1).to_le_bytes();
        let ret = unsafe { (self.set)(0x69, mask.as_ptr(), mask.len() as u32) };
        if ret != 0x69 {
            self.restore_armed = true;
            return Err(format!(
                "SetDCHU_Data(0x69) restore-auto failed: return={ret:#x}"
            ));
        }
        self.restore_armed = false;
        Ok(())
    }
}

impl Drop for Tester {
    fn drop(&mut self) {
        if self.restore_armed {
            let mask = ((1u32 << self.count) - 1).to_le_bytes();
            let ret = unsafe { (self.set)(0x69, mask.as_ptr(), mask.len() as u32) };
            if ret != 0x69 {
                eprintln!(
                    "WARNING: final best-effort restore-auto failed: return={ret:#x}. Reboot or use a known-good fan tool if fan control appears stuck."
                );
            }
        }
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("ERROR: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let confirmed = args.iter().any(|arg| arg == CONFIRM_FLAG);
    let cpu = args.iter().any(|arg| arg == "--cpu");
    let gpu = args.iter().any(|arg| arg == "--gpu");

    if !confirmed || cpu == gpu {
        return Err(format!(
            "usage: nh5xax_write_test (--cpu | --gpu) {CONFIRM_FLAG}\n\nThis test writes exactly one fan channel to {TEST_DUTY_PERCENT}% for {HOLD_SECONDS} seconds, reads it back, then restores BOTH channels to firmware Auto. Run CPU first and inspect the result before testing GPU."
        ));
    }

    println!("NH5xAx guarded write test");
    println!("WARNING: this program WILL change one fan channel temporarily.");
    println!("Fixed test: {TEST_DUTY_PERCENT}% for {HOLD_SECONDS} seconds, then restore both fans to firmware Auto.");
    println!("Do not run another fan-control utility at the same time and do not interrupt the 2-second test window.\n");

    let machine = platform::machine()?;
    if !machine.eq_ignore_ascii_case("NH5xAx") {
        return Err(format!(
            "refusing to run on unexpected SystemProductName: {machine:?}"
        ));
    }

    let dll_path = args
        .iter()
        .find_map(|arg| arg.strip_prefix("--dll=").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(DEFAULT_DLL));

    let mut tester = Tester::open(dll_path)?;
    let target = if cpu { 0 } else { 1 };
    let target_name = if cpu { "CPU" } else { "GPU1" };

    let before = tester.sample()?;
    print_sample("Before", &before);

    println!(
        "\nWriting {target_name} (channel {}) to {TEST_DUTY_PERCENT}%...",
        target + 1
    );
    tester.set_channel(target, TEST_DUTY_PERCENT)?;
    thread::sleep(Duration::from_secs(HOLD_SECONDS));

    let during = tester.sample()?;
    print_sample("During 70% test", &during);

    let observed = during[target].duty_percent;
    let duty_ok = (observed - TEST_DUTY_PERCENT).abs() <= 2;

    println!("\nRestoring BOTH fan channels to firmware Auto...");
    tester.restore_auto()?;
    thread::sleep(Duration::from_secs(HOLD_SECONDS));

    let after = tester.sample()?;
    print_sample("After restore", &after);

    if !duty_ok {
        return Err(format!(
            "target channel readback did not match {TEST_DUTY_PERCENT}% (observed {observed}%). Auto restore was attempted successfully; stop here and do not test the other channel yet."
        ));
    }

    println!(
        "\nPASS: {target_name} channel accepted the fixed {TEST_DUTY_PERCENT}% duty and both channels were returned to firmware Auto."
    );
    println!("If this was the CPU test, inspect the RPM/duty output before running the GPU test.");
    Ok(())
}

fn print_sample(label: &str, readings: &[Reading]) {
    println!("\n{label}:");
    for (i, reading) in readings.iter().enumerate() {
        let name = match i {
            0 => "CPU",
            1 => "GPU1",
            _ => "Fan",
        };
        println!(
            "  {name:<4} temp={:>3} C  duty_raw=0x{:02X}  duty={:>3}%  tach=0x{:04X}  rpm={}",
            reading.temperature,
            reading.raw_duty,
            reading.duty_percent,
            reading.tach_counter,
            reading
                .rpm
                .map_or_else(|| "invalid".to_string(), |value| value.to_string())
        );
    }
}
