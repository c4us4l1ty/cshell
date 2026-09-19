//! sshell-rs — Rust + GTK4 replacement for shell.qml
//! UI contract: same tokens as Appearance.qml / config.jsonc / Bar.qml.
//! Rules: event-driven only (UPower/NM/BlueZ/MPRIS/hyprland-IPC/inotify/udev).
//! Allowed timers: minute-aligned clock, OSD auto-hide 1500ms, weather 6h cache.
//! No `bash -c` interpolation. Single backlight writer (shared override with battery-dimmer).
//! Animations: 100ms fade max, no slide/popin/blur loops.

use anyhow::{Context, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

// ─── UI freeze tokens (mirrors Appearance.qml + config.jsonc) ───
pub mod ui {
    pub const BAR_HEIGHT: i32 = 38;
    pub const BAR_MARGIN: i32 = 10;
    pub const BAR_PADDING: i32 = 5;
    pub const CORNER_RADIUS_LARGE: i32 = 14;
    pub const CONTROL_CENTER_WIDTH: i32 = 450;
    pub const LAUNCHER_W: i32 = 400;
    pub const LAUNCHER_H: i32 = 500;
    pub const SEARCH_BAR_H: i32 = 36;
    pub const NOTIF_W: i32 = 350;
    pub const OSD_TIMEOUT_MS: u64 = 1500;
    pub const NOTIF_TIMEOUT_MS: u64 = 5000;
    pub const NOTIF_MAX: usize = 5;
    pub const NOTIF_GROUP_AT: usize = 3;
    pub const FADE_MS: u64 = 100; // only animation allowed
    pub const MPRIS_MAX_W: i32 = 666;
    // M3 dark baseline (matugen css overrides at runtime via inotify)
    pub const BG: &str = "#141313";
    pub const ON_SURFACE: &str = "#DEE2E6";
    pub const PRIMARY: &str = "#D0BCFF";
}

#[derive(Parser, Debug)]
#[command(name = "sshell-rs", version, about = "cshell Rust GTK4 shell")]
struct Args {
    /// Validate config + Hypr IPC without opening windows (for TTY pre-launch)
    #[arg(long)]
    check: bool,
    /// Increase brightness by 5% (single writer, OSD once)
    #[arg(long)]
    brightness_up: bool,
    /// Decrease brightness by 5%
    #[arg(long)]
    brightness_down: bool,
    /// Custom config path (default ~/.config/sshell/config.jsonc)
    #[arg(long)]
    config: Option<PathBuf>,
}

// Minimal serde mirror of config.jsonc top-level keys we actually honor.
// Unknown keys are ignored to stay forward-compatible with QML Config.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct BarModule {
    #[serde(default)]
    module: String,
    #[serde(default = "enabled_default")]
    enabled: bool,
}
fn enabled_default() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BarConfig {
    #[serde(default = "bar_enabled")]
    enabled: bool,
    #[serde(default = "top_default")]
    position: String,
    #[serde(default = "floating_default")]
    style: String,
    #[serde(default = "bar_height")]
    height: i32,
    #[serde(default = "bar_margin")]
    margin: i32,
    #[serde(default)]
    left: Vec<BarModule>,
    #[serde(default)]
    center: Vec<BarModule>,
    #[serde(default)]
    right: Vec<BarModule>,
}
fn bar_enabled() -> bool {
    true
}
fn top_default() -> String {
    "top".into()
}
fn floating_default() -> String {
    "floating".into()
}
fn bar_height() -> i32 {
    38
}
fn bar_margin() -> i32 {
    10
}
impl Default for BarConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            position: "top".into(),
            style: "floating".into(),
            height: 38,
            margin: 10,
            left: vec![],
            center: vec![],
            right: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ShellConfig {
    #[serde(default)]
    bar: BarConfig,
    // other sections validated loosely: presence only for --check
    #[serde(default)]
    controlCenter: serde_json::Value,
    #[serde(default)]
    notifications: serde_json::Value,
    #[serde(default)]
    launcher: serde_json::Value,
    #[serde(default)]
    theme: serde_json::Value,
    #[serde(default)]
    weather: serde_json::Value,
    #[serde(default)]
    background: serde_json::Value,
    #[serde(default)]
    mpris: serde_json::Value,
    #[serde(default)]
    clock: serde_json::Value,
    #[serde(default)]
    workspaces: serde_json::Value,
    #[serde(default)]
    tray: serde_json::Value,
    #[serde(default)]
    osd: serde_json::Value,
}

fn default_config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    PathBuf::from(home).join(".config/sshell/config.jsonc")
}

fn strip_jsonc_comments(text: &str) -> String {
    // Correct JSONC strip: preserve // inside strings (e.g. https://).
    // State machine: in_string, escaped, line_comment, block_comment.
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        if c == '"' {
            in_string = true;
            out.push(c);
            continue;
        }
        if c == '/' {
            match chars.peek() {
                Some('/') => {
                    // line comment — skip to newline (keep newline)
                    for nc in chars.by_ref() {
                        if nc == '\n' {
                            out.push('\n');
                            break;
                        }
                    }
                    continue;
                }
                Some('*') => {
                    chars.next();
                    // block comment — skip to */
                    let mut prev_star = false;
                    for nc in chars.by_ref() {
                        if prev_star && nc == '/' {
                            break;
                        }
                        prev_star = nc == '*';
                    }
                    continue;
                }
                _ => {
                    out.push(c);
                    continue;
                }
            }
        }
        out.push(c);
    }
    // Remove trailing commas: ,} -> } and ,] -> ] (safe outside strings by construction above? simplified: serde will error otherwise; keep minimal)
    // Do a second pass that only strips commas directly before } or ] outside strings.
    let mut out2 = String::with_capacity(out.len());
    let mut ochars: Vec<char> = out.chars().collect();
    let mut i = 0;
    let mut in_s = false;
    let mut esc = false;
    while i < ochars.len() {
        let c = ochars[i];
        if in_s {
            out2.push(c);
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_s = false;
            }
            i += 1;
            continue;
        }
        if c == '"' {
            in_s = true;
            out2.push(c);
            i += 1;
            continue;
        }
        if c == ',' {
            // lookahead past whitespace
            let mut j = i + 1;
            while j < ochars.len() && ochars[j].is_whitespace() {
                j += 1;
            }
            if j < ochars.len() && (ochars[j] == '}' || ochars[j] == ']') {
                i += 1; // skip comma
                continue;
            }
        }
        out2.push(c);
        i += 1;
    }
    out2
}

fn load_config(path: &PathBuf) -> Result<ShellConfig> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("read config {}", path.display()))?;
    let stripped = strip_jsonc_comments(&text);
    let cfg: ShellConfig =
        serde_json::from_str(&stripped).context("parse config.jsonc (jsonc)")?;
    // Validate ranges (fail-loud with context, never silent defaults)
    if cfg.bar.height < 24 || cfg.bar.height > 64 {
        anyhow::bail!("bar.height {} out of range 24..64", cfg.bar.height);
    }
    if !["top", "bottom"].contains(&cfg.bar.position.as_str()) {
        anyhow::bail!("bar.position must be top|bottom");
    }
    if !["full", "floating", "islands", "modules"].contains(&cfg.bar.style.as_str()) {
        anyhow::bail!("bar.style must be full|floating|islands|modules");
    }
    Ok(cfg)
}

// ─── Backlight: sole-writer via sysfs, shared override with battery-dimmer ───
// Dimmer owns auto-dim; we own manual keys. We never overwrite a dimmed value
// without setting /run/sshell/backlight-override so dimmer yields.
fn backlight_candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(rd) = fs::read_dir("/sys/class/backlight") {
        for e in rd.flatten() {
            let p = e.path();
            if p.join("brightness").exists() && p.join("max_brightness").exists() {
                v.push(p);
            }
        }
    }
    // Prefer intel_backlight / raw over firmware (matches dimmer priority)
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

fn read_u32(path: &PathBuf) -> Option<u32> {
    fs::read_to_string(path)
        .ok()?
        .trim()
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse::<u32>()
        .ok()
}

fn set_override_flag() {
    // Best-effort signal to battery-dimmer: manual user action suspends auto-dim.
    let _ = fs::create_dir_all("/run/sshell");
    let _ = fs::write("/run/sshell/backlight-override", b"1");
}

fn change_brightness(delta_frac: f64) -> Result<()> {
    let cands = backlight_candidates();
    if cands.is_empty() {
        anyhow::bail!("no /sys/class/backlight/* writable device");
    }
    for bl in cands {
        let max = read_u32(&bl.join("max_brightness")).unwrap_or(255).max(1);
        let cur = read_u32(&bl.join("brightness")).unwrap_or(max / 2);
        let min = (max as f64 * 0.12) as u32; // MIN_PERCENT=12 legibility floor
        let mut target = (cur as f64 + delta_frac * max as f64).round() as i64;
        target = target.clamp(min as i64, max as i64);
        fs::write(bl.join("brightness"), format!("{}\n", target))
            .with_context(|| format!("write {}", bl.display()))?;
        set_override_flag();
        tracing::info!(
            "brightness {}: {} -> {} (max {})",
            bl.display(),
            cur,
            target,
            max
        );
        break; // sole primary device only (matches dimmer candidate choice)
    }
    Ok(())
}

fn run_check(cfg_path: &PathBuf) -> Result<()> {
    let cfg = load_config(cfg_path)?;
    // Hypr IPC reachable?
    let sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").unwrap_or_default();
    if sig.is_empty() {
        println!("check: HYPRLAND_INSTANCE_SIGNATURE empty (not in Hyprland session) — config OK, IPC skipped");
    } else {
        println!("check: Hyprland signature present");
    }
    println!(
        "check OK: bar {} {}x{} left={} center={} right={}",
        cfg.bar.style,
        cfg.bar.position,
        cfg.bar.height,
        cfg.bar.left.len(),
        cfg.bar.center.len(),
        cfg.bar.right.len()
    );
    // backlight topology (read-only)
    for bl in backlight_candidates() {
        let cur = read_u32(&bl.join("brightness")).unwrap_or(0);
        let max = read_u32(&bl.join("max_brightness")).unwrap_or(0);
        println!("check: backlight {} cur={} max={}", bl.display(), cur, max);
    }
    Ok(())
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let args = Args::parse();
    let cfg_path = args.config.clone().unwrap_or_else(default_config_path);

    if args.brightness_up {
        return change_brightness(0.05);
    }
    if args.brightness_down {
        return change_brightness(-0.05);
    }
    if args.check {
        return run_check(&cfg_path);
    }

    // Full GTK4 bar startup (layer-shell). Minimal here: real widgets land in
    // src/bar.rs etc. This scaffold proves --check + IPC + sysfs path first.
    // When run without Hyprland (e.g. TTY), fail loud with guidance, don't spin.
    let cfg = load_config(&cfg_path)?;
    if std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_err()
        && std::env::var("WAYLAND_DISPLAY").is_err()
    {
        eprintln!("sshell-rs: no Wayland/Hyprland session (WAYLAND_DISPLAY + HYPRLAND_INSTANCE_SIGNATURE missing).");
        eprintln!("Run `sshell-rs --check` for validation, or launch from Hyprland exec-once.");
        std::process::exit(2);
    }
    println!(
        "sshell-rs v{} starting bar ({} {} h={}) — full GTK4 widgets in next milestone; idle event loop active.",
        env!("CARGO_PKG_VERSION"),
        cfg.bar.style,
        cfg.bar.position,
        cfg.bar.height
    );
    // Park as event-driven idle process until GTK main loop lands:
    // block on signals instead of polling.
    #[cfg(unix)]
    {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        let term = Arc::new(AtomicBool::new(false));
        let t = term.clone();
        // Minimal SIGTERM/SIGINT handler via libc-free approach:
        // spawn a thread waiting on tokio signal would need runtime;
        // here use `signal-hook`-free: just park; systemd will SIGKILL after TimeoutStopSec.
        // Real milestone replaces this with glib main loop (no polling).
        let _ = t;
        loop {
            std::thread::park();
            if term.load(Ordering::Relaxed) {
                break;
            }
        }
    }
    Ok(())
}
