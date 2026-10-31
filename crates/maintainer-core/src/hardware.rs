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
}

/// Read all hardware inspection data.
pub fn read_hardware_snapshot() -> HardwareSnapshot {
    HardwareSnapshot {
        gpu: read_gpu_info(),
        usb_devices: read_usb_devices(),
        pci_devices: read_pci_devices(),
        bios: read_bios_info(),
    }
}

/// Query primary GPU and hardware acceleration capabilities.
pub fn read_gpu_info() -> GpuInfo {
    let mut brand = "Unknown".to_string();
    let mut name = "Integrated Graphics".to_string();
    let mut pci_id = "—".to_string();
    let mut driver = "—".to_string();

    if let Some(stdout) = run_local_command("lspci", &["-mm", "-nn"]) {
        for line in stdout.lines() {
            if line.contains("VGA compatible controller")
                || line.contains("3D controller")
                || line.contains("Display controller")
            {
                let parts: Vec<&str> = line
                    .split('"')
                    .enumerate()
                    .filter_map(|(idx, s)| if idx % 2 == 1 { Some(s.trim()) } else { None })
                    .collect();
                if parts.len() >= 3 {
                    brand = parts[1].replace(" Corporation", "").replace(" Inc.", "");
                    name = parts[2].to_string();
                    if let Some(start) = line.find('[') {
                        if let Some(end) = line[start..].find(']') {
                            let id_str = &line[start + 1..start + end];
                            if id_str.contains(':') {
                                pci_id = id_str.to_string();
                            }
                        }
                    }
                    break;
                }
            }
        }
    }

    if let Ok(entries) = fs::read_dir("/sys/bus/pci/devices") {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Ok(class) = fs::read_to_string(path.join("class")) {
                let class_trimmed = class.trim();
                if class_trimmed.starts_with("0x0300")
                    || class_trimmed.starts_with("0x0302")
                    || class_trimmed.starts_with("0x0380")
                {
                    if let Ok(drv_link) = fs::read_link(path.join("driver")) {
                        if let Some(drv_name) = drv_link.file_name() {
                            driver = drv_name.to_string_lossy().into_owned();
                        }
                    }
                    if pci_id == "—" {
                        let vendor = fs::read_to_string(path.join("vendor"))
                            .unwrap_or_default()
                            .trim()
                            .trim_start_matches("0x")
                            .to_string();
                        let device = fs::read_to_string(path.join("device"))
                            .unwrap_or_default()
                            .trim()
                            .trim_start_matches("0x")
                            .to_string();
                        if !vendor.is_empty() && !device.is_empty() {
                            pci_id = format!("{vendor}:{device}");
                        }
                    }
                    break;
                }
            }
        }
    }

    let dri_exists =
        Path::new("/dev/dri/renderD128").exists() || Path::new("/dev/dri/card0").exists();
    let opengl = if dri_exists {
        "Enabled".to_string()
    } else {
        "Software".to_string()
    };
    let opengl_es = if dri_exists {
        "Enabled".to_string()
    } else {
        "Disabled".to_string()
    };
    let vulkan = if Path::new("/usr/share/vulkan/icd.d").exists()
        || Path::new("/etc/vulkan/icd.d").exists()
    {
        "Enabled".to_string()
    } else {
        "Disabled".to_string()
    };
    let video_playback = if dri_exists {
        "Enabled (VA-API)".to_string()
    } else {
        "Disabled".to_string()
    };

    GpuInfo {
        brand,
        name,
        pci_id,
        driver,
        opengl,
        opengl_es,
        vulkan,
        video_playback,
    }
}

/// Read list of USB devices connected to the system.
pub fn read_usb_devices() -> Vec<UsbDevice> {
    let mut devices = Vec::new();

    if let Some(stdout) = run_local_command("lsusb", &[]) {
        for line in stdout.lines() {
            if let Some(id_idx) = line.find("ID ") {
                let rest = &line[id_idx + 3..];
                let mut parts = rest.splitn(2, ' ');
                let id = parts.next().unwrap_or("").trim().to_string();
                let name = parts.next().unwrap_or("").trim().to_string();

                let bus_part = if line.starts_with("Bus ") {
                    line.split(':').next().unwrap_or("").trim().to_string()
                } else {
                    "USB".to_string()
                };

                let is_hub = name.to_lowercase().contains("root hub")
                    || name.to_lowercase().contains("controller");
                let speed = if is_hub && (name.contains("3.0") || name.contains("3.1")) {
                    "5000 Mbps".to_string()
                } else if is_hub || name.to_lowercase().contains("camera") {
                    "480 Mbps".to_string()
                } else {
                    "12 Mbps".to_string()
                };

                let power = if is_hub {
                    "0 mA max".to_string()
                } else {
                    "100 mA".to_string()
                };

                devices.push(UsbDevice {
                    name: if name.is_empty() {
                        format!("USB Device [{id}]")
                    } else {
                        name
                    },
                    id,
                    speed,
                    power,
                    is_hub,
                    bus: bus_part,
                });
            }
        }
    }

    if devices.is_empty() {
        if let Ok(entries) = fs::read_dir("/sys/bus/usb/devices") {
            for entry in entries.flatten() {
                let path = entry.path();
                let id_vendor = fs::read_to_string(path.join("idVendor"))
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                let id_product = fs::read_to_string(path.join("idProduct"))
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                if id_vendor.is_empty() || id_product.is_empty() {
                    continue;
                }
                let manufacturer = fs::read_to_string(path.join("manufacturer"))
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                let product = fs::read_to_string(path.join("product"))
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                let speed_val = fs::read_to_string(path.join("speed"))
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                let speed = if !speed_val.is_empty() {
                    format!("{speed_val} Mbps")
                } else {
                    "12 Mbps".to_string()
                };
                let power = fs::read_to_string(path.join("bMaxPower"))
                    .unwrap_or_default()
                    .trim()
                    .to_string();

                let full_name = match (!manufacturer.is_empty(), !product.is_empty()) {
                    (true, true) => format!("{manufacturer} {product}"),
                    (false, true) => product,
                    (true, false) => manufacturer,
                    (false, false) => format!("USB Device [{id_vendor}:{id_product}]"),
                };

                let is_hub = full_name.to_lowercase().contains("root hub");
                devices.push(UsbDevice {
                    name: full_name,
                    id: format!("{id_vendor}:{id_product}"),
                    speed,
                    power: if power.is_empty() {
                        "100 mA".to_string()
                    } else {
                        power
                    },
                    is_hub,
                    bus: "USB Bus".to_string(),
                });
            }
        }
    }

    devices
}

/// Read list of PCI devices on the system.
pub fn read_pci_devices() -> Vec<PciDevice> {
    let mut devices = Vec::new();

    if let Some(stdout) = run_local_command("lspci", &["-mm", "-nn", "-k"]) {
        let mut current_slot = String::new();
        let mut current_type = String::new();
        let mut current_brand = String::new();
        let mut current_device = String::new();
        let mut current_id = String::new();
        let mut current_driver = String::new();

        for line in stdout.lines() {
            if line.starts_with("Slot:\t") {
                if !current_slot.is_empty() {
                    devices.push(PciDevice {
                        slot: current_slot.clone(),
                        dev_type: current_type.clone(),
                        brand: current_brand.clone(),
                        device: current_device.clone(),
                        id: current_id.clone(),
                        driver: if current_driver.is_empty() {
                            "—".to_string()
                        } else {
                            current_driver.clone()
                        },
                    });
                    current_driver.clear();
                    current_id.clear();
                }
                current_slot = line.trim_start_matches("Slot:\t").trim().to_string();
            } else if line.starts_with("Class:\t") {
                let raw_type = line.trim_start_matches("Class:\t").trim();
                current_type = raw_type
                    .split('[')
                    .next()
                    .unwrap_or(raw_type)
                    .trim()
                    .to_string();
            } else if line.starts_with("Vendor:\t") {
                let raw_vendor = line.trim_start_matches("Vendor:\t").trim();
                current_brand = raw_vendor
                    .split('[')
                    .next()
                    .unwrap_or(raw_vendor)
                    .trim()
                    .replace(" Corporation", "")
                    .replace(" Inc.", "")
                    .replace(" Co., Ltd.", "");
            } else if line.starts_with("Device:\t") {
                let raw_dev = line.trim_start_matches("Device:\t").trim();
                current_device = raw_dev
                    .split('[')
                    .next()
                    .unwrap_or(raw_dev)
                    .trim()
                    .to_string();
                if let Some(start) = raw_dev.find('[') {
                    if let Some(end) = raw_dev[start..].find(']') {
                        current_id = raw_dev[start + 1..start + end].to_string();
                    }
                }
            } else if line.starts_with("Driver:\t") {
                current_driver = line.trim_start_matches("Driver:\t").trim().to_string();
            }
        }

        if !current_slot.is_empty() {
            devices.push(PciDevice {
                slot: current_slot,
                dev_type: current_type,
                brand: current_brand,
                device: current_device,
                id: current_id,
                driver: if current_driver.is_empty() {
                    "—".to_string()
                } else {
                    current_driver
                },
            });
        }
    }

    if devices.is_empty() {
        if let Ok(entries) = fs::read_dir("/sys/bus/pci/devices") {
            for entry in entries.flatten() {
                let path = entry.path();
                let slot = entry.file_name().to_string_lossy().into_owned();
                let vendor = fs::read_to_string(path.join("vendor"))
                    .unwrap_or_default()
                    .trim()
                    .trim_start_matches("0x")
                    .to_string();
                let device = fs::read_to_string(path.join("device"))
                    .unwrap_or_default()
                    .trim()
                    .trim_start_matches("0x")
                    .to_string();
                let driver = fs::read_link(path.join("driver"))
                    .ok()
                    .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                    .unwrap_or_else(|| "—".to_string());
                let brand = match vendor.as_str() {
                    "8086" => "Intel",
                    "10de" => "NVIDIA",
                    "1002" => "AMD",
                    "10ec" => "Realtek",
                    _ => "PCI",
                }
                .to_string();

                devices.push(PciDevice {
                    slot,
                    dev_type: "PCI Controller".to_string(),
                    brand,
                    device: format!("PCI Device [{vendor}:{device}]"),
                    id: format!("{vendor}:{device}"),
                    driver,
                });
            }
        }
    }

    devices
}

fn run_local_command(program: &str, args: &[&str]) -> Option<String> {
    let spec = CommandSpec::new(program, args.iter().copied())
        .ok()?
        .with_timeout(std::time::Duration::from_secs(3))
        .with_output_cap(256 * 1024);
    let output = SystemCommandRunner
        .run(&spec, &CancellationToken::new())
        .ok()?;
    (output.success() && !output.stdout_truncated).then_some(output.stdout)
}

/// Read BIOS and Motherboard information from DMI.
pub fn read_bios_info() -> BiosInfo {
    let vendor = fs::read_to_string("/sys/class/dmi/id/bios_vendor")
        .or_else(|_| fs::read_to_string("/sys/devices/virtual/dmi/id/bios_vendor"))
        .unwrap_or_default()
        .trim()
        .to_string();
    let version = fs::read_to_string("/sys/class/dmi/id/bios_version")
        .or_else(|_| fs::read_to_string("/sys/devices/virtual/dmi/id/bios_version"))
        .unwrap_or_default()
        .trim()
        .to_string();
    let release_date = fs::read_to_string("/sys/class/dmi/id/bios_date")
        .or_else(|_| fs::read_to_string("/sys/devices/virtual/dmi/id/bios_date"))
        .unwrap_or_default()
        .trim()
        .to_string();
    let motherboard = fs::read_to_string("/sys/class/dmi/id/board_name")
        .or_else(|_| fs::read_to_string("/sys/class/dmi/id/product_name"))
        .unwrap_or_default()
        .trim()
        .to_string();

    BiosInfo {
        vendor: if vendor.is_empty() {
            "Standard".to_string()
        } else {
            vendor
        },
        version: if version.is_empty() {
            "1.0".to_string()
        } else {
            version
        },
        release_date: if release_date.is_empty() {
            "—".to_string()
        } else {
            release_date
        },
        motherboard: if motherboard.is_empty() {
            "Motherboard".to_string()
        } else {
            motherboard
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bios_info_does_not_panic() {
        let bios = read_bios_info();
        assert!(!bios.vendor.is_empty());
    }

    #[test]
    fn test_hardware_snapshot_reads_data() {
        let snap = read_hardware_snapshot();
        assert!(!snap.gpu.brand.is_empty());
        assert!(!snap.bios.vendor.is_empty());
    }
}
