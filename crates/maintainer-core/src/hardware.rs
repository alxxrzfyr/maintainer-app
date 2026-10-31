//! Hardware inspection collectors for GPU, USB devices, PCI devices, and BIOS.

use crate::command::{CommandRunner, CommandSpec, SystemCommandRunner};
use crate::jobs::CancellationToken;
use std::fs;
use std::path::Path;

/// Information about the primary GPU and graphics acceleration features.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GpuInfo {
    pub brand: String,
    pub name: String,
    pub pci_id: String,
    pub driver: String,
    pub opengl: String,
    pub opengl_es: String,
    pub vulkan: String,
    pub video_playback: String,
}

/// A connected or integrated USB device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbDevice {
    pub name: String,
    pub id: String,
    pub speed: String,
    pub power: String,
    pub is_hub: bool,
    pub bus: String,
}

/// A PCI device detected on the system bus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PciDevice {
    pub slot: String,
    pub dev_type: String,
    pub brand: String,
    pub device: String,
    pub id: String,
    pub driver: String,
}

/// System BIOS and motherboard firmware information.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BiosInfo {
    pub vendor: String,
    pub version: String,
    pub release_date: String,
    pub motherboard: String,
}

/// Complete hardware specification snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HardwareSnapshot {
    pub gpu: GpuInfo,
    pub usb_devices: Vec<UsbDevice>,
    pub pci_devices: Vec<PciDevice>,
    pub bios: BiosInfo,
    /// Optional collection failures; readable fallback data remains available.
    pub collection_issues: Vec<String>,
}

/// Read all hardware inspection data.
pub fn read_hardware_snapshot() -> HardwareSnapshot {
    read_snapshot_with(&SystemCommandRunner, Path::new("/sys"))
}

fn read_snapshot_with(runner: &dyn CommandRunner, sys: &Path) -> HardwareSnapshot {
    let mut collection_issues = Vec::new();
    let pci_devices = read_pci_with(runner, sys, &mut collection_issues);
    HardwareSnapshot {
        gpu: gpu_from_pci(&pci_devices),
        usb_devices: read_usb_with(runner, sys, &mut collection_issues),
        pci_devices,
        bios: read_bios_with(sys),
        collection_issues,
    }
}

/// Query primary GPU identity. Acceleration requires an API probe, not device nodes.
pub fn read_gpu_info() -> GpuInfo {
    gpu_from_pci(&read_pci_devices())
}

fn gpu_from_pci(devices: &[PciDevice]) -> GpuInfo {
    let gpu = devices.iter().find(|device| {
        matches!(
            device.dev_type.as_str(),
            "VGA compatible controller" | "3D controller" | "Display controller"
        )
    });
    GpuInfo {
        brand: gpu.map_or_else(|| "Unavailable".into(), |gpu| gpu.brand.clone()),
        name: gpu.map_or_else(|| "No GPU identified".into(), |gpu| gpu.device.clone()),
        pci_id: gpu.map_or_else(|| "Unavailable".into(), |gpu| gpu.id.clone()),
        driver: gpu.map_or_else(|| "Unavailable".into(), |gpu| gpu.driver.clone()),
        // A DRM node or installed ICD does not prove working hardware acceleration.
        opengl: "Unverified (OpenGL probe not run)".into(),
        opengl_es: "Unverified (OpenGL ES probe not run)".into(),
        vulkan: "Unverified (Vulkan probe not run)".into(),
        video_playback: "Unverified (VA-API probe not run)".into(),
    }
}

/// Read list of USB devices connected to the system.
pub fn read_usb_devices() -> Vec<UsbDevice> {
    read_usb_with(&SystemCommandRunner, Path::new("/sys"), &mut Vec::new())
}

fn read_usb_with(
    runner: &dyn CommandRunner,
    sys: &Path,
    issues: &mut Vec<String>,
) -> Vec<UsbDevice> {
    let mut devices = Vec::new();
    match fs::read_dir(sys.join("bus/usb/devices")) {
        Ok(entries) => {
            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(error) => {
                        issues.push(format!("USB sysfs entry unavailable: {}", error.kind()));
                        continue;
                    }
                };
                let path = entry.path();
                let vendor = read_trimmed(&path.join("idVendor"));
                let product_id = read_trimmed(&path.join("idProduct"));
                if vendor.is_empty() || product_id.is_empty() {
                    continue; // Interface entries have no device descriptors.
                }
                let id = format!("{vendor}:{product_id}");
                let manufacturer = read_trimmed(&path.join("manufacturer"));
                let product = read_trimmed(&path.join("product"));
                let name = match (manufacturer.is_empty(), product.is_empty()) {
                    (false, false) if product.starts_with(&manufacturer) => product,
                    (false, false) => format!("{manufacturer} {product}"),
                    (true, false) => product,
                    (false, true) => manufacturer,
                    (true, true) => format!("USB Device [{id}]"),
                };
                let speed = read_trimmed(&path.join("speed"));
                let speed = match speed.parse::<f64>() {
                    Ok(value) if value.is_finite() && value > 0.0 => format!("{speed} Mbps"),
                    _ => "Unavailable".into(),
                };
                let power = read_trimmed(&path.join("bMaxPower"));
                let power = match power.strip_suffix("mA").map(str::trim) {
                    Some(value) if value.parse::<u32>().is_ok() => format!("{value} mA max"),
                    _ => "Unavailable".into(),
                };
                let bus = read_trimmed(&path.join("busnum"));
                let device = read_trimmed(&path.join("devnum"));
                let bus = match (bus.parse::<u32>(), device.parse::<u32>()) {
                    (Ok(bus), Ok(device)) => format!("Bus {bus:03} Device {device:03}"),
                    _ => "USB Bus (address unavailable)".into(),
                };
                devices.push(UsbDevice {
                    name,
                    id,
                    speed,
                    power,
                    is_hub: read_trimmed(&path.join("bDeviceClass")) == "09",
                    bus,
                });
            }
        }
        Err(error) => issues.push(format!("USB sysfs unavailable: {}", error.kind())),
    }

    // lsusb provides names and IDs, but does not report link speed or power.
    match run_local_command(runner, "lsusb", &[]) {
        Ok(stdout) => {
            let parsed = parse_lsusb(&stdout);
            if parsed.is_empty() && !stdout.trim().is_empty() {
                issues.push(
                    "USB names unavailable: lsusb returned unrecognized device records".into(),
                );
            }
            for device in parsed {
                if let Some(existing) = devices
                    .iter_mut()
                    .find(|existing| existing.bus == device.bus && existing.id == device.id)
                {
                    if existing.name.starts_with("USB Device [") {
                        existing.name = device.name;
                    }
                } else {
                    devices.push(device);
                }
            }
        }
        Err(reason) => issues.push(reason),
    }
    devices.sort_by(|left, right| left.bus.cmp(&right.bus).then(left.id.cmp(&right.id)));
    devices
}

fn parse_lsusb(stdout: &str) -> Vec<UsbDevice> {
    stdout
        .lines()
        .filter_map(|line| {
            let (address, description) = line.split_once(": ID ")?;
            let fields: Vec<_> = address.split_whitespace().collect();
            if fields.len() != 4 || fields[0] != "Bus" || fields[2] != "Device" {
                return None;
            }
            let bus = fields[1].parse::<u32>().ok()?;
            let device = fields[3].parse::<u32>().ok()?;
            let mut parts = description.splitn(2, char::is_whitespace);
            let id = parts.next()?;
            let (vendor, product) = id.split_once(':')?;
            if !valid_hex_id(vendor) || !valid_hex_id(product) {
                return None;
            }
            let name = parts.next().unwrap_or_default().trim();
            Some(UsbDevice {
                name: if name.is_empty() {
                    format!("USB Device [{id}]")
                } else {
                    name.into()
                },
                id: id.into(),
                speed: "Unavailable".into(),
                power: "Unavailable".into(),
                is_hub: name.to_lowercase().contains("root hub"),
                bus: format!("Bus {bus:03} Device {device:03}"),
            })
        })
        .collect()
}

/// Read list of PCI devices on the system.
pub fn read_pci_devices() -> Vec<PciDevice> {
    read_pci_with(&SystemCommandRunner, Path::new("/sys"), &mut Vec::new())
}

fn read_pci_with(
    runner: &dyn CommandRunner,
    sys: &Path,
    issues: &mut Vec<String>,
) -> Vec<PciDevice> {
    let mut devices = Vec::new();
    match fs::read_dir(sys.join("bus/pci/devices")) {
        Ok(entries) => {
            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(error) => {
                        issues.push(format!("PCI sysfs entry unavailable: {}", error.kind()));
                        continue;
                    }
                };
                let path = entry.path();
                let vendor = read_trimmed(&path.join("vendor"))
                    .trim_start_matches("0x")
                    .to_owned();
                let device = read_trimmed(&path.join("device"))
                    .trim_start_matches("0x")
                    .to_owned();
                if !valid_hex_id(&vendor) || !valid_hex_id(&device) {
                    continue;
                }
                let class = read_trimmed(&path.join("class"));
                let dev_type = if class.starts_with("0x0300") {
                    "VGA compatible controller"
                } else if class.starts_with("0x0302") {
                    "3D controller"
                } else if class.starts_with("0x0380") {
                    "Display controller"
                } else {
                    "PCI Controller"
                };
                let driver = match fs::read_link(path.join("driver")) {
                    Ok(link) => link
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned()),
                    Err(_) => None,
                }
                .unwrap_or_else(|| "Unavailable (no bound driver reported)".into());
                let brand = match vendor.as_str() {
                    "8086" => "Intel",
                    "10de" => "NVIDIA",
                    "1002" => "AMD",
                    "10ec" => "Realtek",
                    _ => "PCI",
                };
                devices.push(PciDevice {
                    slot: entry.file_name().to_string_lossy().into_owned(),
                    dev_type: dev_type.into(),
                    brand: brand.into(),
                    device: format!("PCI Device [{vendor}:{device}]"),
                    id: format!("{vendor}:{device}"),
                    driver,
                });
            }
        }
        Err(error) => issues.push(format!("PCI sysfs unavailable: {}", error.kind())),
    }
    match run_local_command(runner, "lspci", &["-D", "-vmm", "-nn", "-k"]) {
        Ok(stdout) => {
            let parsed = parse_lspci(&stdout);
            if parsed.is_empty() && !stdout.trim().is_empty() {
                issues.push(
                    "PCI names unavailable: lspci returned unrecognized device records".into(),
                );
            }
            for device in parsed {
                if let Some(existing) = devices
                    .iter_mut()
                    .find(|existing| existing.slot == device.slot)
                {
                    let driver = if device.driver.starts_with("Unavailable") {
                        existing.driver.clone()
                    } else {
                        device.driver.clone()
                    };
                    *existing = PciDevice { driver, ..device };
                } else {
                    devices.push(device);
                }
            }
        }
        Err(reason) => issues.push(reason),
    }
    devices.sort_by(|left, right| left.slot.cmp(&right.slot));
    devices
}

fn parse_lspci(stdout: &str) -> Vec<PciDevice> {
    let mut devices = Vec::new();
    for record in stdout.split("\n\n") {
        let mut device = PciDevice {
            slot: String::new(),
            dev_type: String::new(),
            brand: String::new(),
            device: String::new(),
            id: String::new(),
            driver: "Unavailable (no bound driver reported)".into(),
        };
        let mut vendor_id = None;
        let mut device_id = None;
        for line in record.lines() {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let value = value.trim();
            let (name, id) = split_pci_name_id(value);
            match key {
                "Slot" => device.slot = value.into(),
                "Class" => device.dev_type = name.into(),
                "Vendor" => {
                    device.brand = name.replace(" Corporation", "").replace(" Inc.", "");
                    vendor_id = id;
                }
                "Device" => {
                    device.device = name.into();
                    device_id = id;
                }
                "Driver" => device.driver = value.into(),
                _ => {}
            }
        }
        if let (Some(vendor_id), Some(device_id)) = (vendor_id, device_id) {
            if !device.slot.is_empty() && !device.dev_type.is_empty() {
                device.id = format!("{vendor_id}:{device_id}");
                devices.push(device);
            }
        }
    }
    devices
}

fn split_pci_name_id(value: &str) -> (&str, Option<&str>) {
    if let Some((name, id)) = value.rsplit_once(" [") {
        if let Some(id) = id.strip_suffix(']') {
            if valid_hex_id(id) {
                return (name, Some(id));
            }
        }
    }
    (value, None)
}

fn valid_hex_id(id: &str) -> bool {
    id.len() == 4 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn read_trimmed(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_default()
        .trim()
        .to_owned()
}

fn run_local_command(
    runner: &dyn CommandRunner,
    program: &str,
    args: &[&str],
) -> Result<String, String> {
    let spec = CommandSpec::new(program, args.iter().copied())
        .map_err(|error| format!("{program} unavailable: {error}"))?
        .with_timeout(std::time::Duration::from_secs(3))
        .with_output_cap(256 * 1024);
    let output = runner
        .run(&spec, &CancellationToken::new())
        .map_err(|error| format!("{program} unavailable: {error}"))?;
    if !output.success() {
        return Err(format!(
            "{program} unavailable: command exited with status {:?}",
            output.exit_code
        ));
    }
    if output.stdout_truncated {
        return Err(format!(
            "{program} unavailable: output exceeded capture limit"
        ));
    }
    Ok(output.stdout)
}

/// Read BIOS and motherboard information from DMI.
pub fn read_bios_info() -> BiosInfo {
    read_bios_with(Path::new("/sys"))
}

fn read_bios_with(sys: &Path) -> BiosInfo {
    BiosInfo {
        vendor: read_dmi_value(sys, &["bios_vendor"]),
        version: read_dmi_value(sys, &["bios_version"]),
        release_date: read_dmi_value(sys, &["bios_date"]),
        motherboard: read_dmi_value(sys, &["board_name", "product_name"]),
    }
}

fn read_dmi_value(sys: &Path, fields: &[&str]) -> String {
    let mut permission_required = false;
    for field in fields {
        for directory in ["class/dmi/id", "devices/virtual/dmi/id"] {
            match fs::read_to_string(sys.join(directory).join(field)) {
                Ok(value) if !value.trim().is_empty() => return value.trim().to_owned(),
                Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                    permission_required = true;
                }
                _ => {}
            }
        }
    }
    if permission_required {
        "Unavailable (DMI permission required)".into()
    } else {
        "Unavailable (DMI field missing or unreadable)".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::CommandOutput;
    use crate::jobs::Cancellation;
    use std::sync::Mutex;

    const PCI: &str = include_str!("../tests/fixtures/hardware/lspci.txt");
    const USB: &str = include_str!("../tests/fixtures/hardware/lsusb.txt");

    #[derive(Default)]
    struct FixtureRunner {
        calls: Mutex<Vec<(String, Vec<String>)>>,
    }

    impl CommandRunner for FixtureRunner {
        fn run(
            &self,
            spec: &CommandSpec,
            _: &dyn Cancellation,
        ) -> crate::error::Result<CommandOutput> {
            self.calls
                .lock()
                .unwrap()
                .push((spec.program().into(), spec.args().to_vec()));
            Ok(CommandOutput {
                exit_code: Some(0),
                stdout: match spec.program() {
                    "lspci" => PCI,
                    "lsusb" => USB,
                    _ => panic!("unexpected command"),
                }
                .into(),
                stderr: String::new(),
                stdout_truncated: false,
                stderr_truncated: false,
            })
        }
    }

    fn fixture_sys() -> tempfile::TempDir {
        fn copy_tree(source: &Path, target: &Path) {
            fs::create_dir_all(target).unwrap();
            for entry in fs::read_dir(source).unwrap() {
                let entry = entry.unwrap();
                let destination = target.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    copy_tree(&entry.path(), &destination);
                } else {
                    fs::copy(entry.path(), destination).unwrap();
                }
            }
        }
        let fixture = tempfile::tempdir().unwrap();
        copy_tree(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hardware/sys"),
            fixture.path(),
        );
        // Real PCI slots contain colons. Create that Linux layout at runtime
        // so the tracked fixture tree remains checkout-compatible on Windows.
        let devices = fixture.path().join("bus/pci/devices");
        fs::rename(devices.join("0000_00_02_0"), devices.join("0000:00:02.0")).unwrap();
        fixture
    }

    #[test]
    fn hardware_commands_are_allowed() {
        for (program, args) in [
            ("lspci", vec!["-D", "-vmm", "-nn", "-k"]),
            ("lsusb", vec![]),
        ] {
            assert!(
                CommandSpec::new(program, args).is_ok(),
                "{program} must reach its runner"
            );
        }
    }

    #[test]
    fn hardware_commands_reach_runner_and_preserve_named_devices() {
        let runner = FixtureRunner::default();
        let snapshot = read_snapshot_with(&runner, fixture_sys().path());
        let calls = runner.calls.lock().unwrap();
        assert_eq!(
            calls.as_slice(),
            &[
                (
                    "lspci".into(),
                    vec!["-D".into(), "-vmm".into(), "-nn".into(), "-k".into()]
                ),
                ("lsusb".into(), vec![]),
            ]
        );
        assert!(snapshot.collection_issues.is_empty());
        assert_eq!(snapshot.gpu.name, "HD Graphics 530");
        assert_eq!(snapshot.gpu.pci_id, "8086:191b");
        assert_eq!(snapshot.gpu.driver, "i915");
        assert_eq!(snapshot.pci_devices.len(), 2);
    }

    #[test]
    fn pci_fixture_parses_tagged_records_and_does_not_reuse_driver() {
        let devices = parse_lspci(PCI);
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].brand, "Intel");
        assert_eq!(devices[0].id, "8086:191b");
        assert_eq!(devices[1].id, "8086:a12f");
        assert_eq!(devices[1].driver, "Unavailable (no bound driver reported)");
        assert!(parse_lspci("Slot:\t0000:00:01.0\nClass:\tVGA controller\n").is_empty());
    }

    #[test]
    fn acceleration_is_unverified_even_with_a_bound_gpu_driver() {
        let gpu = gpu_from_pci(&parse_lspci(PCI));
        assert_eq!(gpu.driver, "i915");
        for value in [
            &gpu.opengl,
            &gpu.opengl_es,
            &gpu.vulkan,
            &gpu.video_playback,
        ] {
            assert!(value.starts_with("Unverified"), "{value}");
        }
        assert_eq!(gpu_from_pci(&[]).name, "No GPU identified");
    }

    #[test]
    fn usb_fixture_uses_measured_speed_descriptor_power_and_unknowns() {
        let devices = read_usb_with(
            &FixtureRunner::default(),
            fixture_sys().path(),
            &mut Vec::new(),
        );
        assert_eq!(devices.len(), 3);
        assert_eq!(devices[0].name, "Example Camera");
        assert_eq!(devices[0].speed, "5000 Mbps");
        assert_eq!(devices[0].power, "224 mA max");
        assert!(!devices[0].is_hub);
        assert_eq!(devices[1].speed, "Unavailable");
        assert_eq!(devices[1].power, "Unavailable");
        // An lsusb-only root hub has no observed speed or power, despite its name.
        assert!(devices[2].is_hub);
        assert_eq!(devices[2].speed, "Unavailable");
        assert_eq!(devices[2].power, "Unavailable");
    }

    #[test]
    fn lsusb_fixture_rejects_bad_records_without_guessing_speed() {
        let devices = parse_lsusb(USB);
        assert_eq!(devices.len(), 3);
        assert_eq!(devices[0].id, "1234:abcd");
        assert_eq!(devices[0].bus, "Bus 001 Device 002");
        assert!(parse_lsusb("Bus 001 Device 002: ID invalid Camera").is_empty());
        assert!(
            devices
                .iter()
                .all(|device| device.speed == "Unavailable" && device.power == "Unavailable")
        );
    }

    struct FailedRunner;

    impl CommandRunner for FailedRunner {
        fn run(
            &self,
            _: &CommandSpec,
            _: &dyn Cancellation,
        ) -> crate::error::Result<CommandOutput> {
            Err(crate::error::MaintainerError::io(
                "spawning command",
                std::io::Error::from(std::io::ErrorKind::NotFound),
            ))
        }
    }

    #[test]
    fn unavailable_commands_keep_reasons_and_sysfs_evidence() {
        let snapshot = read_snapshot_with(&FailedRunner, fixture_sys().path());
        assert_eq!(snapshot.collection_issues.len(), 2);
        assert!(snapshot.collection_issues[0].contains("lspci unavailable"));
        assert!(snapshot.collection_issues[1].contains("lsusb unavailable"));
        assert_eq!(snapshot.gpu.pci_id, "8086:191b");
        assert_eq!(snapshot.usb_devices[0].speed, "5000 Mbps");
        assert_eq!(snapshot.usb_devices[0].power, "224 mA max");
    }

    struct OutputRunner(CommandOutput);

    impl CommandRunner for OutputRunner {
        fn run(
            &self,
            _: &CommandSpec,
            _: &dyn Cancellation,
        ) -> crate::error::Result<CommandOutput> {
            Ok(self.0.clone())
        }
    }

    #[test]
    fn rejected_failed_and_truncated_commands_report_reasons() {
        let runner = FixtureRunner::default();
        assert!(
            run_local_command(&runner, "unapproved-hardware-tool", &[])
                .unwrap_err()
                .contains("allowlist")
        );
        assert!(runner.calls.lock().unwrap().is_empty());
        for (exit_code, stdout_truncated, expected) in [
            (Some(7), false, "command exited with status"),
            (Some(0), true, "output exceeded capture limit"),
        ] {
            let runner = OutputRunner(CommandOutput {
                exit_code,
                stdout_truncated,
                stdout: "private device output".into(),
                stderr: "private diagnostic output".into(),
                stderr_truncated: false,
            });
            let reason = run_local_command(&runner, "lspci", &[]).unwrap_err();
            assert!(reason.contains(expected));
            assert!(!reason.contains("private"));
        }
    }

    #[test]
    fn malformed_command_output_reports_unavailable_instead_of_empty_success() {
        let runner = OutputRunner(CommandOutput {
            exit_code: Some(0),
            stdout: "not a device record".into(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
        });
        let snapshot = read_snapshot_with(&runner, fixture_sys().path());
        assert!(
            snapshot
                .collection_issues
                .iter()
                .any(|reason| reason.starts_with("USB names unavailable:"))
        );
        assert!(
            snapshot
                .collection_issues
                .iter()
                .any(|reason| reason.starts_with("PCI names unavailable:"))
        );
        assert_eq!(snapshot.gpu.pci_id, "8086:191b");
    }

    #[test]
    fn bios_fixture_preserves_reported_values() {
        let bios = read_bios_with(fixture_sys().path());
        assert_eq!(bios.vendor, "Example Firmware");
        assert_eq!(bios.version, "2.7.4");
        assert_eq!(bios.release_date, "10/15/2026");
        assert_eq!(bios.motherboard, "Example Board");
    }

    #[test]
    fn missing_bios_fields_are_unavailable_instead_of_invented() {
        let directory = tempfile::tempdir().unwrap();
        let bios = read_bios_with(directory.path());
        for value in [
            &bios.vendor,
            &bios.version,
            &bios.release_date,
            &bios.motherboard,
        ] {
            assert_eq!(value, "Unavailable (DMI field missing or unreadable)");
        }
    }

    #[test]
    fn missing_sysfs_keeps_command_evidence_and_reports_reason() {
        let directory = tempfile::tempdir().unwrap();
        let snapshot = read_snapshot_with(&FixtureRunner::default(), directory.path());
        assert_eq!(snapshot.pci_devices.len(), 2);
        assert_eq!(snapshot.usb_devices.len(), 3);
        assert!(
            snapshot
                .collection_issues
                .iter()
                .any(|issue| issue.starts_with("USB sysfs unavailable:"))
        );
        assert!(
            snapshot
                .collection_issues
                .iter()
                .any(|issue| issue.starts_with("PCI sysfs unavailable:"))
        );
    }
}
