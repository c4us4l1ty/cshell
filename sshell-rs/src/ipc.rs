//! Control socket: single-instance daemon + CLI toggles/OSD.
//! Path: /run/sshell/control.sock (fallback $XDG_RUNTIME_DIR/sshell-control.sock).
//! Protocol: one JSON line per connection: {"cmd":"toggle","window":"launcher"}.
//! Windows: launcher, control-center, session, settings, wallpaper, osd-hide.
//! OSD: {"cmd":"osd","text":"...","timeout_ms":1500}.
//! Shell commands only on user action: keybinds exec `sshell-rs toggle …`.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum Event {
    Refresh,
    Toggle(String),
    Osd { text: String, timeout_ms: u64 },
    Quit,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct Msg {
    cmd: String,
    #[serde(default)]
    window: String,
    #[serde(default)]
    text: String,
    #[serde(default = "default_timeout")]
    timeout_ms: u64,
}
fn default_timeout() -> u64 {
    1500
}

pub fn sock_path() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        let p = PathBuf::from(xdg).join("sshell-control.sock");
        if p.parent().map(|d| d.exists()).unwrap_or(false) {
            return p;
        }
    }
    PathBuf::from("/run/sshell/control.sock")
}

fn parse_event(line: &str) -> Option<Event> {
    let m: Msg = serde_json::from_str(line).ok()?;
    match m.cmd.as_str() {
        "toggle" => Some(Event::Toggle(m.window)),
        "osd" => Some(Event::Osd { text: m.text, timeout_ms: m.timeout_ms }),
        "quit" => Some(Event::Quit),
        "refresh" => Some(Event::Refresh),
        _ => None,
    }
}

fn send_line(line: &str) -> bool {
    let path = sock_path();
    let mut s = match UnixStream::connect(&path) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let _ = s.set_read_timeout(Some(std::time::Duration::from_millis(500)));
    let _ = s.set_write_timeout(Some(std::time::Duration::from_millis(500)));
    if s.write_all(line.as_bytes()).is_err() {
        return false;
    }
    let _ = s.write_all(b"\n");
    // wait for ack (daemon replies "ok")
    let mut r = BufReader::new(s);
    let mut ack = String::new();
    r.read_line(&mut ack).is_ok()
}

/// Fire-and-forget from CLI. Returns true if daemon acked.
pub fn send_toggle(window: &str) -> bool {
    send_line(&serde_json::json!({"cmd":"toggle","window":window}).to_string())
}

pub fn send_osd(text: &str, timeout_ms: u64) -> bool {
    send_line(&serde_json::json!({"cmd":"osd","text":text,"timeout_ms":timeout_ms}).to_string())
}

/// Blocking server loop for the daemon thread. Forwards Events via `emit`.
/// Stale socket from unclean exit is unlinked first. Single instance: if bind
/// fails because a live daemon holds it, returns immediately (second instance exits).
pub fn serve(emit: impl Fn(Event) + Send + 'static) {
    let path = sock_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // If something already listens, this is a second instance: do not steal.
    if UnixStream::connect(&path).is_ok() {
        return;
    }
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(_) => return,
    };
    // world-writable dir safety: restrict socket perms (owner-only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    for conn in listener.incoming() {
        let s = match conn {
            Ok(s) => s,
            Err(_) => continue,
        };
        let mut reader = BufReader::new(s);
        let mut line = String::new();
        let ev = match reader.read_line(&mut line) {
            Ok(0) => None,
            Ok(_) => parse_event(line.trim()),
            Err(_) => None,
        };
        // ack before handling so CLI never blocks on UI work
        let _ = reader.get_mut().write_all(b"ok\n");
        match ev {
            Some(Event::Quit) => break,
            Some(e) => emit(e),
            None => {}
        }
    }
    let _ = std::fs::remove_file(&path);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_toggle() {
        let e = parse_event(r#"{"cmd":"toggle","window":"launcher"}"#).unwrap();
        assert!(matches!(e, Event::Toggle(w) if w == "launcher"));
    }
    #[test]
    fn parse_osd_defaults() {
        let e = parse_event(r#"{"cmd":"osd","text":"50%"}"#).unwrap();
        assert!(matches!(e, Event::Osd { timeout_ms: 1500, .. }));
    }
    #[test]
    fn reject_garbage() {
        assert!(parse_event("hello").is_none());
        assert!(parse_event(r#"{"cmd":"rm -rf"}"#).is_none());
    }
}
