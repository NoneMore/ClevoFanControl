//! Bounded, slow local adaptation for ordinary workloads; unobserved high temperatures
//! retain the seed curve. This does not identify a full thermal model or measure noise.
use crate::{
    config::{ControlConfig, Point},
    hardware::Sample,
};
use serde::Serialize;
use std::{collections::VecDeque, time::Duration};

pub const LIMITS: [i32; 2] = [85, 80];
pub const FLOORS: [i32; 2] = [25, 40];
const PROTECTED: [i32; 2] = [80, 75];
const WINDOW: f64 = 180.0;
// A full window at the supported 10-second interval contains 18-19 samples.
const MIN_WINDOW_SAMPLES: usize = 12;

pub fn seed_control() -> ControlConfig {
    let points = |values: &[(i32, i32)]| {
        values
            .iter()
            .map(|&(temperature, duty)| Point { temperature, duty })
            .collect()
    };
    ControlConfig {
        cpu_curve: points(&[
            (35, 25),
            (45, 25),
            (55, 30),
            (65, 35),
            (75, 60),
            (80, 80),
            (85, 100),
            (100, 100),
        ]),
        gpu_curve: points(&[
            (30, 40),
            (40, 40),
            (50, 45),
            (60, 55),
            (70, 70),
            (75, 85),
            (80, 100),
            (100, 100),
        ]),
        linear: true,
        transition_temp: 3,
        take_over: false,
        soft_control: false,
        stable_response: true,
        ..ControlConfig::default()
    }
}

#[derive(Debug, Serialize)]
pub struct Adjustment {
    pub channel: usize,
    pub temperature: i32,
    pub delta: i32,
    pub reason: &'static str,
}

pub struct Tuner {
    pub control: ControlConfig,
    pub targets: [i32; 2],
    history: VecDeque<(f64, [i32; 2])>,
    last_adjust: f64,
}
impl Tuner {
    pub fn new(targets: [i32; 2]) -> Self {
        Self {
            control: seed_control(),
            targets,
            history: VecDeque::new(),
            last_adjust: 0.0,
        }
    }
    pub fn observe(&mut self, elapsed: Duration, temperatures: [i32; 2]) -> Vec<Adjustment> {
        let now = elapsed.as_secs_f64();
        self.history.push_back((now, temperatures));
        while self
            .history
            .front()
            .is_some_and(|(at, _)| now - at > WINDOW + 5.0)
        {
            self.history.pop_front();
        }
        if now - self.last_adjust < WINDOW
            || self.history.len() < MIN_WINDOW_SAMPLES
            || self
                .history
                .front()
                .is_none_or(|(at, _)| now - at < WINDOW - 5.0)
        {
            return vec![];
        }
        self.last_adjust = now;
        let mut changes = vec![];
        for ch in 0..2 {
            let mut values: Vec<_> = self.history.iter().map(|(_, t)| t[ch]).collect();
            let n = values.len();
            let section = (n / 3).max(1);
            let first = values[..section].iter().sum::<i32>() as f64 / section as f64;
            let last = values[n - section..].iter().sum::<i32>() as f64 / section as f64;
            values.sort_unstable();
            let median = values[n / 2];
            let p95 = values[(n - 1) * 95 / 100];
            let maximum = values[n - 1];
            let upper = self.targets[ch] + if ch == 0 { 5 } else { 2 };
            let (delta, reason) =
                if p95 > upper || (last - first > 2.0 && last > (upper - 1) as f64) {
                    (4, "temperature above target or rising")
                } else if maximum < self.targets[ch] - 4 && (last - first).abs() <= 1.0 {
                    (-2, "stable below target")
                } else {
                    continue;
                };
            let curve = if ch == 0 {
                &mut self.control.cpu_curve
            } else {
                &mut self.control.gpu_curve
            };
            if adjust_local(curve, median, delta, FLOORS[ch], PROTECTED[ch]) {
                changes.push(Adjustment {
                    channel: ch + 1,
                    temperature: median,
                    delta,
                    reason,
                });
            }
        }
        self.history.clear();
        changes
    }
}

fn adjust_local(
    curve: &mut [Point],
    temperature: i32,
    delta: i32,
    floor: i32,
    protected: i32,
) -> bool {
    let lower = curve
        .iter()
        .rposition(|p| p.temperature <= temperature)
        .unwrap_or(0);
    let upper = (lower + 1).min(curve.len() - 1);
    let old = curve.to_vec();
    // Only adapt interpolation nodes adjacent to the observed median temperature.
    for point in curve.iter_mut().take(upper + 1).skip(lower) {
        if point.temperature >= protected {
            continue;
        }
        point.duty = (point.duty + delta).clamp(floor, 100);
    }
    // Project back into the monotone envelope without changing untouched nodes.
    for i in (lower..=upper).rev() {
        if i + 1 < curve.len() {
            curve[i].duty = curve[i].duty.min(curve[i + 1].duty);
        }
    }
    for i in lower..=upper {
        if i > 0 {
            curve[i].duty = curve[i].duty.max(curve[i - 1].duty);
        }
    }
    curve != old
}

pub fn validate_sample(sample: &Sample, expected_channels: Option<usize>) -> Result<(), String> {
    if sample.channels.len() != 2 || expected_channels.is_some_and(|n| n != sample.channels.len()) {
        return Err("CLI tuning currently requires exactly two physical fans".into());
    }
    for (ch, reading) in sample.channels.iter().enumerate() {
        if !(1..=125).contains(&reading.temperature) || !(0..=100).contains(&reading.duty) {
            return Err(format!("Invalid telemetry on channel {}", ch + 1));
        }
    }
    Ok(())
}

pub fn check_temperature(sample: &Sample) -> Result<(), String> {
    validate_sample(sample, Some(2))?;
    for (ch, reading) in sample.channels.iter().enumerate() {
        if reading.temperature >= LIMITS[ch] {
            return Err(format!(
                "Channel {} reached session limit {} C (measured {} C)",
                ch + 1,
                LIMITS[ch],
                reading.temperature
            ));
        }
    }
    Ok(())
}

#[derive(Default, Serialize)]
pub struct ChannelStats {
    pub samples: u64,
    pub min_temperature: Option<i32>,
    pub max_temperature: Option<i32>,
    pub temperature_sum: i64,
    pub duty_sum: i64,
    pub rpm_sum: u64,
    pub rpm_samples: u64,
    pub duty_changes: u64,
    pub total_duty_movement: u64,
    #[serde(skip)]
    previous_duty: Option<i32>,
    pub temperature_histogram: std::collections::BTreeMap<i32, u64>,
}
impl ChannelStats {
    pub fn add(&mut self, reading: &crate::hardware::Reading) {
        self.samples += 1;
        self.min_temperature = Some(
            self.min_temperature
                .map_or(reading.temperature, |t| t.min(reading.temperature)),
        );
        self.max_temperature = Some(
            self.max_temperature
                .map_or(reading.temperature, |t| t.max(reading.temperature)),
        );
        self.temperature_sum += i64::from(reading.temperature);
        self.duty_sum += i64::from(reading.duty);
        if let Some(previous) = self.previous_duty {
            let movement = (previous - reading.duty).unsigned_abs() as u64;
            self.total_duty_movement += movement;
            self.duty_changes += u64::from(movement > 0);
        }
        self.previous_duty = Some(reading.duty);
        if let Some(rpm) = reading.rpm {
            self.rpm_sum += u64::from(rpm);
            self.rpm_samples += 1;
        }
        *self
            .temperature_histogram
            .entry(reading.temperature)
            .or_default() += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::Reading;
    #[test]
    fn ten_second_sampling_can_complete_an_adaptation_window() {
        let mut tuner = Tuner::new([60, 50]);
        let original = tuner.control.clone();
        for at in (0..180).step_by(10) {
            assert!(tuner.observe(Duration::from_secs(at), [70, 55]).is_empty());
        }
        let adjustments = tuner.observe(Duration::from_secs(180), [70, 55]);
        assert_eq!(adjustments.len(), 2);
        assert!(adjustments.iter().all(|change| change.delta > 0));
        assert!(tuner.control.cpu_curve[4].duty > original.cpu_curve[4].duty);
        assert!(tuner.control.gpu_curve[2].duty > original.gpu_curve[2].duty);
    }
    #[test]
    fn cpu_fallback_band_holds_the_curve_instead_of_chasing_sixty() {
        let mut tuner = Tuner::new([60, 50]);
        let original = tuner.control.clone();
        for at in (0..=180).step_by(2) {
            assert!(tuner.observe(Duration::from_secs(at), [63, 50]).is_empty());
        }
        assert_eq!(tuner.control, original);
    }
    fn readings(temps: [i32; 2]) -> Sample {
        Sample {
            channels: temps
                .into_iter()
                .map(|temperature| Reading {
                    temperature,
                    duty: 40,
                    rpm: Some(2000),
                })
                .collect(),
            at: std::time::Instant::now(),
        }
    }
    #[test]
    fn stable_low_temperature_adapts_locally_and_keeps_high_guard() {
        let mut tuner = Tuner::new([75, 70]);
        let original = tuner.control.clone();
        for at in (0..=180).step_by(2) {
            let changes = tuner.observe(Duration::from_secs(at), [60, 50]);
            if at < 180 {
                assert!(changes.is_empty());
            } else {
                assert_eq!(changes.len(), 2);
            }
        }
        assert_eq!(tuner.control.cpu_curve[0], original.cpu_curve[0]);
        assert_eq!(tuner.control.cpu_curve[2].duty, 28);
        assert_eq!(tuner.control.cpu_curve[3].duty, 33);
        for (before, after) in original
            .cpu_curve
            .iter()
            .zip(&tuner.control.cpu_curve)
            .filter(|(p, _)| p.temperature >= 80)
        {
            assert_eq!(before, after);
        }
        tuner.control.validate().unwrap();
    }
    #[test]
    fn repeated_adaptation_respects_floors_and_monotonicity() {
        let mut tuner = Tuner::new([75, 70]);
        for at in (0..14400).step_by(2) {
            tuner.observe(Duration::from_secs(at), [60, 50]);
        }
        for (ch, curve) in [&tuner.control.cpu_curve, &tuner.control.gpu_curve]
            .into_iter()
            .enumerate()
        {
            assert!(curve.iter().all(|p| p.duty >= FLOORS[ch]));
            assert!(curve.windows(2).all(|p| p[0].duty <= p[1].duty));
            assert_eq!(curve.last().unwrap().duty, 100);
        }
    }
    #[test]
    fn rising_temperature_does_not_reduce_cooling() {
        let mut tuner = Tuner::new([75, 70]);
        for at in (0..=180).step_by(2) {
            tuner.observe(
                Duration::from_secs(at),
                [70 + (at / 25) as i32, 65 + (at / 25) as i32],
            );
        }
        assert!(tuner.control.cpu_curve[4].duty >= 60);
        assert!(tuner.control.gpu_curve[4].duty >= 70);
    }
    #[test]
    fn temperature_limits_and_unsupported_channel_counts_fail_closed() {
        assert!(check_temperature(&readings([84, 79])).is_ok());
        assert!(check_temperature(&readings([85, 60])).is_err());
        assert!(check_temperature(&readings([60, 80])).is_err());
        let mut s = readings([50, 50]);
        s.channels.pop();
        assert!(check_temperature(&s).is_err());
    }
}
