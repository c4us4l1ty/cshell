//! Live D-Bus watchers (signal-driven, zero polling).
//! Pattern: D-Bus signal = wakeup, sysfs/D-Bus property read = data.
//! All watchers are best-effort: bus missing (TTY/--check) => silent no-op,
//! minute-tick fallback in app layer keeps bar correct.

use futures_lite::StreamExt;

/// Spawn a tokio runtime thread watching UPower device signals.
/// Calls `on_change` on every DeviceChanged / PropertiesChanged under org.freedesktop.UPower.
pub fn watch_upower(on_change: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(_) => return,
        };
        rt.block_on(async move {
            let conn = match zbus::Connection::system().await {
                Ok(c) => c,
                Err(_) => return,
            };
            // UPower emits PropertiesChanged on device paths + DeviceChanged on manager.
            let rule = match zbus::MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .sender("org.freedesktop.UPower")
            {
                Ok(b) => b.build(),
                Err(_) => return,
            };
            let mut stream = match zbus::MessageStream::for_match_rule(rule, &conn, None).await {
                Ok(s) => s,
                Err(_) => return,
            };
            while stream.next().await.is_some() {
                on_change();
            }
        });
    });
}

/// Spawn watcher for MPRIS PropertiesChanged on the session bus (any player).
pub fn watch_mpris(on_change: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(_) => return,
        };
        rt.block_on(async move {
            let conn = match zbus::Connection::session().await {
                Ok(c) => c,
                Err(_) => return,
            };
            let rule = match zbus::MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .interface("org.freedesktop.DBus.Properties")
            {
                Ok(b) => b.build(),
                Err(_) => return,
            };
            // Filter in callback by path prefix /org/mpris/MediaPlayer2
            let mut stream = match zbus::MessageStream::for_match_rule(rule, &conn, None).await {
                Ok(s) => s,
                Err(_) => return,
            };
            while let Some(msg) = stream.next().await {
                let msg = match msg {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                if let Some(path) = msg.path() {
                    if path.as_str().starts_with("/org/mpris/MediaPlayer2") {
                        on_change();
                    }
                }
            }
        });
    });
}

/// Spawn watcher for NetworkManager + BlueZ property changes (system bus).
pub fn watch_net(on_change: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(_) => return,
        };
        rt.block_on(async move {
            let conn = match zbus::Connection::system().await {
                Ok(c) => c,
                Err(_) => return,
            };
            let rule = match zbus::MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .interface("org.freedesktop.DBus.Properties")
            {
                Ok(b) => b.build(),
                Err(_) => return,
            };
            let mut stream = match zbus::MessageStream::for_match_rule(rule, &conn, None).await {
                Ok(s) => s,
                Err(_) => return,
            };
            while let Some(msg) = stream.next().await {
                let msg = match msg {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                let sender = msg.header().sender().map(|s| s.to_string()).unwrap_or_default();
                if sender.starts_with("org.freedesktop.NetworkManager") || sender.starts_with(":") && msg.path().map(|p| p.as_str().starts_with("/org/freedesktop/NetworkManager")).unwrap_or(false) {
                    on_change();
                    continue;
                }
                if sender.starts_with("org.bluez") {
                    on_change();
                }
            }
        });
    });
}
