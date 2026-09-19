//! Audio: PipeWire-native next, wpctl-on-demand now.
//! Rule: NO polling. wpctl runs only on user action (key/slider) or explicit refresh.
//! OSD state mirrors QML AudioService + ControlCenter SliderRows.

use std::process::Command;

#[derive(Debug, Clone, Default)]
pub struct AudioState {
    pub volume01: f64, // 0..1
    pub muted: bool,
}

fn wpctl(args: &[&str]) -> Option<String> {
    let out = Command::new("wpctl").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Read-only refresh, called on ControlCenter open / OSD trigger — never on a timer.
/// Returns 0.0 (not phantom 0.5) when PipeWire has no default sink.
pub fn refresh() -> AudioState {
    // `wpctl get-volume @DEFAULT_AUDIO_SINK@` -> "Volume: 0.45 [MUTED]" variants
    let mut st = AudioState { volume01: 0.0, muted: false };
    let mut _have_sink = false;
    if let Some(line) = wpctl(&["get-volume", "@DEFAULT_AUDIO_SINK@"]) {
        _have_sink = true;
        let _ = have_sink;
        // parse first float in line
        let mut num = String::new();
        let mut started = false;
        for c in line.chars() {
            if c.is_ascii_digit() || c == '.' {
                num.push(c);
                started = true;
            } else if started {
                break;
            }
        }
        if let Ok(v) = num.parse::<f64>() {
            st.volume01 = v.clamp(0.0, 1.0);
        }
        if line.contains("MUTED") || line.contains("MUTE") {
            st.muted = true;
        }
    }
    st
}

pub fn set_volume(frac: f64) {
    let v = frac.clamp(0.0, 1.0);
    let pct = (v * 100.0).round() as i64;
    // standard UX: volume keys unmute (wpctl set-volume alone leaves MUTE on)
    let _ = Command::new("wpctl")
        .args(["set-mute", "@DEFAULT_AUDIO_SINK@", "0"])
        .output();
    // single writer: one wpctl call per user gesture (no double-step)
    let _ = Command::new("wpctl")
        .args(["set-volume", "@DEFAULT_AUDIO_SINK@", &format!("{}%", pct), "-l", "1.0"])
        .output();
}

pub fn toggle_mute() {
    let _ = Command::new("wpctl")
        .args(["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"])
        .output();
}
