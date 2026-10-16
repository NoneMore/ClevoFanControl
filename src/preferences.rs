//! Committed configuration is independent of the GUI draft and runtime toggles.
use crate::config::{Result, Settings};

pub struct CommittedSettings(Option<Settings>);
impl CommittedSettings {
    pub fn new(settings: &Settings, valid: bool) -> Self {
        Self(valid.then(|| settings.clone()))
    }
    pub fn full_save(&mut self, settings: &Settings) {
        self.0 = Some(settings.clone());
    }
    pub fn recording_candidate(&self, enabled: bool) -> Result<Settings> {
        let mut candidate=self.0.clone().ok_or("Repair and explicitly save the invalid configuration before saving recording preferences")?;
        candidate.history_recording_enabled = enabled;
        Ok(candidate)
    }
    pub fn recording_saved(&mut self, enabled: bool) {
        if let Some(settings) = self.0.as_mut() {
            settings.history_recording_enabled = enabled;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preference_transactions_preserve_committed_values_and_invalid_originals() {
        let mut settings = Settings::default();
        let mut committed = CommittedSettings::new(&settings, true);
        settings.global.cpu_curve[0].duty = 70;
        settings.auto_switch = true;
        let only_recording = committed.recording_candidate(true).unwrap();
        assert_eq!(only_recording.global, Settings::default().global);
        assert!(!only_recording.auto_switch);
        committed.recording_saved(true);
        settings.history_recording_enabled = true;
        committed.full_save(&settings);
        let off = committed.recording_candidate(false).unwrap();
        assert_eq!(off.global, settings.global);
        assert!(off.auto_switch);
        assert!(!off.history_recording_enabled);
        assert!(
            CommittedSettings::new(&settings, false)
                .recording_candidate(false)
                .is_err()
        );
    }
}
