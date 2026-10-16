use crate::config::{ControlConfig, Point};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

/// Shared by CLI tuning and the GUI when a tuned config enables stableResponse.
#[derive(Default)]
pub struct StableResponse {
    history: VecDeque<(Instant, i32)>,
    filtered: Option<f64>,
    observed: Option<Instant>,
    last_change: Option<Instant>,
    decrease_since: Option<Instant>,
    boost_normal_duty: Option<i32>,
    boost_recovery_since: Option<Instant>,
}
impl StableResponse {
    pub fn boosted(&self) -> bool {
        self.boost_normal_duty.is_some()
    }
    pub fn temperature(&mut self, raw: i32, now: Instant) -> i32 {
        if self.observed == Some(now) {
            return self.filtered.unwrap_or(raw as f64).round() as i32;
        }
        self.history.push_back((now, raw));
        while self
            .history
            .front()
            .is_some_and(|(at, _)| now.saturating_duration_since(*at) > Duration::from_secs(15))
        {
            self.history.pop_front();
        }
        let mut values: Vec<_> = self.history.iter().map(|(_, t)| *t).collect();
        values.sort_unstable();
        let median = values[values.len() / 2] as f64;
        let dt = self
            .observed
            .map_or(2.0, |at| now.saturating_duration_since(at).as_secs_f64());
        let alpha = 1.0 - (-dt / 30.0).exp();
        self.filtered = Some(
            self.filtered
                .map_or(median, |old| old + alpha * (median - old)),
        );
        self.observed = Some(now);
        self.filtered.unwrap().round() as i32
    }
    pub fn duty(
        &mut self,
        current: i32,
        target: i32,
        raw: i32,
        channel: usize,
        now: Instant,
    ) -> i32 {
        // Raw temperatures bypass all filtering/delays near the high-temperature guard.
        let guard = if channel == 0 { 80 } else { 75 };
        if raw >= guard {
            self.boost_normal_duty.get_or_insert(current);
            self.boost_recovery_since = None;
            self.last_change = Some(now);
            self.decrease_since = None;
            return 100;
        }
        if let Some(normal) = self.boost_normal_duty {
            if raw >= guard - 5 {
                self.boost_recovery_since = None;
                return 100;
            }
            let since = *self.boost_recovery_since.get_or_insert(now);
            if now.saturating_duration_since(since) < Duration::from_secs(30) {
                return 100;
            }
            // End a temporary safety boost separately from ordinary slow ramping.
            // Preserve at least the pre-boost cooling, or more if the curve needs it.
            self.boost_normal_duty = None;
            self.boost_recovery_since = None;
            self.last_change = Some(now);
            self.decrease_since = None;
            return normal.max(target).clamp(0, 100);
        }
        if self.last_change.is_none() {
            // Firmware can report stopped or near-full duty on takeover/resume.
            // Establish the curve demand before applying normal rate limits;
            // raw high-temperature protection above still takes precedence.
            self.last_change = Some(now);
            return target.clamp(0, 100);
        }
        let last = self.last_change.unwrap();
        let difference = target - current;
        if difference.abs() <= 2 {
            self.decrease_since = None;
            return current;
        }
        if difference < 0 {
            let since = *self.decrease_since.get_or_insert(now);
            if now.saturating_duration_since(since) < Duration::from_secs(120) {
                return current;
            }
        } else {
            self.decrease_since = None;
        }
        let warm = difference > 0
            && self
                .filtered
                .is_some_and(|t| t >= if channel == 0 { 70.0 } else { 65.0 });
        if now.saturating_duration_since(last) < Duration::from_secs(if warm { 10 } else { 30 }) {
            return current;
        }
        self.last_change = Some(now);
        current + difference.clamp(-1, if warm { 4 } else { 2 })
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct ChannelState {
    pub level: usize,
    pub effective: Option<i32>,
}
pub fn linear(curve: &[Point], t: i32) -> i32 {
    if t <= curve[0].temperature {
        return curve[0].duty;
    }
    for p in curve.windows(2) {
        if t <= p[1].temperature {
            return (p[0].duty as f64
                + (p[1].duty - p[0].duty) as f64 * (t - p[0].temperature) as f64
                    / (p[1].temperature - p[0].temperature) as f64)
                .round() as i32;
        }
    }
    curve.last().unwrap().duty
}
impl ChannelState {
    pub fn evaluate(&mut self, curve: &[Point], t: i32, c: &ControlConfig) -> i32 {
        if c.linear {
            let e = self
                .effective
                .unwrap_or(t)
                .max(t)
                .min(t + c.transition_temp);
            self.effective = Some(e);
            self.level = curve.iter().rposition(|p| p.temperature <= e).unwrap_or(0);
            linear(curve, e)
        } else {
            self.level = self.level.min(curve.len() - 1);
            while self.level + 1 < curve.len() && t >= curve[self.level + 1].temperature {
                self.level += 1;
            }
            while self.level > 0 && t < curve[self.level].temperature - c.transition_temp {
                self.level -= 1;
            }
            curve[self.level].duty
        }
    }
}
pub fn cool_complete(temps: &[i32], threshold: i32) -> bool {
    !temps.is_empty() && temps.iter().all(|t| *t < threshold)
}
pub fn ramp(current: i32, target: i32) -> i32 {
    current + (target - current).signum()
}
pub fn to_raw(percent: i32) -> u8 {
    ((percent.clamp(0, 100) * 255 + 50) / 100) as u8
}
pub fn to_percent(raw: u8) -> i32 {
    (raw as i32 * 100 + 127) / 255
}
