//! org.freedesktop.Notifications server (session bus).
//! Same behavior as QML NotificationService: max 5, groupAt 3, timeout 5000ms,
//! top-right popups + ControlCenter list + DND flag.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use zbus::zvariant::Value;

#[derive(Debug, Clone)]
pub struct Notif {
    pub id: u32,
    pub app: String,
    pub title: String,
    pub body: String,
    /// Icon name or path (shown in CC list rows next milestone; kept for spec parity).
    #[allow(dead_code)]
    pub icon: String,
    pub timeout_ms: i32,
}

#[derive(Debug, Clone, Default)]
pub struct NotifState {
    pub list: Vec<Notif>,
    pub dnd: bool,
    pub max: usize,
}

impl NotifState {
    pub fn push(&mut self, n: Notif) {
        if self.list.len() >= self.max.max(1) {
            self.list.remove(0);
        }
        self.list.push(n);
    }
    pub fn close(&mut self, id: u32) {
        self.list.retain(|n| n.id != id);
    }
}

pub struct Server {
    pub state: Arc<Mutex<NotifState>>,
    pub changed: Arc<dyn Fn() + Send + Sync>,
    next_id: Mutex<u32>,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Server {
    // 8 params are the freedesktop spec signature — not splittable.
    #[allow(clippy::too_many_arguments)]
    async fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        _actions: Vec<String>,
        _hints: HashMap<String, Value<'_>>,
        expire_timeout: i32,
    ) -> u32 {
        let id = if replaces_id != 0 {
            replaces_id
        } else {
            let mut n = self.next_id.lock().unwrap();
            *n = n.wrapping_add(1).max(1);
            *n
        };
        {
            let mut st = self.state.lock().unwrap();
            st.list.retain(|x| x.id != id);
            st.push(Notif { id, app: app_name, title: summary, body, icon: app_icon, timeout_ms: expire_timeout });
        }
        (self.changed)();
        id
    }

    async fn close_notification(&self, id: u32) {
        self.state.lock().unwrap().close(id);
        (self.changed)();
    }

    async fn get_capabilities(&self) -> Vec<String> {
        vec!["body".into(), "actions".into(), "persistence".into()]
    }

    async fn get_server_information(&self) -> (String, String, String, String) {
        ("cshell".into(), "cshell".into(), env!("CARGO_PKG_VERSION").into(), "1.2".into())
    }
}

/// Serve on session bus; best-effort (TTY without bus => no-op).
pub fn serve(state: Arc<Mutex<NotifState>>, changed: Arc<dyn Fn() + Send + Sync>) {
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
            let srv = Server { state, changed, next_id: Mutex::new(0) };
            if conn.object_server().at("/org/freedesktop/Notifications", srv).await.is_err() {
                return;
            }
            if conn.request_name("org.freedesktop.Notifications").await.is_err() {
                return; // another server owns it; bar still shows via ... (documented)
            }
            std::future::pending::<()>().await;
        });
    });
}
