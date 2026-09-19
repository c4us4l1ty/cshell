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
    pub bt_connected: bool,
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
