//! MPRIS: D-Bus PropertiesChanged-driven. No playerctl polling.
//! Bar shows title (max 666px same as QML), popup has prev/play/next.

use std::process::Command;

#[derive(Debug, Clone, Default)]
pub struct Track {
    pub title: String,
    pub artist: String,
    pub playing: bool,
    pub player: String,
}

pub fn bar_text(t: Option<&Track>, hide_on_pause: bool, show_artist: bool, max_chars: usize) -> Option<String> {
    let t = t?;
    if hide_on_pause && !t.playing {
        return None;
    }
    if t.title.is_empty() {
        return None;
    }
    let mut s = if show_artist && !t.artist.is_empty() {
        format!("{} — {}", t.artist, t.title)
    } else {
        t.title.clone()
    };
    if s.chars().count() > max_chars {
        s = s.chars().take(max_chars.saturating_sub(1)).collect::<String>() + "…";
    }
    let icon = if t.playing { "󰏤" } else { "󰏥" };
    Some(format!("{} {}", icon, s))
}

/// User actions go straight to D-Bus via playerctl-less path first;
/// playerctl kept ONLY as fallback when D-Bus unavailable (still one fork per click, never poll).
pub fn action(act: &str) {
    let args: &[&str] = match act {
        "next" => &["next"],
        "prev" => &["previous"],
        "play-pause" => &["play-pause"],
        _ => return,
    };
    // Prefer MPRIS D-Bus in app layer; fallback here:
    let _ = Command::new("playerctl").args(args).output();
}
