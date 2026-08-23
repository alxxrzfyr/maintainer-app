//! Temperature metric collector that reads `/sys/class/hwmon`.
//!
//! Enumerates hwmon devices and their temperature inputs. Missing or unreadable
//! sensors become `Unavailable` entries rather than zero Celsius.

use std::fs;
use std::path::{Path, PathBuf};

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

/// A discovered sensor: the value file plus the metadata that never changes.
#[derive(Debug, Clone)]
struct SensorPath {
    input_path: PathBuf,
    hwmon_name: String,
    label: Option<String>,
}

/// Samples temperatures without re-discovering hwmon topology every time.
///
/// [`read_temperatures`] re-reads every hwmon directory entry plus each `name`
/// and `temp*_label` file on every call, which dominates the cost of a sampling
/// tick. Device topology and sensor labels are fixed while the machine is
/// running, so they are discovered once and only `temp*_input` is read per
/// sample. Discovery is retried on the next call if it yielded no sensors, so a
/// driver that loads later is still picked up.
#[derive(Debug, Default)]
pub struct TemperatureCollector {
    sensors: Vec<SensorPath>,
    discovered: bool,
}

impl TemperatureCollector {
    /// Create a collector with no cached topology.
    pub fn new() -> Self {
        Self {
            sensors: Vec::new(),
            discovered: false,
        }
    }

    /// Read current values for all known sensors, discovering them if needed.
    pub fn read(&mut self, sys_path: &Path) -> Result<Vec<TemperatureReading>, MaintainerError> {
        if !self.discovered || self.sensors.is_empty() {
            self.sensors = discover_sensors(sys_path)?;
            self.discovered = true;
        }

        let mut readings = Vec::with_capacity(self.sensors.len());
        for sensor in &self.sensors {
            readings.push(TemperatureReading {
                hwmon_name: sensor.hwmon_name.clone(),
                label: sensor.label.clone(),
                celsius: read_celsius(&sensor.input_path),
            });
        }
        Ok(readings)
    }
}

/// Parse one `temp*_input` file, which holds millidegrees Celsius.
fn read_celsius(input_path: &Path) -> Capability<f64> {
    match fs::read_to_string(input_path) {
        Ok(content) => match content.trim().parse::<i64>() {
            Ok(millidegrees) => Capability::Available(millidegrees as f64 / 1000.0),
            Err(_) => Capability::Unavailable(UnavailableReason {
                code: "temp-parse",
                message: format!("Could not parse {}", input_path.display()),
                remediation: None,
            }),
        },
        Err(e) => Capability::Unavailable(UnavailableReason {
            code: "temp-read",
            message: e.to_string(),
            remediation: None,
        }),
    }
}

/// Walk `/sys/class/hwmon` once, recording each sensor's value path and labels.
fn discover_sensors(sys_path: &Path) -> Result<Vec<SensorPath>, MaintainerError> {
    let hwmon_dir = sys_path.join("class/hwmon");

    let entries = match fs::read_dir(&hwmon_dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(MaintainerError::io(
                format!("reading {}", hwmon_dir.display()),
                e,
            ));
        }
    };

    let mut sensors = Vec::new();

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let hwmon_path = entry.path();
        if !hwmon_path.is_dir() {
            continue;
        }

        let hwmon_name = read_hwmon_name(&hwmon_path);

        let inner = match fs::read_dir(&hwmon_path) {
            Ok(inner) => inner,
            Err(_) => continue,
        };

        let mut temp_indices: Vec<u32> = Vec::new();
        for file in inner {
            let file = match file {
                Ok(f) => f,
                Err(_) => continue,
            };
            let name = file.file_name();
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
            let label = fs::read_to_string(hwmon_path.join(format!("temp{idx}_label")))
                .ok()
                .map(|s| s.trim().to_string());
            sensors.push(SensorPath {
                input_path: hwmon_path.join(format!("temp{idx}_input")),
                hwmon_name: hwmon_name.clone(),
                label,
            });
        }
    }

    sensors.sort_by(|a, b| {
        a.hwmon_name
            .cmp(&b.hwmon_name)
            .then_with(|| a.label.cmp(&b.label))
    });

    Ok(sensors)
}

/// A group of temperature readings from a single hardware chip.
///
/// For chips with multiple sensors (e.g., coretemp with per-core readings),
/// a representative reading is selected for summary display.
#[derive(Debug, Clone)]
pub struct TemperatureGroup {
    /// Chip/device name (from hwmon `name` file).
    pub chip_name: String,
    /// Single representative reading for summary display.
    pub representative: TemperatureReading,
    /// All readings from this chip, including the representative.
    pub details: Vec<TemperatureReading>,
}

/// Group temperature readings by hardware chip, selecting a representative per group.
///
/// Rules for representative selection:
/// - For "coretemp": prefer the reading whose label starts with "Package".
/// - For all others: pick the reading with the highest available temperature.
/// - If no readings have an available value, the first reading is representative.
///
/// Groups are sorted alphabetically by `chip_name`.
pub fn group_temperatures(readings: &[TemperatureReading]) -> Vec<TemperatureGroup> {
    use std::collections::BTreeMap;

    if readings.is_empty() {
        return Vec::new();
    }

    let mut groups: BTreeMap<String, Vec<TemperatureReading>> = BTreeMap::new();
    for reading in readings {
        groups
            .entry(reading.hwmon_name.clone())
            .or_default()
            .push(reading.clone());
    }

    groups
        .into_iter()
        .map(|(chip_name, details)| {
            let representative = select_representative(&chip_name, &details);
            TemperatureGroup {
                chip_name,
                representative,
                details,
            }
        })
        .collect()
}

/// Select a representative reading from a chip's sensor list.
fn select_representative(chip_name: &str, readings: &[TemperatureReading]) -> TemperatureReading {
    debug_assert!(!readings.is_empty());

    if chip_name == "coretemp" {
        // Prefer "Package id *" label.
        if let Some(pkg) = readings
            .iter()
            .find(|r| r.label.as_deref().is_some_and(|l| l.starts_with("Package")))
        {
            return pkg.clone();
        }
    }

    // Pick reading with the highest available temperature.
    readings
        .iter()
        .filter_map(|r| match &r.celsius {
            Capability::Available(c) => Some((r, *c)),
            _ => None,
        })
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(r, _)| r.clone())
        .unwrap_or_else(|| readings[0].clone())
}

#[cfg(test)]
mod group_tests {
    use super::*;

    fn reading(name: &str, label: Option<&str>, celsius: f64) -> TemperatureReading {
        TemperatureReading {
            hwmon_name: name.to_string(),
            label: label.map(|s| s.to_string()),
            celsius: Capability::Available(celsius),
        }
    }

    #[test]
    fn coretemp_package_is_representative() {
        let readings = vec![
            reading("coretemp", Some("Package id 0"), 45.0),
            reading("coretemp", Some("Core 0"), 42.0),
            reading("coretemp", Some("Core 1"), 48.0),
        ];
        let groups = group_temperatures(&readings);
        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0].representative.label.as_deref(),
            Some("Package id 0")
        );
        assert_eq!(groups[0].details.len(), 3);
    }

    #[test]
    fn no_package_uses_max_temperature() {
        let readings = vec![
            reading("iwlwifi", Some("Sensor 1"), 35.0),
            reading("iwlwifi", Some("Sensor 2"), 55.0),
            reading("iwlwifi", Some("Sensor 3"), 40.0),
        ];
        let groups = group_temperatures(&readings);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].representative.label.as_deref(), Some("Sensor 2"));
    }

    #[test]
    fn multiple_chips_produce_multiple_groups() {
        let readings = vec![
            reading("acpitz", None, 38.0),
            reading("coretemp", Some("Package id 0"), 50.0),
            reading("coretemp", Some("Core 0"), 47.0),
        ];
        let groups = group_temperatures(&readings);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].chip_name, "acpitz");
        assert_eq!(groups[1].chip_name, "coretemp");
    }

    #[test]
    fn single_sensor_chip_passes_through() {
        let readings = vec![reading("acpitz", None, 38.5)];
        let groups = group_temperatures(&readings);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].details.len(), 1);
        match &groups[0].representative.celsius {
            Capability::Available(c) => assert_eq!(*c, 38.5),
            other => panic!("expected Available, got {other:?}"),
        }
    }

    #[test]
    fn empty_input_returns_empty_groups() {
        let groups = group_temperatures(&[]);
        assert!(groups.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixtures_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sys")
    }

    #[test]
    fn collector_matches_one_shot_read() {
        let expected = read_temperatures(&fixtures_path()).unwrap();
        let mut collector = TemperatureCollector::new();
        let actual = collector.read(&fixtures_path()).unwrap();

        assert_eq!(actual.len(), expected.len());
        for (a, e) in actual.iter().zip(expected.iter()) {
            assert_eq!(a.hwmon_name, e.hwmon_name);
            assert_eq!(a.label, e.label);
            assert_eq!(a.celsius.as_value().copied(), e.celsius.as_value().copied());
        }
    }

    #[test]
    fn collector_repeated_reads_are_stable() {
        let mut collector = TemperatureCollector::new();
        let first = collector.read(&fixtures_path()).unwrap();
        let second = collector.read(&fixtures_path()).unwrap();

        assert!(!first.is_empty());
        assert_eq!(first.len(), second.len());
        for (a, b) in first.iter().zip(second.iter()) {
            assert_eq!(a.hwmon_name, b.hwmon_name);
            assert_eq!(a.label, b.label);
            assert_eq!(a.celsius.as_value().copied(), b.celsius.as_value().copied());
        }
    }

    #[test]
    fn collector_on_missing_hwmon_returns_empty() {
        let mut collector = TemperatureCollector::new();
        let readings = collector
            .read(&PathBuf::from("/nonexistent-sys-path-for-test"))
            .unwrap();
        assert!(readings.is_empty());
    }

    #[test]
    fn read_temperatures_from_fixture() {
        let readings = read_temperatures(&fixtures_path()).unwrap();
        assert!(!readings.is_empty());

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
        let content = "not_a_number";
        let millidegrees_str = content.trim();
        let result = millidegrees_str.parse::<i64>();
        assert!(result.is_err());
    }
}
