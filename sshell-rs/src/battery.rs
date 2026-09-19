//! Battery: UPower via zbus (signal-driven) + sysfs fallback (no fork).
//! Replaces Battery.qml 5s Timer + get_battery.sh + bc.

use std::fs;

#[derive(Debug, Clone, Default)]
pub struct BatteryState {
    pub pct: Option<u8>,      // 0..100
    pub charging: bool,       // status == Charging
    pub plugged: bool,        // mains online
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

#[derive(Debug, Clone, Default)]
pub struct Details {
    pub pct: Option<u8>,
    pub status: String,
    pub health_pct: Option<f64>,
    pub rate_w: Option<f64>,
    pub time_hm: Option<String>,
}

/// On-demand details for Battery popup (same fields as QML BatteryPopup).
/// Reads sysfs directly, no forks. Called on popup open / battery wake only.
pub fn details() -> Details {
    let st = read_sysfs();
    let mut d = Details { pct: st.pct, status: String::new(), health_pct: None, rate_w: None, time_hm: None };
    let rd = std::fs::read_dir("/sys/class/power_supply").map(|r| r.filter_map(|e| e.ok()).collect::<Vec<_>>()).unwrap_or_default();
    for e in rd {
        let dev = e.path();
        let typ = read_trim(&format!("{}", dev.join("type").display())).unwrap_or_default().to_lowercase();
        if typ != "battery" {
            continue;
        }
        d.status = read_trim(&format!("{}", dev.join("status").display())).unwrap_or_default();
        let full = read_u64_digits(&format!("{}", dev.join("energy_full").display()))
            .or_else(|| read_u64_digits(&format!("{}", dev.join("charge_full").display())));
        let design = read_u64_digits(&format!("{}", dev.join("energy_full_design").display()))
            .or_else(|| read_u64_digits(&format!("{}", dev.join("charge_full_design").display())));
        if let (Some(f), Some(dsg)) = (full, design) {
            if dsg > 0 {
                d.health_pct = Some(f as f64 * 100.0 / dsg as f64);
            }
        }
        let pnow = read_u64_digits(&format!("{}", dev.join("power_now").display()));
        if let Some(p) = pnow {
            d.rate_w = Some(p as f64 / 1_000_000.0);
            // time estimate (guard divide-by-zero without extra branch noise)
            let now = read_u64_digits(&format!("{}", dev.join("energy_now").display())).unwrap_or(0);
            let num = if d.status == "Charging" {
                full.unwrap_or(0).saturating_sub(now)
            } else {
                now
            };
            if let Some(secs) = num.checked_mul(3600).and_then(|n| n.checked_div(p.max(1))) {
                d.time_hm = Some(format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60));
            }
        }
        break; // primary battery only (multi-battery aggregate already in bar %)
    }
    d
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
