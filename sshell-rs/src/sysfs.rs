//! sysfs backlight: sole manual writer. Dimmer owns auto-dim; we own keys.
//! Shared override flag lets dimmer yield: /run/sshell/backlight-override.

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

pub const MIN_PERCENT_FLOOR: f64 = 0.12; // legibility floor on AUO panel
pub const STEP_FRAC: f64 = 0.05;

pub fn candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(rd) = fs::read_dir("/sys/class/backlight") {
        for e in rd.flatten() {
            let p = e.path();
            if p.join("brightness").exists() && p.join("max_brightness").exists() {
                v.push(p);
            }
        }
    }
    v.sort_by_key(|p| {
        let n = p.file_name().unwrap_or_default().to_string_lossy().to_string();
        if n.contains("intel") {
            0
        } else if n.contains("amdgpu") || n.contains("nvidia") {
            1
        } else {
            2
        }
    });
    v
}

pub fn read_u32(path: &PathBuf) -> Option<u32> {
    fs::read_to_string(path)
        .ok()?
        .trim()
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse::<u32>()
        .ok()
}

/// Current brightness fraction 0..1 for primary device.
pub fn current_frac() -> Option<f64> {
    let bl = candidates().into_iter().next()?;
    let max = read_u32(&bl.join("max_brightness"))? as f64;
    let cur = read_u32(&bl.join("brightness"))? as f64;
    if max <= 0.0 {
        return None;
    }
    Some((cur / max).clamp(0.0, 1.0))
}

fn set_override_flag() {
    let _ = fs::create_dir_all("/run/sshell");
    let _ = fs::write("/run/sshell/backlight-override", b"1");
}

pub fn change(delta_frac: f64) -> Result<()> {
    let cands = candidates();
    if cands.is_empty() {
        anyhow::bail!("no /sys/class/backlight/* writable device");
    }
    for bl in cands {
        let max = read_u32(&bl.join("max_brightness")).unwrap_or(255).max(1);
        let cur = read_u32(&bl.join("brightness")).unwrap_or(max / 2);
        let min = (max as f64 * MIN_PERCENT_FLOOR) as u32;
        let mut target = (cur as f64 + delta_frac * max as f64).round() as i64;
        target = target.clamp(min as i64, max as i64);
        fs::write(bl.join("brightness"), format!("{}\n", target))
            .with_context(|| format!("write {}", bl.display()))?;
        set_override_flag();
        tracing::info!("brightness {}: {} -> {} (max {})", bl.display(), cur, target, max);
        break;
    }
    Ok(())
}

pub fn topology_lines() -> Vec<String> {
    candidates()
        .into_iter()
        .map(|bl| {
            let cur = read_u32(&bl.join("brightness")).unwrap_or(0);
            let max = read_u32(&bl.join("max_brightness")).unwrap_or(0);
            format!("backlight {} cur={} max={}", bl.display(), cur, max)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_math() {
        let max = 21333u32; // intel_backlight on IdeaPad (from --status)
        let min = (max as f64 * MIN_PERCENT_FLOOR) as u32;
        assert!(min > 2000 && min < 3000);
        // 30% linear step from 7743 would land below legibility floor → clamped to min
        let raw = (7743f64 - 0.30 * max as f64).round() as i64;
        let clamped = raw.clamp(min as i64, max as i64);
        assert_eq!(clamped, min as i64);
        // 5% key step from mid stays in range
        let step = (10000f64 + 0.05 * max as f64).round() as i64;
        assert!(step > min as i64 && step < max as i64);
    }
}
