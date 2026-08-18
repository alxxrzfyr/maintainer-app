//! SMART storage health types and assessment logic.
//!
//! Health wording is bounded by source evidence:
//! - `Passed` — drive reports healthy
//! - `Warning reported by drive` — pre-fail attribute threshold reached
//! - `Failure reported by drive` — drive reports failure
//! - `Data stale` — last SMART test too old
//! - `Unavailable` — no SMART support or interface
//!
//! This module NEVER predicts remaining drive life and NEVER labels
//! missing SMART as healthy.

use std::time::{Duration, SystemTime};

/// SMART health assessment with bounded, evidence-based wording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmartAssessment {
    /// Drive self-test reports healthy.
    Passed,
    /// Pre-fail attribute has reached threshold.
    WarningReportedByDrive,
    /// Drive self-test reports failure.
    FailureReportedByDrive,
    /// SMART data is too old to be meaningful.
    DataStale,
    /// No SMART support or interface available for this drive.
    Unavailable,
}

impl SmartAssessment {
    /// Human-readable label for the assessment.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Passed => "Passed",
            Self::WarningReportedByDrive => "Warning reported by drive",
            Self::FailureReportedByDrive => "Failure reported by drive",
            Self::DataStale => "Data stale",
            Self::Unavailable => "Unavailable",
        }
    }
}

/// A single SMART attribute relevant to health assessment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmartAttribute {
    /// Attribute ID (e.g., 5, 197, 198).
    pub id: u8,
    /// Human-readable name.
    pub name: String,
    /// Current normalized value (0–255 typically).
    pub value: u64,
    /// Worst recorded normalized value.
    pub worst: u64,
    /// Threshold for failure.
    pub threshold: u64,
    /// Raw value from the drive.
    pub raw: u64,
    /// Whether this is a pre-fail (critical) attribute.
    pub pre_fail: bool,
}

impl SmartAttribute {
    /// Whether this attribute has crossed its failure threshold.
    pub fn is_failing(&self) -> bool {
        self.pre_fail && self.threshold > 0 && self.value <= self.threshold
    }
}

/// Temperature reading in degrees Celsius, converted from Kelvin source.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TemperatureCelsius(pub f64);

impl TemperatureCelsius {
    /// Convert from Kelvin (UDisks2 reports temperature in decikelvin × 10 or millikelvin).
    /// UDisks2 SmartTemperature is in Kelvin.
    pub fn from_kelvin(kelvin: f64) -> Self {
        Self(kelvin - 273.15)
    }

    /// Returns the temperature value in Celsius.
    pub fn value(self) -> f64 {
        self.0
    }
}

/// Information about a single storage drive with SMART data.
#[derive(Debug, Clone)]
pub struct SmartDrive {
    /// Object path on D-Bus (internal identifier).
    pub object_path: String,
    /// Drive model name.
    pub model: String,
    /// Firmware revision.
    pub firmware: String,
    /// Serial number — internal only, never shown in reports unless
    /// the user explicitly disables redaction.
    pub serial: String,
    /// Whether this is a removable drive.
    pub removable: bool,
    /// SMART health assessment.
    pub assessment: SmartAssessment,
    /// Temperature in Celsius, if available.
    pub temperature_celsius: Option<TemperatureCelsius>,
    /// Power-on hours, if reported.
    pub power_on_hours: Option<u64>,
    /// Selected critical SMART attributes.
    pub attributes: Vec<SmartAttribute>,
    /// When the SMART data was last sampled.
    pub sample_time: Option<SystemTime>,
    /// Source of the SMART data (e.g., "UDisks2").
    pub source: &'static str,
}

/// Maximum age before SMART data is considered stale.
const STALE_THRESHOLD: Duration = Duration::from_secs(7 * 24 * 3600); // 7 days

/// Classify overall assessment from UDisks2 properties.
///
/// `overall_passed`: the UDisks2 SmartOverallAssessment (true = passed).
/// `attributes`: critical attributes from the drive.
/// `sample_time`: when the data was last collected.
pub fn classify_assessment(
    smart_supported: bool,
    smart_enabled: bool,
    overall_passed: Option<bool>,
    attributes: &[SmartAttribute],
    sample_time: Option<SystemTime>,
) -> SmartAssessment {
    if !smart_supported || !smart_enabled {
        return SmartAssessment::Unavailable;
    }

    // Check staleness.
    if let Some(sample) = sample_time {
        if let Ok(age) = SystemTime::now().duration_since(sample) {
            if age > STALE_THRESHOLD {
                return SmartAssessment::DataStale;
            }
        }
    }

    // Check if overall assessment reports failure.
    if let Some(passed) = overall_passed {
        if !passed {
            return SmartAssessment::FailureReportedByDrive;
        }
    }

    // Check critical attributes for threshold breach.
    if attributes.iter().any(|a| a.is_failing()) {
        return SmartAssessment::WarningReportedByDrive;
    }

    // If we have no overall assessment, we cannot claim healthy.
    match overall_passed {
        Some(true) => SmartAssessment::Passed,
        None => SmartAssessment::Unavailable,
        Some(false) => SmartAssessment::FailureReportedByDrive,
    }
}

/// Redact serial number for report output.
pub fn redact_serial(serial: &str) -> String {
    if serial.len() <= 4 {
        "****".to_string()
    } else {
        let visible = &serial[serial.len() - 4..];
        format!("****{visible}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn kelvin_to_celsius_conversion() {
        let temp = TemperatureCelsius::from_kelvin(313.15);
        assert!((temp.value() - 40.0).abs() < 0.01);
    }

    #[test]
    fn kelvin_zero_to_celsius() {
        let temp = TemperatureCelsius::from_kelvin(273.15);
        assert!(temp.value().abs() < 0.01);
    }

    #[test]
    fn assessment_passed_when_overall_true_and_no_failing_attrs() {
        let result = classify_assessment(true, true, Some(true), &[], Some(SystemTime::now()));
        assert_eq!(result, SmartAssessment::Passed);
    }

    #[test]
    fn assessment_failure_when_overall_false() {
        let result = classify_assessment(true, true, Some(false), &[], Some(SystemTime::now()));
        assert_eq!(result, SmartAssessment::FailureReportedByDrive);
    }

    #[test]
    fn assessment_warning_when_critical_attr_failing() {
        let attrs = vec![SmartAttribute {
            id: 5,
            name: "Reallocated Sectors".into(),
            value: 10,
            worst: 10,
            threshold: 36,
            raw: 100,
            pre_fail: true,
        }];
        let result = classify_assessment(true, true, Some(true), &attrs, Some(SystemTime::now()));
        assert_eq!(result, SmartAssessment::WarningReportedByDrive);
    }

    #[test]
    fn assessment_unavailable_when_smart_not_supported() {
        let result = classify_assessment(false, false, None, &[], None);
        assert_eq!(result, SmartAssessment::Unavailable);
    }

    #[test]
    fn assessment_unavailable_when_smart_not_enabled() {
        let result = classify_assessment(true, false, None, &[], None);
        assert_eq!(result, SmartAssessment::Unavailable);
    }

    #[test]
    fn assessment_data_stale_when_sample_too_old() {
        let old_time = SystemTime::now() - Duration::from_secs(8 * 24 * 3600);
        let result = classify_assessment(true, true, Some(true), &[], Some(old_time));
        assert_eq!(result, SmartAssessment::DataStale);
    }

    #[test]
    fn assessment_unavailable_when_no_overall_assessment() {
        let result = classify_assessment(true, true, None, &[], Some(SystemTime::now()));
        assert_eq!(result, SmartAssessment::Unavailable);
    }

    #[test]
    fn attribute_is_failing_when_value_at_or_below_threshold() {
        let attr = SmartAttribute {
            id: 197,
            name: "Current Pending Sector".into(),
            value: 5,
            worst: 5,
            threshold: 10,
            raw: 42,
            pre_fail: true,
        };
        assert!(attr.is_failing());
    }

    #[test]
    fn attribute_not_failing_when_above_threshold() {
        let attr = SmartAttribute {
            id: 197,
            name: "Current Pending Sector".into(),
            value: 100,
            worst: 100,
            threshold: 10,
            raw: 0,
            pre_fail: true,
        };
        assert!(!attr.is_failing());
    }

    #[test]
    fn non_prefail_attribute_never_failing() {
        let attr = SmartAttribute {
            id: 194,
            name: "Temperature".into(),
            value: 5,
            worst: 5,
            threshold: 10,
            raw: 42,
            pre_fail: false,
        };
        assert!(!attr.is_failing());
    }

    #[test]
    fn redact_serial_short() {
        assert_eq!(redact_serial("AB"), "****");
    }

    #[test]
    fn redact_serial_long() {
        assert_eq!(redact_serial("WD-WMC4N0123456"), "****3456");
    }

    #[test]
    fn assessment_labels_are_correct() {
        assert_eq!(SmartAssessment::Passed.label(), "Passed");
        assert_eq!(
            SmartAssessment::WarningReportedByDrive.label(),
            "Warning reported by drive"
        );
        assert_eq!(
            SmartAssessment::FailureReportedByDrive.label(),
            "Failure reported by drive"
        );
        assert_eq!(SmartAssessment::DataStale.label(), "Data stale");
        assert_eq!(SmartAssessment::Unavailable.label(), "Unavailable");
    }
}
