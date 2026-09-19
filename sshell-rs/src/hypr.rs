//! Hyprland IPC without extra crates: raw Unix sockets.
//! Requests: `$XDG_RUNTIME_DIR/hypr/$SIG/.socket.sock` (`j/workspaces`).
//! Events: `.socket2.sock` lines like `workspace>>3`, `focusedmon>>...`.
//! Falls back to static persistent dots when socket missing (TTY/--check safe).

use serde::Deserialize;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Workspace {
    pub id: i32,
    /// Hyprland workspace name (rules match on it; kept for parity, dots use id).
    #[allow(dead_code)]
    pub name: String,
    #[serde(default)]
    /// Origin monitor (multi-monitor follow logic; kept for parity).
    #[allow(dead_code)]
    pub monitor: String,
}

fn sock_path(name: &str) -> Option<PathBuf> {
    let sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
    if sig.is_empty() {
        return None;
    }
    let runtime = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| format!("/run/user/{}", self_uid()));
    Some(PathBuf::from(runtime).join("hypr").join(sig).join(name))
}

/// UID without libc dep: parse /proc/self/status (procfs always present on Linux).
fn self_uid() -> u32 {
    // no libc dep: parse /proc/self/status Uid line
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|t| {
            t.lines().find(|l| l.starts_with("Uid:")).and_then(|l| {
                l.split_whitespace().nth(1).and_then(|n| n.parse::<u32>().ok())
            })
        })
        .unwrap_or(1000)
}

fn request(cmd: &str) -> Option<String> {
    let path = sock_path(".socket.sock")?;
    let mut s = UnixStream::connect(path).ok()?;
    s.write_all(cmd.as_bytes()).ok()?;
    let mut out = String::new();
    s.read_to_string(&mut out).ok()?;
    Some(out)
}

/// Snapshot of workspaces (event-driven callers invoke on socket event, never on timer).
pub fn workspaces() -> Vec<Workspace> {
    let raw = match request("j/workspaces") {
        Some(r) => r,
        None => return persistent_fallback(1),
    };
    serde_json::from_str::<Vec<Workspace>>(&raw).unwrap_or_else(|_| persistent_fallback(1))
}

pub fn active_id() -> i32 {
    let raw = match request("j/activeworkspace") {
        Some(r) => r,
        None => return 1,
    };
    serde_json::from_str::<serde_json::Value>(&raw)
        .ok()
        .and_then(|v| v.get("id").and_then(|i| i.as_i64()).map(|i| i as i32))
        .unwrap_or(1)
}

fn persistent_fallback(active: i32) -> Vec<Workspace> {
    let _ = active;
    (1..=5)
        .map(|i| Workspace { id: i, name: i.to_string(), monitor: String::new() })
        .collect()
}

/// Blocking event loop: calls `on_event` for workspace/monitor changes.
/// Runs on a dedicated thread; app forwards to glib main context (no polling).
pub fn event_loop(mut on_event: impl FnMut() + Send + 'static) {
    let path = match sock_path(".socket2.sock") {
        Some(p) => p,
        None => return,
    };
    let mut s = match UnixStream::connect(path) {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut buf = [0u8; 4096];
    let mut acc = Vec::<u8>::new();
    loop {
        let n = match s.read(&mut buf) {
            Ok(0) => break, // compositor closed; listener thread exits, no spin
            Ok(n) => n,
            Err(_) => break,
        };
        acc.extend_from_slice(&buf[..n]);
        // events are newline-separated: workspace>>3, focusedmon>>, openwindow>>, etc.
        while let Some(pos) = acc.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = acc.drain(..=pos).collect();
            let text = String::from_utf8_lossy(&line);
            let ev = text.trim();
            if ev.starts_with("workspace>>")
                || ev.starts_with("focusedmon>>")
                || ev.starts_with("moveworkspace>>")
                || ev.starts_with("createworkspace>>")
                || ev.starts_with("destroyworkspace>>")
                || ev.starts_with("monitoradded>>")
                || ev.starts_with("monitorremoved>>")
                || ev.starts_with("openwindow>>")
                || ev.starts_with("closewindow>>")
            {
                // NOTE: single top bar (not per-monitor like QML Variants). Monitor
                // events re-snap workspaces label; HDMI gets no second bar by design
                // (documented single-bar limitation saves ~40MB + wakeups).
                on_event();
            }
        }
    }
}

/// Dot-style label identical to QML Workspaces style=dot (● active, ○ rest).
pub fn dots_label(list: &[Workspace], active: i32) -> String {
    let mut ids: Vec<i32> = list.iter().map(|w| w.id).collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        ids = (1..=5).collect();
    }
    let dots: String = ids
        .iter()
        .map(|id| if *id == active { '●' } else { '○' })
        .collect::<Vec<_>>()
        .into_iter()
        .collect::<String>()
        .chars()
        .collect::<Vec<_>>()
        .chunks(1)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(" ");
    format!("{}  {}", dots, active)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dots_static() {
        let ws = persistent_fallback(1);
        let s = dots_label(&ws, 3);
        assert!(s.contains('●'));
        assert!(s.ends_with("3"));
    }
    #[test]
    fn no_sig_no_panic() {
        std::env::remove_var("HYPRLAND_INSTANCE_SIGNATURE");
        assert_eq!(workspaces().len(), 5);
        assert_eq!(active_id(), 1);
    }
}
