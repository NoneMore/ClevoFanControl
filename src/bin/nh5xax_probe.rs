use clevo_fan_control::{control::to_percent, hardware::decode_rpm, platform};
use libloading::Library;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

type GetBuffer = unsafe extern "system" fn(u32, *mut u8) -> i32;

const EXPECTED_SHA256: &str =
    "38aea56871649d1aec8d732efcfadb52142eae337f360e010d516b8457b763a6";
const DEFAULT_DLL: &str = r"C:\Program Files (x86)\ControlCenter\DCHU\InsydeDCHU.dll";

fn main() {
    if let Err(error) = run() {
        eprintln!("ERROR: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    println!("NH5xAx read-only probe");
    println!("This build only resolves GetDCHU_Data_Buffer and only issues read commands 0x0D and 0x0C.");
    println!("SetDCHU_Data is never resolved, so this probe cannot change fan duty or fan mode.\n");

    let machine = platform::machine()?;
    if !machine.eq_ignore_ascii_case("NH5xAx") {
        return Err(format!(
            "refusing to run on unexpected SystemProductName: {machine:?}"
        ));
    }

    let dll_path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_DLL));
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

    let table = read_buffer(get, 0x0d)?;
    let count = table[12] as usize;
    if !(1..=3).contains(&count) {
        return Err(format!("unexpected fan count from package 0x0D: {count}"));
    }

    let live = read_buffer(get, 0x0c)?;

    println!("Machine : {machine}");
    println!("DLL     : {}", dll_path.display());
    println!("SHA256  : {hash}");
    println!("Fans    : {count}\n");

    for i in 0..count {
        let name = match i {
            0 => "CPU",
            1 => "GPU1",
            2 => "GPU2",
            _ => "Fan",
        };
        let counter = u16::from_be_bytes([live[2 + i * 2], live[3 + i * 2]]);
        let raw_duty = live[16 + i * 3];
        let temperature = live[18 + i * 3];
        let rpm = decode_rpm(counter);

        println!(
            "{name:<4} temp={temperature:>3} C  duty_raw=0x{raw_duty:02X}  duty={:>3}%  tach=0x{counter:04X}  rpm={}",
            to_percent(raw_duty),
            rpm.map_or_else(|| "invalid".to_string(), |value| value.to_string())
        );
    }

    print!("\nPackage 0x0C bytes 0..31:");
    for (i, byte) in live[..32].iter().enumerate() {
        if i % 16 == 0 {
            print!("\n  {i:02X}: ");
        }
        print!("{byte:02X} ");
    }
    println!();

    // Keep the library alive until all function calls have completed.
    drop(library);
    Ok(())
}

fn read_buffer(get: GetBuffer, command: u32) -> Result<[u8; 4096], String> {
    let mut buffer = [0xa5; 4096];
    let ret = unsafe { get(command, buffer.as_mut_ptr()) };
    if ret != command as i32 || buffer[..256].iter().all(|byte| *byte == 0xa5) {
        return Err(format!(
            "GetDCHU_Data_Buffer({command:#04x}) failed: return={ret:#x}"
        ));
    }
    Ok(buffer)
}
