use crate::{
    config::{CompatibilityConfig, Result},
    control::{to_percent, to_raw},
};
use libloading::Library;
use sha2::{Digest, Sha256};
use std::{path::PathBuf, time::Instant};

type GetBuffer = unsafe extern "system" fn(u32, *mut u8) -> i32;
type SetData = unsafe extern "system" fn(u32, *const u8, u32) -> i32;

const NH5XAX_DLL_SHA256: &str = "38aea56871649d1aec8d732efcfadb52142eae337f360e010d516b8457b763a6";
const NH5XAX_FALLBACK_DLL: &str = r"C:\Program Files (x86)\ControlCenter\DCHU\InsydeDCHU.dll";

fn is_verified_hash(hash: &str) -> bool {
    [
        "75a47020d3a9d052e94dcb4e3ad61fb69f20177dd46e20c9a20883d92b83981b",
        "22fecadff27f4bf08cb4a17fe455ae490107b409e55827210f821947d65a9d47",
        NH5XAX_DLL_SHA256,
    ]
    .contains(&hash)
}

#[derive(Clone, Debug)]
pub struct Reading {
    pub temperature: i32,
    pub duty: i32,
    pub rpm: Option<u32>,
}
#[derive(Clone, Debug)]
pub struct Sample {
    pub channels: Vec<Reading>,
    pub at: Instant,
}
pub trait Hardware: Send {
    fn sample(&mut self) -> Result<Sample>;
    fn write(&mut self, duties: &[i32]) -> Result<()>;
    fn automatic(&mut self) -> Result<()>;
    fn description(&self) -> String;
}
pub struct Insyde {
    get: GetBuffer,
    set: Option<SetData>,
    _library: Library,
    count: usize,
    description: String,
}
impl Insyde {
    pub fn resolve(compatibility: &CompatibilityConfig) -> Result<(String, PathBuf)> {
        let machine = crate::platform::machine()?;
        Self::resolve_for_machine(compatibility, machine)
    }
    fn resolve_for_machine(
        compatibility: &CompatibilityConfig,
        machine: String,
    ) -> Result<(String, PathBuf)> {
        if !compatibility.matches_machine(&machine)? {
            return Err(format!("当前机型 {machine} 未匹配机型白名单"));
        }
        let path = match &compatibility.dll_path {
            Some(p) => p.clone(),
            None if machine.eq_ignore_ascii_case("NH5xAx") => {
                let p = PathBuf::from(NH5XAX_FALLBACK_DLL);
                if p.is_file() {
                    p
                } else {
                    crate::platform::find_dll()?
                }
            }
            None => crate::platform::find_dll()?,
        };
        if !path.is_absolute() {
            return Err("DLL 必须使用绝对路径".into());
        }
        if !path.is_file() {
            return Err(format!("找不到 DLL：{}", path.display()));
        }
        Ok((machine, path))
    }
    pub fn open(compatibility: &CompatibilityConfig) -> Result<Self> {
        Self::open_inner(compatibility, false)
    }
    pub fn open_probe_unverified(compatibility: &CompatibilityConfig) -> Result<Self> {
        Self::open_inner(compatibility, true)
    }
    fn open_inner(compatibility: &CompatibilityConfig, read_only_probe: bool) -> Result<Self> {
        let (machine, path) = Self::resolve(compatibility)?;
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        let hash = format!("{:x}", Sha256::digest(&bytes));
        let verified = is_verified_hash(&hash);
        if !verified && !read_only_probe {
            return Err(format!(
                "DLL 版本尚未验证：{}，SHA256 {hash}",
                path.display()
            ));
        }
        let library =
            unsafe { libloading::os::windows::Library::load_with_flags(&path, 0x100 | 0x800) }
                .map_err(|e| e.to_string())?;
        let library: Library = library.into();
        let get = unsafe {
            *library
                .get::<GetBuffer>(b"GetDCHU_Data_Buffer\0")
                .map_err(|e| e.to_string())?
        };
        let set = if read_only_probe {
            None
        } else {
            Some(unsafe {
                *library
                    .get::<SetData>(b"SetDCHU_Data\0")
                    .map_err(|e| e.to_string())?
            })
        };
        let mode = if read_only_probe {
            if verified {
                "只读探测（已验证 DLL）"
            } else {
                "只读探测（未验证 DLL）"
            }
        } else {
            "已验证 DLL"
        };
        let mut s = Self {
            get,
            set,
            _library: library,
            count: 0,
            description: format!("{machine}\n{}\nSHA256 {hash}\n{mode}", path.display()),
        };
        let b = s.buffer(13)?;
        s.count = b[12] as usize;
        if !(1..=3).contains(&s.count) {
            return Err(format!("风扇数量未知：{}", s.count));
        }
        s.sample()?;
        Ok(s)
    }
    fn buffer(&mut self, cmd: u32) -> Result<[u8; 4096]> {
        // DLL copies the ACPI buffer plus a trailing byte; allocate beyond its 256-byte payload.
        let mut b = [0xa5; 4096];
        let ret = unsafe { (self.get)(cmd, b.as_mut_ptr()) };
        if ret != cmd as i32 || b[..256].iter().all(|v| *v == 0xa5) {
            return Err(format!("读取 EC 命令 {cmd:#x} 失败（{ret:#x}）"));
        }
        Ok(b)
    }
}
impl Hardware for Insyde {
    fn sample(&mut self) -> Result<Sample> {
        let b = self.buffer(12)?;
        let mut channels = vec![];
        for i in 0..self.count {
            let temperature = b[18 + i * 3] as i32;
            if !(1..=125).contains(&temperature) {
                return Err(format!("通道 {} 温度无效：{temperature}", i + 1));
            }
            channels.push(Reading {
                temperature,
                duty: to_percent(b[16 + i * 3]),
                rpm: decode_rpm(u16::from_be_bytes([b[2 + i * 2], b[3 + i * 2]])),
            });
        }
        Ok(Sample {
            channels,
            at: Instant::now(),
        })
    }
    fn write(&mut self, duties: &[i32]) -> Result<()> {
        let set = self
            .set
            .ok_or_else(|| "只读探测模式禁止写入风扇".to_string())?;
        if duties.len() != self.count || duties.iter().any(|v| !(0..=100).contains(v)) {
            return Err("写入通道或占空比无效".into());
        }
        // PK04(1) forwards one EC C1 command. Unlike 0x68 this does not touch unknown channels.
        for (i, duty) in duties.iter().enumerate() {
            let mut b = [0u8; 256];
            b[0] = 1;
            b[1] = (i + 1) as u8;
            b[2] = to_raw(*duty);
            b[6] = 0xc1;
            let ret = unsafe { set(4, b.as_ptr(), 256) };
            if ret != 4 {
                return Err(format!("EC 风扇 {} 写入失败（{ret:#x}）", i + 1));
            }
        }
        Ok(())
    }
    fn automatic(&mut self) -> Result<()> {
        let set = self
            .set
            .ok_or_else(|| "只读探测模式禁止修改自动控制状态".to_string())?;
        let mask = ((1u32 << self.count) - 1).to_le_bytes();
        let ret = unsafe { set(0x69, mask.as_ptr(), 4) };
        if ret != 0x69 {
            Err(format!("恢复固件自动失败（{ret:#x}）"))
        } else {
            Ok(())
        }
    }
    fn description(&self) -> String {
        self.description.clone()
    }
}
pub struct Demo {
    duty: [i32; 2],
    start: Instant,
}
impl Default for Demo {
    fn default() -> Self {
        Self {
            duty: [30, 30],
            start: Instant::now(),
        }
    }
}
impl Hardware for Demo {
    fn sample(&mut self) -> Result<Sample> {
        let t = self.start.elapsed().as_secs_f64();
        Ok(Sample {
            at: Instant::now(),
            channels: (0..2)
                .map(|i| Reading {
                    temperature: (55. + (t / 12. + i as f64).sin() * 10.) as i32,
                    duty: self.duty[i],
                    rpm: Some((self.duty[i] * 45) as u32),
                })
                .collect(),
        })
    }
    fn write(&mut self, d: &[i32]) -> Result<()> {
        self.duty.copy_from_slice(d);
        Ok(())
    }
    fn automatic(&mut self) -> Result<()> {
        self.duty = [30, 30];
        Ok(())
    }
    fn description(&self) -> String {
        "演示模式 · 模拟数据，不连接 EC".into()
    }
}

// Official FanSpeedSetting UI: round(60 / (counter * 5.565217391304348e-5) * 2).
pub fn decode_rpm(counter: u16) -> Option<u32> {
    if counter == 0 {
        return Some(0);
    }
    let rpm = (2_156_250.0 / f64::from(counter)).round() as u32;
    (rpm <= 20000).then_some(rpm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_hashes_are_verified() {
        assert!(is_verified_hash(
            "75a47020d3a9d052e94dcb4e3ad61fb69f20177dd46e20c9a20883d92b83981b"
        ));
        assert!(is_verified_hash(NH5XAX_DLL_SHA256));
        assert!(!is_verified_hash(
            "430d522c811a5df1037d07714c47316967c015391985e3f9a0915eefc46d328f"
        ));
    }

    #[test]
    fn custom_dll_resolution_checks_the_model_and_file_without_loading() {
        let path = std::env::temp_dir().join(format!("clevo-resolve-{}.dll", std::process::id()));
        std::fs::write(&path, b"resolution must not load this file").unwrap();
        let c = CompatibilityConfig {
            model_whitelist: vec!["^P16 Pro IXA1$".into()],
            dll_path: Some(path.clone()),
        };
        assert_eq!(
            Insyde::resolve_for_machine(&c, "P16 Pro IXA1".into()).unwrap(),
            ("P16 Pro IXA1".into(), path.clone())
        );
        assert!(
            Insyde::resolve_for_machine(&c, "Other model".into())
                .unwrap_err()
                .contains("未匹配")
        );
        std::fs::remove_file(path).unwrap();
        assert!(
            Insyde::resolve_for_machine(&c, "P16 Pro IXA1".into())
                .unwrap_err()
                .contains("找不到 DLL")
        );
    }
}
