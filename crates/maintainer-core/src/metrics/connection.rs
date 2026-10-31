//! Network connection details collector.
//!
//! Reads `/proc/net/route`, `/etc/resolv.conf`, `/proc/net/wireless` and runs
//! `ip` and `iw` for adapter details. Isolated from the UI layer per the
//! architecture boundary: UI must never read Linux interfaces or run commands.

/// Connection details for the active network adapter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConnectionDetails {
    /// Primary IP address (e.g., "192.168.1.42/24").
    pub ip_address: Option<String>,
    /// Default gateway IP address (e.g., "192.168.1.1").
    pub gateway: Option<String>,
    /// DNS servers from resolv.conf.
    pub dns_servers: Vec<String>,
    /// WiFi details, if the default interface is wireless.
    pub wifi: Option<WifiInfo>,
}

/// WiFi connection metadata from `iw`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WifiInfo {
    /// Connected SSID.
    pub ssid: String,
    /// Signal strength (e.g., "-45 dBm").
    pub signal: String,
    /// TX bitrate (e.g., "72.2 MBit/s").
    pub link_speed: String,
    /// Operating frequency (e.g., "5180 MHz").
    pub frequency: String,
}

/// Read network connection details from Linux interfaces.
///
/// Returns `None` fields for anything that cannot be read rather than
/// panicking. This function is safe to call from any thread.
pub fn read_connection_details() -> ConnectionDetails {
    read_connection_details_cancellable(&crate::jobs::CancellationToken::new())
}

pub fn read_connection_details_cancellable(
    cancel: &dyn crate::jobs::Cancellation,
) -> ConnectionDetails {
    if cancel.is_cancelled() {
        return ConnectionDetails::default();
    }
    let runner = crate::command::SystemCommandRunner;
    let default_iface = read_default_interface("/proc/net/route");
    let ip_address = default_iface
        .as_deref()
        .and_then(|iface| read_ip_for_interface(iface, &runner, cancel));
    let gateway = read_default_gateway_from("/proc/net/route");
    let dns_servers = read_dns_from("/etc/resolv.conf").unwrap_or_default();
    let wifi = default_iface
        .as_deref()
        .and_then(|iface| read_wifi_for_interface(iface, &runner, cancel));

    ConnectionDetails {
        ip_address,
        gateway,
        dns_servers,
        wifi,
    }
}

/// Find the default route interface from `/proc/net/route`.
fn read_default_interface(route_path: &str) -> Option<String> {
    let content = std::fs::read_to_string(route_path).ok()?;
    parse_default_interface(&content)
}

/// Parse the default interface from route table content.
pub fn parse_default_interface(content: &str) -> Option<String> {
    content
        .lines()
        .skip(1)
        .find(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            fields.get(1).is_some_and(|dest| *dest == "00000000")
        })
        .and_then(|line| line.split_whitespace().next())
        .map(|s| s.to_string())
}

/// Parse the default gateway from route table content.
/// Gateway field is hex-encoded little-endian IPv4 address.
pub fn parse_gateway_hex(content: &str) -> Option<String> {
    for line in content.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.get(1) == Some(&"00000000") {
            if let Some(hex) = fields.get(2) {
                if let Ok(val) = u32::from_str_radix(hex, 16) {
                    let bytes = val.to_le_bytes();
                    return Some(format!(
                        "{}.{}.{}.{}",
                        bytes[0], bytes[1], bytes[2], bytes[3]
                    ));
                }
            }
        }
    }
    None
}

/// Read default gateway from the route file.
fn read_default_gateway_from(route_path: &str) -> Option<String> {
    let content = std::fs::read_to_string(route_path).ok()?;
    parse_gateway_hex(&content)
}

/// Parse DNS nameservers from resolv.conf content.
pub fn parse_resolv_conf(content: &str) -> Vec<String> {
    content
        .lines()
        .filter(|l| l.starts_with("nameserver"))
        .filter_map(|l| l.split_whitespace().nth(1))
        .map(|s| s.to_string())
        .collect()
}

/// Read DNS servers from resolv.conf file.
fn read_dns_from(resolv_path: &str) -> Option<Vec<String>> {
    let content = std::fs::read_to_string(resolv_path).ok()?;
    let servers = parse_resolv_conf(&content);
    if servers.is_empty() {
        None
    } else {
        Some(servers)
    }
}

/// Read IP address for a given interface using `ip` command.
fn read_ip_for_interface(
    iface: &str,
    runner: &dyn crate::command::CommandRunner,
    cancel: &dyn crate::jobs::Cancellation,
) -> Option<String> {
    let spec = crate::command::CommandSpec::new("ip", ["-4", "-o", "addr", "show", "dev", iface])
        .ok()?
        .with_timeout(std::time::Duration::from_secs(3))
        .with_output_cap(64 * 1024);
    let output = runner.run(&spec, cancel).ok()?;
    if !output.success() || output.stdout_truncated {
        return None;
    }
    parse_ip_from_output(&output.stdout)
}

/// Parse IP/prefix from `ip -4 -o addr show` output.
pub fn parse_ip_from_output(output: &str) -> Option<String> {
    for part in output.split_whitespace() {
        if part.contains('.') && part.contains('/') {
            return Some(part.to_string());
        }
    }
    None
}

/// Read WiFi details for a given interface using `/proc/net/wireless` and `iw`.
fn read_wifi_for_interface(
    iface: &str,
    runner: &dyn crate::command::CommandRunner,
    cancel: &dyn crate::jobs::Cancellation,
) -> Option<WifiInfo> {
    // Verify this interface is wireless.
    let wireless = std::fs::read_to_string("/proc/net/wireless").ok()?;
    if !is_wireless_interface(&wireless, iface) {
        return None;
    }

    let spec = crate::command::CommandSpec::new("iw", ["dev", iface, "link"])
        .ok()?
        .with_timeout(std::time::Duration::from_secs(3))
        .with_output_cap(64 * 1024);
    let output = runner.run(&spec, cancel).ok()?;
    if !output.success() || output.stdout_truncated {
        return None;
    }
    Some(parse_iw_link_output(&output.stdout))
}

/// Check if a given interface appears in `/proc/net/wireless`.
pub fn is_wireless_interface(wireless_content: &str, iface: &str) -> bool {
    wireless_content.lines().skip(2).any(|line| {
        line.trim()
            .split(':')
            .next()
            .is_some_and(|name| name.trim() == iface)
    })
}

/// Parse `iw dev <iface> link` output into WiFi details.
pub fn parse_iw_link_output(output: &str) -> WifiInfo {
    let ssid = output
        .lines()
        .find(|l| l.contains("SSID:"))
        .and_then(|l| l.split("SSID:").nth(1))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "Unavailable".to_string());

    let signal = output
        .lines()
        .find(|l| l.contains("signal:"))
        .and_then(|l| l.split("signal:").nth(1))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "Unavailable".to_string());

    let link_speed = output
        .lines()
        .find(|l| l.contains("tx bitrate:"))
        .and_then(|l| l.split("tx bitrate:").nth(1))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "Unavailable".to_string());

    let frequency = output
        .lines()
        .find(|l| l.contains("freq:"))
        .and_then(|l| l.split("freq:").nth(1))
        .map(|s| format!("{} MHz", s.trim()))
        .unwrap_or_else(|| "Unavailable".to_string());

    WifiInfo {
        ssid,
        signal,
        link_speed,
        frequency,
    }
}

/// Parse wireless content to extract the first wireless interface name.
pub fn parse_wireless_interface(content: &str) -> Option<String> {
    content
        .lines()
        .nth(2)
        .and_then(|line| line.split(':').next())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct RecordingRunner(Mutex<Option<crate::command::CommandSpec>>);

    impl crate::command::CommandRunner for RecordingRunner {
        fn run(
            &self,
            spec: &crate::command::CommandSpec,
            _cancel: &dyn crate::jobs::Cancellation,
        ) -> crate::error::Result<crate::command::CommandOutput> {
            *self.0.lock().unwrap() = Some(spec.clone());
            Ok(crate::command::CommandOutput {
                exit_code: Some(0),
                stdout: "2: eth0 inet 192.0.2.8/24 scope global eth0".into(),
                stderr: String::new(),
                stdout_truncated: false,
                stderr_truncated: false,
            })
        }
    }

    #[test]
    fn ip_details_use_bounded_runner_and_literal_arguments() {
        let runner = RecordingRunner(Mutex::new(None));
        let cancel = crate::jobs::CancellationToken::new();
        let result = read_ip_for_interface("eth0; touch /tmp/unwanted", &runner, &cancel);

        assert_eq!(result.as_deref(), Some("192.0.2.8/24"));
        let spec = runner.0.lock().unwrap().clone().unwrap();
        assert_eq!(spec.program(), "ip");
        assert_eq!(
            spec.args(),
            [
                "-4",
                "-o",
                "addr",
                "show",
                "dev",
                "eth0; touch /tmp/unwanted"
            ]
        );
        assert_eq!(spec.timeout(), std::time::Duration::from_secs(3));
        assert_eq!(spec.output_cap_bytes(), 64 * 1024);
    }

    #[test]
    fn parse_gateway_hex_little_endian() {
        let content = "Iface\tDestination\tGateway\n\
                       eth0\t00000000\t0101A8C0\t0003\t0\t0\t600\t00000000\n";
        let gw = parse_gateway_hex(content).unwrap();
        assert_eq!(gw, "192.168.1.1");
    }

    #[test]
    fn parse_gateway_hex_different_address() {
        let content = "Iface\tDestination\tGateway\n\
                       wlan0\t00000000\tFE01A8C0\t0003\t0\t0\t600\t00000000\n";
        let gw = parse_gateway_hex(content).unwrap();
        assert_eq!(gw, "192.168.1.254");
    }

    #[test]
    fn parse_gateway_no_default_route_returns_none() {
        let content = "Iface\tDestination\tGateway\n\
                       eth0\t00C0A8C0\t00000000\t0001\t0\t0\t600\t00FFFFFF\n";
        assert_eq!(parse_gateway_hex(content), None);
    }

    #[test]
    fn dns_from_resolv_conf_fixture() {
        let content = "nameserver 8.8.8.8\nnameserver 8.8.4.4\n";
        let servers = parse_resolv_conf(content);
        assert_eq!(servers, vec!["8.8.8.8", "8.8.4.4"]);
    }

    #[test]
    fn dns_ignores_comments_and_other_directives() {
        let content = "# comment\nsearch example.com\nnameserver 1.1.1.1\noptions ndots:2\n";
        let servers = parse_resolv_conf(content);
        assert_eq!(servers, vec!["1.1.1.1"]);
    }

    #[test]
    fn dns_empty_resolv_returns_empty() {
        let content = "# empty resolv.conf\n";
        let servers = parse_resolv_conf(content);
        assert!(servers.is_empty());
    }

    #[test]
    fn parse_default_interface_from_route() {
        let content = "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\n\
                       eth0\t00000000\t0101A8C0\t0003\t0\t0\t600\t00000000\n\
                       eth0\t00C0A8C0\t00000000\t0001\t0\t0\t600\t00FFFFFF\n";
        let iface = parse_default_interface(content).unwrap();
        assert_eq!(iface, "eth0");
    }

    #[test]
    fn parse_default_interface_no_default_returns_none() {
        let content = "Iface\tDestination\tGateway\n\
                       eth0\t00C0A8C0\t00000000\t0001\t0\t0\t600\t00FFFFFF\n";
        assert_eq!(parse_default_interface(content), None);
    }

    #[test]
    fn parse_ip_from_addr_show_output() {
        let output = "2: eth0    inet 192.168.1.42/24 brd 192.168.1.255 scope global eth0\n";
        let ip = parse_ip_from_output(output).unwrap();
        assert_eq!(ip, "192.168.1.42/24");
    }

    #[test]
    fn is_wireless_detects_interface() {
        let content = "Inter-| sta-|   Quality\n face |tus |   ...\n wlan0: 0000 ...\n";
        assert!(is_wireless_interface(content, "wlan0"));
        assert!(!is_wireless_interface(content, "eth0"));
    }

    #[test]
    fn empty_wireless_no_interface() {
        let content = "Inter-| sta-|   Quality\n face |tus |   ...\n";
        assert_eq!(parse_wireless_interface(content), None);
    }

    #[test]
    fn parse_iw_link_extracts_fields() {
        let output = "\tConnected to aa:bb:cc:dd:ee:ff\n\
                      \tSSID: MyNetwork\n\
                      \tfreq: 5180\n\
                      \tsignal: -42 dBm\n\
                      \ttx bitrate: 866.7 MBit/s\n";
        let wifi = parse_iw_link_output(output);
        assert_eq!(wifi.ssid, "MyNetwork");
        assert_eq!(wifi.signal, "-42 dBm");
        assert_eq!(wifi.link_speed, "866.7 MBit/s");
        assert_eq!(wifi.frequency, "5180 MHz");
    }

    #[test]
    fn parse_iw_link_missing_fields_returns_unavailable() {
        let output = "Not connected.\n";
        let wifi = parse_iw_link_output(output);
        assert_eq!(wifi.ssid, "Unavailable");
        assert_eq!(wifi.signal, "Unavailable");
    }
}
