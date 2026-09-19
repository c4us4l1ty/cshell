//! Network + Bluetooth: zbus signal-driven primary, sysfs/nmcli on-demand fallback.
//! Replaces Network.qml 5s Timer + nmcli fork chain. No auto rescan.

use std::fs;

#[derive(Debug, Clone, Default)]
pub struct NetState {
    pub wifi_enabled: bool,
    pub wifi_connected: bool,
    pub ssid: String,
    pub signal: u8,
    pub eth_connected: bool,
    pub eth_iface: String,
    /// Bluetooth live state is intentionally NOT polled here: BlueZ RSSI
    /// PropertiesChanged can fire several times per second, and resolving names
    /// would fork bluetoothctl per signal (fork storm). BT surfaces on demand
    /// via bt_list() when the detail popup opens. Fields kept for API parity.
    #[allow(dead_code)]
    pub bt_connected: bool,
    #[allow(dead_code)]
    pub bt_name: String,
}

/// Cheap read-only snapshot without forking nmcli (operstate + wireless links).
/// Full SSID/signal arrives via NetworkManager D-Bus in app layer; this is the
/// offline-safe fallback used by --check.
pub fn snapshot_offline() -> NetState {
    let mut st = NetState::default();
    // ethernet / wifi operstate guess via /sys/class/net
    if let Ok(rd) = fs::read_dir("/sys/class/net") {
        for e in rd.flatten() {
            let iface = e.file_name().to_string_lossy().to_string();
            if iface == "lo" {
                continue;
            }
            let op = fs::read_to_string(e.path().join("operstate"))
                .unwrap_or_default()
                .trim()
                .to_string();
            let up = op == "up" || op == "dormant";
            if iface.starts_with('e') {
                // en* / eth*
                if up {
                    st.eth_connected = true;
                    st.eth_iface = iface;
                }
            } else if iface.starts_with('w') {
                st.wifi_enabled = true; // interface exists; radio state refined via D-Bus
                if up {
                    st.wifi_connected = true;
                }
            }
        }
    }
    st
}

pub fn bar_text(st: &NetState, show_name: bool) -> String {
    if st.eth_connected {
        return if show_name && !st.eth_iface.is_empty() {
            format!("󰈀 {}", st.eth_iface)
        } else {
            "󰈀 eth".into()
        };
    }
    if st.wifi_connected {
        let bars = match st.signal {
            75..=100 => "󰤨",
            50..=74 => "󰤥",
            25..=49 => "󰤢",
            _ => "󰤟",
        };
        if show_name && !st.ssid.is_empty() {
            return format!("{} {}", bars, st.ssid);
        }
        return format!("{} wifi", bars);
    }
    if st.wifi_enabled {
        return "󰤭 off".into();
    }
    "󰤭 down".into()
}

/// On-demand WiFi scan list for the detail popup. Runs `nmcli` ONLY when the
/// popup opens / Rescan is pressed (user action) — never background.
/// Output capped to 12 lines to keep the popup readable.
pub fn wifi_list() -> String {
    let out = std::process::Command::new("nmcli")
        .args(["-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY", "device", "wifi", "list", "--rescan", "no"])
        .output();
    let out = match out {
        Ok(o) if o.status.success() => o,
        _ => return "nmcli unavailable".into(),
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut rows = vec![];
    for line in text.lines().take(12) {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() < 3 {
            continue;
        }
        let mark = if parts[0] == "*" { "●" } else { "○" };
        let ssid = if parts[1].is_empty() { "(hidden)" } else { parts[1] };
        rows.push(format!("{} {}  {}%", mark, ssid, parts[2]));
    }
    if rows.is_empty() {
        "No networks (radio off?)".into()
    } else {
        rows.join("\n")
    }
}

/// On-demand Bluetooth device list for the detail popup (user action only).
pub fn bt_list() -> String {
    let out = std::process::Command::new("bluetoothctl")
        .args(["devices"])
        .output();
    let out = match out {
        Ok(o) if o.status.success() => o,
        _ => return "bluetoothctl unavailable".into(),
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let rows: Vec<String> = text
        .lines()
        .take(12)
        .filter_map(|l| {
            let l = l.strip_prefix("Device ")?;
            let mut it = l.splitn(2, ' ');
            Some(format!("󰂯 {}", it.nth(1).unwrap_or(it.next().unwrap_or("?"))))
        })
        .collect();
    if rows.is_empty() {
        "No devices paired".into()
    } else {
        rows.join("\n")
    }
}
