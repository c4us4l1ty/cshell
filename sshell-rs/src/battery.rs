//! Battery: UPower via zbus (signal-driven) + sysfs fallback (no fork).
//! Replaces Battery.qml 5s Timer + get_battery.sh + bc.

use std::fs;

#[derive(Debug, Clone, Default)]
pub struct BatteryState {
    pub pct: Option<u8>,      // 0..100
    pub charging: bool,       // status == Charging
    pub plugged: bool,        // mains online
    pub time_empty_s: u64,
    pub time_full_s: u64,
}

fn read_trim(path: &str) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

fn read_u64_digits(path: &str) -> Option<u64> {
    read_trim(path).and_then(|s| {
        s.chars().filter(|c| c.is_ascii_digit()).collect::<String>().parse::<u64>().ok()
    })
}

/// Aggregate all system batteries (BAT*, energy/charge/capacity), mains online.
/// Mirrors battery-dimmer telemetry math (µWh/µAh normalize) in simplified form.
pub fn read_sysfs() -> BatteryState {
    let mut total_now: u64 = 0;
    let mut total_full: u64 = 0;
    let mut cap_sum: u64 = 0;
    let mut cap_n: u64 = 0;
    let mut any_charging = false;
    let mut mains_online = false;

    let rd = fs::read_dir("/sys/class/power_supply").map(|r| r.filter_map(|e| e.ok()).collect::<Vec<_>>()).unwrap_or_default();
    for e in rd {
        let dev = e.path();
        let typ = read_trim(&format!("{}", dev.join("type").display())).unwrap_or_default().to_lowercase();
        let name = dev.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
        if name.contains("hid") || name.contains("mouse") || name.contains("kbd") || name.contains("wacom") || name.contains("bluetooth") {
            continue;
        }
        if typ == "mains" || typ.starts_with("usb") || typ == "wireless" || typ == "brickid" {
            if read_trim(&format!("{}", dev.join("online").display())).as_deref() == Some("1") {
                mains_online = true;
            }
            continue;
        }
        if typ != "battery" && typ != "ups" {
            continue;
        }
        if let Some(st) = read_trim(&format!("{}", dev.join("status").display())) {
            if st.to_lowercase() == "charging" {
                any_charging = true;
            }
        }
        // energy_now/energy_full (µWh)
        if let (Some(now), Some(full)) = (
            read_u64_digits(&format!("{}", dev.join("energy_now").display())),
            read_u64_digits(&format!("{}", dev.join("energy_full").display())),
        ) {
            if full >= 1000 {
                total_now += now / 1000;
                total_full += full / 1000;
                continue;
            }
        }
        // charge_now/charge_full (µAh) + voltage normalize when available
        if let (Some(now), Some(full)) = (
            read_u64_digits(&format!("{}", dev.join("charge_now").display())),
            read_u64_digits(&format!("{}", dev.join("charge_full").display())),
        ) {
            if full >= 1000 {
                let vnow = read_u64_digits(&format!("{}", dev.join("voltage_now").display()))
                    .or_else(|| read_u64_digits(&format!("{}", dev.join("voltage_min_design").display())))
                    .unwrap_or(0);
                if vnow > 100_000 {
                    let mv = vnow / 1000;
                    total_now += (now / 1000) * mv / 1000;
                    total_full += (full / 1000) * mv / 1000;
                } else {
                    // fall back to percentage math
                    let pct = now * 100 / full.max(1);
                    cap_sum += pct.min(100);
                    cap_n += 1;
                }
                continue;
            }
        }
        if let Some(cap) = read_u64_digits(&format!("{}", dev.join("capacity").display())) {
            cap_sum += cap.min(100);
            cap_n += 1;
        }
    }

    let pct = if total_full > 0 {
        Some(((total_now * 100 / total_full.max(1)).min(100)) as u8)
    } else if cap_n > 0 {
        Some((cap_sum / cap_n.max(1)).min(100) as u8)
    } else {
        None
    };

    BatteryState {
        pct,
        charging: any_charging,
        plugged: mains_online || any_charging,
        time_empty_s: 0,
        time_full_s: 0,
    }
}

/// Text for bar module. Same as QML Battery popup trigger label.
pub fn bar_text(s: &BatteryState) -> String {
    match s.pct {
        Some(p) => {
            let icon = if s.charging { "󰂄" } else if p > 80 { "󰁹" } else if p > 50 { "󰁾" } else { "󰁻" };
            format!("{} {}%", icon, p)
        }
        None => "󰂃 --%".into(),
    }
}

/// Should dimmer be active? Same hysteresis as battery-dimmer (50/53).
pub fn should_dim(pct: u8, dimmed: bool) -> bool {
    const THRESHOLD: u8 = 50;
    const HYST: u8 = 3;
    if dimmed {
        pct <= THRESHOLD + HYST
    } else {
        pct <= THRESHOLD
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hysteresis() {
        assert!(should_dim(50, false));
        assert!(!should_dim(51, false));
        assert!(should_dim(53, true));
        assert!(!should_dim(54, true));
    }
    #[test]
    fn bar_text_shapes() {
        assert!(bar_text(&BatteryState { pct: Some(74), charging: true, ..Default::default() }).contains("74%"));
        assert!(bar_text(&BatteryState { pct: None, ..Default::default() }).contains("--"));
    }
}
