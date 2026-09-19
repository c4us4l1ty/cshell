//! sshell-rs — Rust + GTK4 replacement for shell.qml
//! Same UI tokens. Event-driven. No polling. Single backlight writer.
//! CLI doubles as single-handler for Hypr keybinds (no double-step).

mod app;
mod audio;
mod battery;
mod config;
mod launcher;
mod mpris;
mod network;
mod sysfs;
mod wallpaper;
mod weather;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "sshell-rs", version, about = "cshell Rust GTK4 shell")]
struct Args {
    #[command(subcommand)]
    cmd: Option<Cmd>,
    /// Validate config + hardware without opening windows (TTY pre-launch)
    #[arg(long, global = true)]
    check: bool,
    /// Custom config path
    #[arg(long, global = true)]
    config: Option<std::path::PathBuf>,
}

#[derive(Subcommand, Debug, Clone)]
enum Cmd {
    /// Increase brightness 5% (sysfs sole-writer + override flag + OSD once)
    BrightnessUp,
    /// Decrease brightness 5%
    BrightnessDown,
    /// Volume up 5% (one wpctl call)
    VolumeUp,
    /// Volume down 5%
    VolumeDown,
    /// Toggle mute (one wpctl call)
    Mute,
    /// MPRIS next/prev/play-pause (D-Bus first, playerctl fallback)
    Mpris { action: String },
    /// Session: lock|logout|suspend|reboot|poweroff via logind/systemctl (no QML Process per button)
    Session { action: String },
}

fn run_check(cfg_path: &std::path::PathBuf) -> Result<()> {
    let cfg = config::load_config(cfg_path)?;
    let sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").unwrap_or_default();
    if sig.is_empty() {
        println!("check: HYPRLAND_INSTANCE_SIGNATURE empty (not in Hyprland) — config OK, IPC skipped");
    } else {
        println!("check: Hyprland signature present");
    }
    println!(
        "check OK: bar {} {} h={} left={} center={} right={} cc_w={} osd={}ms city='{}'",
        cfg.bar.style,
        cfg.bar.position,
        cfg.bar.height,
        cfg.bar.left.len(),
        cfg.bar.center.len(),
        cfg.bar.right.len(),
        cfg.control_center_width(),
        cfg.osd_timeout_ms(),
        cfg.weather_city(),
    );
    for line in sysfs::topology_lines() {
        println!("check: {}", line);
    }
    let b = battery::read_sysfs();
    println!(
        "check: battery {} plugged={} charging={}",
        b.pct.map(|p| format!("{}%", p)).unwrap_or("--%".into()),
        b.plugged,
        b.charging
    );
    let apps = launcher::scan();
    println!("check: apps {} (.desktop)", apps.len());
    Ok(())
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let args = Args::parse();
    let cfg_path = args.config.clone().unwrap_or_else(config::default_config_path);

    if args.check {
        return run_check(&cfg_path);
    }

    if let Some(cmd) = args.cmd {
        match cmd {
            Cmd::BrightnessUp => return sysfs::change(sysfs::STEP_FRAC),
            Cmd::BrightnessDown => return sysfs::change(-sysfs::STEP_FRAC),
            Cmd::VolumeUp => {
                let st = audio::refresh();
                audio::set_volume(st.volume01 + 0.05);
                return Ok(());
            }
            Cmd::VolumeDown => {
                let st = audio::refresh();
                audio::set_volume(st.volume01 - 0.05);
                return Ok(());
            }
            Cmd::Mute => {
                audio::toggle_mute();
                return Ok(());
            }
            Cmd::Mpris { action } => {
                mpris::action(&action);
                return Ok(());
            }
            Cmd::Session { action } => {
                use std::process::Command;
                match action.as_str() {
                    "suspend" => {
                        let _ = Command::new("systemctl").arg("suspend").spawn();
                    }
                    "reboot" => {
                        let _ = Command::new("systemctl").arg("reboot").spawn();
                    }
                    "poweroff" => {
                        let _ = Command::new("systemctl").arg("poweroff").spawn();
                    }
                    "logout" => {
                        let user = std::env::var("USER").unwrap_or_default();
                        let _ = Command::new("loginctl").args(["terminate-user", &user]).spawn();
                    }
                    "lock" => {
                        let _ = Command::new("loginctl").arg("lock-session").spawn();
                    }
                    _ => anyhow::bail!("session action must be lock|logout|suspend|reboot|poweroff"),
                }
                return Ok(());
            }
        }
    }

    let cfg = config::load_config(&cfg_path)?;
    if std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_err()
        && std::env::var("WAYLAND_DISPLAY").is_err()
    {
        eprintln!("sshell-rs: no Wayland/Hyprland session.");
        eprintln!("Run `sshell-rs --check` for validation, or launch from Hyprland exec-once.");
        std::process::exit(2);
    }
    app::run(cfg);
    Ok(())
}
