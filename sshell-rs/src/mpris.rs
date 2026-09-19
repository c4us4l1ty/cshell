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
    if mpris_dbus(act) {
        return;
    }
    let args: &[&str] = match act {
        "next" => &["next"],
        "prev" => &["previous"],
        "play-pause" => &["play-pause"],
        _ => return,
    };
    // Prefer MPRIS D-Bus in app layer; fallback here:
    let _ = Command::new("playerctl").args(args).output();
}

/// Best-effort MPRIS method call over session bus (first available player).
/// Returns true if any player acked. Synchronous, called on user action only.
fn mpris_dbus(act: &str) -> bool {
    let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(_) => return false,
    };
    rt.block_on(async {
        let conn = match zbus::Connection::session().await {
            Ok(c) => c,
            Err(_) => return false,
        };
        let names = list_players(&conn).await;
        for name in names {
            let member = match act {
                "next" => "Next",
                "prev" => "Previous",
                "play-pause" => "PlayPause",
                _ => return false,
            };
            let msg = match zbus::Message::method("/org/mpris/MediaPlayer2", member) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let msg = match msg
                .destination(name.as_str())
                .and_then(|b| b.interface("org.mpris.MediaPlayer2.Player"))
                .and_then(|b| b.build(&()))
            {
                Ok(m) => m,
                Err(_) => continue,
            };
            if conn.send(&msg).await.is_ok() {
                return true;
            }
        }
        false
    })
}

async fn list_players(conn: &zbus::Connection) -> Vec<zbus::names::OwnedBusName> {
    let proxy = match zbus::fdo::DBusProxy::new(conn).await {
        Ok(p) => p,
        Err(_) => return vec![],
    };
    let names = proxy.list_names().await.unwrap_or_default();
    names.into_iter().filter(|n| n.as_str().starts_with("org.mpris.MediaPlayer2.")).collect()
}

/// Current track for bar/popup. Blocking D-Bus read, called on mpris wake or popup open only.
pub fn current() -> Option<Track> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().ok()?;
    rt.block_on(async {
        let conn = zbus::Connection::session().await.ok()?;
        for name in list_players(&conn).await {
            let proxy = zbus::Proxy::new(&conn, name.clone(), "/org/mpris/MediaPlayer2", "org.mpris.MediaPlayer2.Player").await.ok()?;
            let playing: bool = proxy.get_property::<String>("PlaybackStatus").await.ok().map(|s| s == "Playing").unwrap_or(false);
            let meta: std::collections::HashMap<String, zbus::zvariant::OwnedValue> =
                proxy.get_property("Metadata").await.unwrap_or_default();
            let title = meta
                .get("xesam:title")
                .and_then(|v| {
                    let owned = (*v).try_clone().ok()?;
                    let val: zbus::zvariant::Value<'_> = owned.into();
                    String::try_from(&val).ok()
                })
                .unwrap_or_default();
            if title.is_empty() {
                continue;
            }
            let artist = meta
                .get("xesam:artist")
                .and_then(|v| {
                    let owned = (*v).try_clone().ok()?;
                    let val: zbus::zvariant::Value<'_> = owned.into();
                    if let zbus::zvariant::Value::Array(arr) = &val {
                        arr.iter().filter_map(|e| String::try_from(e).ok()).next()
                    } else {
                        None
                    }
                })
                .unwrap_or_default();
            return Some(Track { title, artist, playing, player: name.to_string() });
        }
        None
    })
}
