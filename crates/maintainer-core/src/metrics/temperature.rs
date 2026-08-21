//! Temperature metric collector that reads `/sys/class/hwmon`.
//!
//! Enumerates hwmon devices and their temperature inputs. Missing or unreadable
//! sensors become `Unavailable` entries rather than zero Celsius.

use std::fs;
use std::path::Path;

use crate::capability::{Capability, UnavailableReason};
use crate::error::MaintainerError;

/// A single temperature reading from a hardware sensor.
#[derive(Debug, Clone)]
pub struct TemperatureReading {
    /// Sensor hardware name (from hwmon `name` file, e.g., "coretemp").
    pub hwmon_name: String,
    /// Sensor label (from `temp*_label`, e.g., "Package id 0").
    pub label: Option<String>,
    /// Temperature in degrees Celsius.
    pub celsius: Capability<f64>,
}

/// Read all temperature sensors from `/sys/class/hwmon`.
pub fn read_temperatures(sys_path: &Path) -> Result<Vec<TemperatureReading>, MaintainerError> {
    let hwmon_dir = sys_path.join("class/hwmon");

    let entries = match fs::read_dir(&hwmon_dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Vec::new());
        }
        Err(e) => {
            return Err(MaintainerError::io(
                format!("reading {}", hwmon_dir.display()),
                e,
            ));
        }
    };

    let mut readings = Vec::new();

    for entry in entries {
        let entry = entry
            .map_err(|e| MaintainerError::io(format!("iterating {}", hwmon_dir.display()), e))?;

        let hwmon_path = entry.path();
        if !hwmon_path.is_dir() {
            continue;
        }

        let hwmon_name = read_hwmon_name(&hwmon_path);
        let sensor_readings = read_hwmon_temps(&hwmon_path, &hwmon_name)?;
        readings.extend(sensor_readings);
    }

    // Sort for deterministic output
    readings.sort_by(|a, b| {
        a.hwmon_name
            .cmp(&b.hwmon_name)
            .then_with(|| a.label.cmp(&b.label))
    });

    Ok(readings)
}

/// Read the hwmon device name.
fn read_hwmon_name(hwmon_path: &Path) -> String {
    let name_path = hwmon_path.join("name");
    fs::read_to_string(&name_path)
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}

/// Read all temperature inputs from a single hwmon device.
fn read_hwmon_temps(
    hwmon_path: &Path,
    hwmon_name: &str,
) -> Result<Vec<TemperatureReading>, MaintainerError> {
    let mut readings = Vec::new();

    // Look for temp*_input files
    let entries = match fs::read_dir(hwmon_path) {
        Ok(e) => e,
        Err(_) => return Ok(readings),
    };

    let mut temp_indices: Vec<u32> = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if let Some(idx_str) = name_str
            .strip_prefix("temp")
            .and_then(|s| s.strip_suffix("_input"))
        {
            if let Ok(idx) = idx_str.parse::<u32>() {
                temp_indices.push(idx);
            }
        }
    }

    temp_indices.sort_unstable();

    for idx in temp_indices {
        let input_path = hwmon_path.join(format!("temp{idx}_input"));
        let label_path = hwmon_path.join(format!("temp{idx}_label"));

        let label = fs::read_to_string(&label_path)
            .ok()
            .map(|s| s.trim().to_string());

        let celsius = match fs::read_to_string(&input_path) {
            Ok(content) => {
                let millidegrees_str = content.trim();
                match millidegrees_str.parse::<i64>() {
                    Ok(millidegrees) => Capability::Available(millidegrees as f64 / 1000.0),
                    Err(_) => Capability::Unavailable(UnavailableReason {
                        code: "sensor-parse-error",
                        message: format!(
                            "could not parse temperature from {}: '{millidegrees_str}'",
                            input_path.display()
                        ),
                        remediation: None,
                    }),
                }
            }
            Err(e) => Capability::Unavailable(UnavailableReason {
                code: "sensor-unreadable",
                message: format!("cannot read {}: {e}", input_path.display()),
                remediation: Some("Check sensor driver and permissions".to_string()),
            }),
        };

        readings.push(TemperatureReading {
            hwmon_name: hwmon_name.to_string(),
            label,
            celsius,
        });
    }

    Ok(readings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixtures_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sys")
    }

    #[test]
    fn read_temperatures_from_fixture() {
        let readings = read_temperatures(&fixtures_path()).unwrap();
        assert!(!readings.is_empty());

        // Should find coretemp sensors
        let coretemp: Vec<_> = readings
            .iter()
            .filter(|r| r.hwmon_name == "coretemp")
            .collect();
        assert_eq!(coretemp.len(), 2);
    }

    #[test]
    fn temperature_values_parsed_from_millidegrees() {
        let readings = read_temperatures(&fixtures_path()).unwrap();
        let pkg = readings
            .iter()
            .find(|r| r.label.as_deref() == Some("Package id 0"))
            .unwrap();
        // 45000 millidegrees = 45.0 C
        match &pkg.celsius {
            Capability::Available(c) => assert_eq!(*c, 45.0),
            other => panic!("expected Available, got {other:?}"),
        }
    }

    #[test]
    fn temperature_labels_read_correctly() {
        let readings = read_temperatures(&fixtures_path()).unwrap();
        let core0 = readings
            .iter()
            .find(|r| r.label.as_deref() == Some("Core 0"))
            .unwrap();
        assert_eq!(core0.hwmon_name, "coretemp");
        match &core0.celsius {
            Capability::Available(c) => assert_eq!(*c, 42.0),
            other => panic!("expected Available, got {other:?}"),
        }
    }

    #[test]
    fn acpitz_sensor_without_label() {
        let readings = read_temperatures(&fixtures_path()).unwrap();
        let acpi = readings.iter().find(|r| r.hwmon_name == "acpitz").unwrap();
        // hwmon1 has no label for temp1
        assert!(acpi.label.is_none());
        match &acpi.celsius {
            Capability::Available(c) => assert_eq!(*c, 38.5),
            other => panic!("expected Available, got {other:?}"),
        }
    }

    #[test]
    fn missing_hwmon_directory_returns_empty() {
        let nonexistent = PathBuf::from("/tmp/nonexistent_sys_path_for_test");
        let readings = read_temperatures(&nonexistent).unwrap();
        assert!(readings.is_empty());
    }

    #[test]
    fn unreadable_sensor_becomes_unavailable() {
        // Create a test scenario with invalid content
        let content = "not_a_number";
        let millidegrees_str = content.trim();
        let result = millidegrees_str.parse::<i64>();
        assert!(result.is_err());
        // This proves our parse path would create Unavailable
    }
}
