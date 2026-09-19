//! StatusNotifier (system tray) host: watcher + item tracking + Activate.
//! QML Tray parity without icon rendering: popup lists item titles, click
//! activates (e.g. nm-applet, blueman). Best-effort: if another host owns the
//! watcher name, we back off silently. Item icons (pixmap decode + menus) are
//! intentionally out of scope — text rows cost zero wakeups.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Default)]
pub struct TrayState {
    /// service name -> last-known title
    pub items: HashMap<String, String>,
}

pub struct Watcher {
    pub state: Arc<Mutex<TrayState>>,
    pub changed: Arc<dyn Fn() + Send + Sync>,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    async fn register_status_notifier_item(&self, service: String) {
        {
            let mut st = self.state.lock().unwrap();
            st.items.entry(service).or_default();
        }
        (self.changed)();
    }

    async fn unregister_status_notifier_item(&self, service: String) {
        {
            self.state.lock().unwrap().items.remove(&service);
        }
        (self.changed)();
    }

    async fn register_status_notifier_host(&self, _host: String) {}

    #[zbus(property)]
    async fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    async fn registered_status_notifier_items(&self) -> Vec<String> {
        self.state.lock().unwrap().items.keys().cloned().collect()
    }

    #[zbus(property)]
    async fn protocol_version(&self) -> i32 {
        0
    }
}

/// Serve watcher on session bus; backs off if name taken (another host).
pub fn serve(state: Arc<Mutex<TrayState>>, changed: Arc<dyn Fn() + Send + Sync>) {
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
            let w = Watcher { state, changed };
            if conn
                .object_server()
                .at("/StatusNotifierWatcher", w)
                .await
                .is_err()
            {
                return;
            }
            if conn
                .request_name("org.kde.StatusNotifierWatcher")
                .await
                .is_err()
            {
                return; // another host owns it — back off silently
            }
            std::future::pending::<()>().await;
        });
    });
}

/// Best-effort title for an item service (D-Bus Title property read, user-action/wake only).
pub fn item_title(service: &str) -> Option<String> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().ok()?;
    rt.block_on(async {
        let conn = zbus::Connection::session().await.ok()?;
        // item object path is usually /StatusNotifierItem; derive from service via Get
        let proxy = zbus::Proxy::new(
            &conn,
            service,
            "/StatusNotifierItem",
            "org.kde.StatusNotifierItem",
        )
        .await
        .ok()?;
        proxy.get_property::<String>("Title").await.ok()
    })
}

/// Activate an item (left-click parity). Fire-and-forget on user click only.
pub fn activate(service: &str) {
    let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(_) => return,
    };
    rt.block_on(async {
        let conn = match zbus::Connection::session().await {
            Ok(c) => c,
            Err(_) => return,
        };
        let msg = match zbus::Message::method("/StatusNotifierItem", "Activate")
            .and_then(|b| b.destination(service))
            .and_then(|b| b.interface("org.kde.StatusNotifierItem"))
            .and_then(|b| b.build(&(0i32, 0i32)))
        {
            Ok(m) => m,
            Err(_) => return,
        };
        let _ = conn.send(&msg).await;
    });
}
