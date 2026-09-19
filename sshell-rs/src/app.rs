//! GTK4 layer-shell app: same layout tokens as Bar.qml / ControlCenter / Launcher /
//! SessionScreen / WallpaperSelector / Settings / OSD / NotificationPopups.
//! Windows: bar (top exclusive 38), launcher 400x500, control-center 450 right,
//! notifications toast 350x80 top-right, OSD 300x60 top-center 1500ms,
//! session fullscreen overlay, wallpaper 600x500 grid, settings 700x550,
//! module popups (mpris 400x165, battery, clock calendar).
//! Animations: 100ms fade only. Events: async-channel bus fed by Hyprland
//! socket + D-Bus signals + control socket + inotify (zero polling; the only
//! timer is the minute-aligned clock with battery piggyback).

use crate::{
    audio, battery, config::{self, ShellConfig}, hypr, ipc, launcher, mpris, network,
    notifications, sysfs, theme, tray, wallpaper, watch, weather,
};
use gtk4::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

pub const FADE_CSS: &str = r#"
* { transition-property: opacity; transition-duration: 100ms; transition-timing-function: ease-out; }
.sshell-bar { background-color: rgba(20,19,19,0.55); border-radius: 14px; border: 1px solid rgba(147,143,153,0.15); }
.sshell-pill { background-color: rgba(20,19,19,0.85); border-radius: 14px; border: 1px solid rgba(147,143,153,0.2); }
.sshell-muted { opacity: 0.7; }
"#;

type Cfg = Rc<RefCell<ShellConfig>>;

fn apply_css() -> gtk4::CssProvider {
    let provider = gtk4::CssProvider::new();
    theme::reload_into(&provider, FADE_CSS);
    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
    provider
}

fn mod_enabled(cfg: &ShellConfig, side: &str, name: &str) -> bool {
    let list = match side {
        "left" => &cfg.bar.left,
        "center" => &cfg.bar.center,
        "right" => &cfg.bar.right,
        _ => return false,
    };
    list.iter().any(|m| m.module == name && m.enabled)
}

fn clock_text(cfg: &ShellConfig) -> String {
    let now = chrono::Local::now();
    let t = if cfg.clock_format_24() {
        now.format("%H:%M").to_string()
    } else {
        now.format("%-I:%M %p").to_string()
    };
    if cfg.clock_show_date() {
        format!("{}  {}", t, now.format("%a %b %-d"))
    } else {
        t
    }
}

/// Seconds until next minute boundary (clock aligns, not 1Hz redraw).
fn secs_to_next_minute() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let s = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    (60 - (s % 60)).max(1) as u32
}

/// Fill launcher results (apps fuzzy + `;` clipboard mode). Pure helper so the
/// popup closures stay flat. cliphist runs ONLY on explicit `;` query (user action).
fn fill_results(list: &gtk4::ListBox, apps: &[launcher::Entry], q: &str) {
    while let Some(row) = list.first_child() {
        list.remove(&row);
    }
    if q.is_empty() {
        list.set_visible(false);
        return;
    }
    if q.strip_prefix(';').is_some() {
        if let Ok(out) = std::process::Command::new("cliphist").arg("list").output() {
            let text = String::from_utf8_lossy(&out.stdout);
            for (i, line) in text.lines().take(20).enumerate() {
                let row = gtk4::Label::new(Some(&format!(
                    "{}: {}",
                    i,
                    line.chars().take(60).collect::<String>()
                )));
                row.set_halign(gtk4::Align::Start);
                list.append(&row);
            }
        }
        list.set_visible(true);
        return;
    }
    for ent in launcher::fuzzy(apps, q) {
        let row = gtk4::Label::new(Some(&ent.name));
        row.set_halign(gtk4::Align::Start);
        row.set_tooltip_text(Some(&format!("{}\nicon: {}", ent.exec, ent.icon)));
        list.append(&row);
    }
    if list.first_child().is_none() {
        list.append(&gtk4::Label::new(Some("No results found")));
    }
    list.set_visible(true);
}

fn uptime_hm() -> String {
    let t = std::fs::read_to_string("/proc/uptime").unwrap_or_default();
    let secs: f64 = t.split_whitespace().next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
    format!("Uptime: {}h {:02}m", (secs as u64) / 3600, ((secs as u64) % 3600) / 60)
}

struct BarWidgets {
    win: gtk4::ApplicationWindow,
    launcher: gtk4::Label,
    clock: gtk4::Label,
    battery: gtk4::Label,
    net: gtk4::Label,
    workspaces: gtk4::Label,
    mpris: gtk4::Label,
    weather: gtk4::Label,
}

/// Cheap fast path: clock + workspaces only (<10ms, no D-Bus). Called on every wake.
fn refresh_fast(w: &BarWidgets, cfg: &ShellConfig) {
    w.win.set_visible(cfg.bar.enabled);
    w.clock.set_text(&clock_text(cfg));
    w.clock.set_visible(mod_enabled(cfg, "center", "Clock"));
    w.workspaces.set_text(&hypr::dots_label(&hypr::workspaces(), hypr::active_id()));
    w.workspaces.set_visible(mod_enabled(cfg, "left", "Workspaces"));
    w.launcher.set_visible(mod_enabled(cfg, "left", "Launcher"));
}

/// Heavy slow path: battery/net/mpris/weather (D-Bus + sysfs). Debounced 300ms.
fn refresh_slow(w: &BarWidgets, cfg: &ShellConfig) {
    let b = battery::read_sysfs();
    w.battery.set_text(&battery::bar_text(&b));
    w.battery.set_visible(mod_enabled(cfg, "right", "Battery"));
    let n = network::snapshot_offline();
    w.net.set_text(&network::bar_text(&n, true));
    w.net.set_visible(mod_enabled(cfg, "right", "Tray"));
    // MPRIS live read on refresh (wake-driven, never polled)
    if mod_enabled(cfg, "left", "Mpris") {
        let track = mpris::current();
        let hide_pause = true;
        match mpris::bar_text(track.as_ref(), hide_pause, false, 40) {
            Some(t) => {
                w.mpris.set_text(&t);
                w.mpris.set_visible(true);
            }
            None => w.mpris.set_visible(false),
        }
    } else {
        w.mpris.set_visible(false);
    }
    if mod_enabled(cfg, "center", "Weather") && !cfg.weather_city().is_empty() {
        // 6h cache honored (same interval role as QML weather.interval): stale shows placeholder
        if weather::cached_valid(std::time::Duration::from_secs(6 * 3600)) {
            if let Some(t) = weather::cached_temp() {
                w.weather.set_text(&format!("{} {}", "󰖐", t));
            } else {
                w.weather.set_text("󰖐 --");
            }
        } else {
            w.weather.set_text("󰖐 --");
        }
        w.weather.set_visible(true);
    } else {
        w.weather.set_visible(false);
    }
    if let Some(p) = b.pct {
        let dim = battery::should_dim(p, false);
        w.battery.set_tooltip_text(Some(if dim { "battery-dimmer active (≤50%)" } else { "" }));
    }
}

fn refresh_bar(w: &BarWidgets, cfg: &ShellConfig) {
    refresh_fast(w, cfg);
    refresh_slow(w, cfg);
}

fn file_mtime(p: &std::path::Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(p).ok().and_then(|m| m.modified().ok())
}

fn make_bar_window(app: &gtk4::Application, cfg: &ShellConfig) -> BarWidgets {
    let win = gtk4::ApplicationWindow::new(app);
    win.set_decorated(false);
    win.set_resizable(false);
    win.init_layer_shell();
    win.set_layer(Layer::Top);
    win.set_namespace("sshell:bar");
    win.set_anchor(Edge::Top, true);
    win.set_anchor(Edge::Left, true);
    win.set_anchor(Edge::Right, true);
    win.set_exclusive_zone(cfg.bar.height);
    win.set_margin(Edge::Top, cfg.bar.margin);
    win.set_margin(Edge::Left, cfg.bar.margin);
    win.set_margin(Edge::Right, cfg.bar.margin);
    win.set_keyboard_mode(KeyboardMode::None);

    let root = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    root.add_css_class("sshell-bar");
    root.set_margin_start(8);
    root.set_margin_end(8);
    win.set_child(Some(&root));

    let left = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let center = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    center.set_hexpand(true);
    center.set_halign(gtk4::Align::Center);
    let right = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    right.set_halign(gtk4::Align::End);
    root.append(&left);
    root.append(&center);
    root.append(&right);

    let mk = |t: &str| {
        let l = gtk4::Label::new(Some(t));
        l.set_yalign(0.5);
        l
    };
    let w_launcher = mk("󰀻");
    let w_workspaces = mk("● ○ ○ ○ ○");
    let w_mpris = mk("");
    let w_clock = mk("--:--");
    let w_weather = mk("");
    let w_battery = mk("--%");
    let w_net = mk("wifi");
    w_launcher.set_tooltip_text(Some("launcher"));

    left.append(&w_launcher);
    left.append(&w_workspaces);
    left.append(&w_mpris);
    center.append(&w_clock);
    center.append(&w_weather);
    right.append(&w_battery);
    right.append(&w_net);

    BarWidgets { win, launcher: w_launcher, clock: w_clock, battery: w_battery, net: w_net, workspaces: w_workspaces, mpris: w_mpris, weather: w_weather }
}

/// Overlay popup: same sizes as QML (launcher 400x500, CC 450 right, OSD center).
fn make_popup(app: &gtk4::Application, ns: &str, w: i32, h: i32, anchor_right: bool) -> gtk4::ApplicationWindow {
    let win = gtk4::ApplicationWindow::new(app);
    win.set_decorated(false);
    win.set_resizable(false);
    win.set_default_size(w, h);
    win.init_layer_shell();
    win.set_layer(Layer::Overlay);
    win.set_namespace(ns);
    win.set_anchor(Edge::Top, true);
    win.set_anchor(Edge::Bottom, false);
    if anchor_right {
        win.set_anchor(Edge::Right, true);
        win.set_margin(Edge::Right, 10);
    } else {
        win.set_anchor(Edge::Left, false);
        win.set_anchor(Edge::Right, false);
    }
    win.set_keyboard_mode(KeyboardMode::Exclusive);
    win.set_visible(false);
    win
}

/// Small anchored popup for bar modules (mpris 400x165 same as QML).
fn make_module_popup(app: &gtk4::Application, ns: &str, w: i32, h: i32) -> gtk4::ApplicationWindow {
    let win = make_popup(app, ns, w, h, false);
    win.set_anchor(Edge::Top, true);
    win
}

fn session_do(action: &str) {
    use std::process::Command;
    match action {
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
        _ => {}
    }
}

fn save_config(cfg: &ShellConfig, path: &std::path::Path) {
    // Atomic tmp+rename. Note: pretty JSON drops // comments (documented in UI footer).
    if let Ok(text) = serde_json::to_string_pretty(cfg) {
        let tmp = path.with_extension("tmp.jsonc");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

pub fn run(cfg: ShellConfig) {
    let app = gtk4::Application::new(Some("com.cshell.sshell"), Default::default());
    let cfg: Cfg = Rc::new(RefCell::new(cfg));
    let cfg_path = config::default_config_path();
    app.connect_activate(move |app| {
        let css_provider = apply_css();
        let widgets = Rc::new(make_bar_window(app, &cfg.borrow()));
        widgets.win.set_visible(cfg.borrow().bar.enabled);
        refresh_bar(&widgets, &cfg.borrow());

        // Event bus: Hyprland socket + D-Bus signals + control socket + inotify
        // all send ipc::Event; ONE spawn_local consumer applies them.
        let (etx, erx) = async_channel::bounded::<ipc::Event>(64);
        {
            let tx = etx.clone();
            std::thread::spawn(move || {
                hypr::event_loop(move || {
                    let _ = tx.send_blocking(ipc::Event::Refresh);
                })
            });
        }
        for spawn in [watch::watch_upower as fn(_) -> (), watch::watch_mpris, watch::watch_net] {
            let tx = etx.clone();
            spawn(move || {
                let _ = tx.send_blocking(ipc::Event::Refresh);
            });
        }
        {
            let tx = etx.clone();
            std::thread::spawn(move || ipc::serve(move |e| {
                let _ = tx.send_blocking(e);
            }));
        }
        // Config hot-reload via inotify (debounced 150ms, same as QML retry timer).
        {
            let tx = etx.clone();
            let path = cfg_path.clone();
            std::thread::spawn(move || {
                use notify::{RecursiveMode, Watcher};
                let (ntx, nrx) = std::sync::mpsc::channel();
                let mut watcher = match notify::RecommendedWatcher::new(ntx, notify::Config::default()) {
                    Ok(w) => w,
                    Err(_) => return,
                };
                let parent = path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| std::path::PathBuf::from("."));
                if watcher.watch(&parent, RecursiveMode::NonRecursive).is_err() {
                    return;
                }
                let wanted = path.file_name().map(|n| n.to_owned());
                let mut last = std::time::Instant::now() - Duration::from_secs(10);
                for ev in nrx.into_iter().flatten() {
                    let hit = ev.paths.iter().any(|p| {
                        p == &path
                            || p.file_name()
                                .map(|f| wanted.as_ref().map(|n| n.as_os_str() == f).unwrap_or(false))
                                .unwrap_or(false)
                    });
                    if hit && last.elapsed() > Duration::from_millis(150) {
                        last = std::time::Instant::now();
                        let _ = tx.send_blocking(ipc::Event::Refresh);
                    }
                }
            });
        }

        // Notifications server state (session bus). Toast + CC count ride the
        // event bus below (server wake -> Refresh -> UI read). Zero polling.
        let notif_state = std::sync::Arc::new(std::sync::Mutex::new(
            notifications::NotifState { list: vec![], dnd: false, max: 5 },
        ));
        // Tray (StatusNotifier) state: watcher host + item rows on demand.
        let tray_state = std::sync::Arc::new(std::sync::Mutex::new(tray::TrayState::default()));

        // Minute-aligned clock (+ battery piggyback): the ONLY timer in the app.
        // Everything else is socket/signal/inotify-driven. First tick aligns to
        // the next minute boundary (same as QML Clock intent, without 1Hz redraw).
        {
            let tx = etx.clone();
            glib::timeout_add_seconds_local(secs_to_next_minute(), move || {
                let _ = tx.send_blocking(ipc::Event::Refresh);
                let tx2 = tx.clone();
                glib::timeout_add_seconds_local(60, move || {
                    let _ = tx2.send_blocking(ipc::Event::Refresh);
                    glib::ControlFlow::Continue
                });
                glib::ControlFlow::Break
            });
        }

        // ---- Popups ----
        // Launcher 400x500
        let launcher_win = make_popup(app, "sshell:launcher", 400, 500, false);
        let launcher_entry = gtk4::SearchEntry::new();
        {
            let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
            box_.add_css_class("sshell-pill");
            box_.set_margin_top(12);
            box_.set_margin_bottom(12);
            box_.set_margin_start(12);
            box_.set_margin_end(12);
            launcher_entry.set_placeholder_text(Some("Search apps… (Esc clears, Enter launches)"));
            box_.append(&launcher_entry);
            let list = gtk4::ListBox::new();
            list.set_visible(false);
            box_.append(&list);
            launcher_win.set_child(Some(&box_));
            let apps = Rc::new(launcher::scan());
            let launcher2 = launcher_win.clone();
            let apps_fill = apps.clone();
            let list_s = list.clone();
            launcher_entry.connect_search_changed(move |e| {
                fill_results(&list_s, &apps_fill, &e.text());
            });
            let apps3 = apps.clone();
            launcher_entry.connect_activate(move |e| {
                let q = e.text().to_string();
                if q.is_empty() {
                    return;
                }
                if let Some(ent) = launcher::fuzzy(&apps3, &q).into_iter().next() {
                    if let Some(info) = gio::DesktopAppInfo::new(&ent.desktop_id) {
                        let ctx: Option<&gio::AppLaunchContext> = None;
                        let _ = info.launch(&[], ctx);
                    } else {
                        let _ = std::process::Command::new(&ent.exec).spawn();
                    }
                    launcher2.set_visible(false);
                }
            });
            // Esc clears, then closes (same as QML AppLauncher Keys.onEscapePressed)
            let lw = launcher_win.clone();
            let entry_esc = launcher_entry.clone();
            let esc_list = list.clone();
            let esc_apps = apps.clone();
            let ec = gtk4::EventControllerKey::new();
            ec.connect_key_pressed(move |_, key, _, _| {
                if key == gtk4::gdk::Key::Escape {
                    if !entry_esc.text().is_empty() {
                        entry_esc.set_text("");
                        fill_results(&esc_list, &esc_apps, "");
                    } else {
                        lw.set_visible(false);
                    }
                    return true.into();
                }
                false.into()
            });
            launcher_win.add_controller(ec);
        }

        // ControlCenter 450 right + sliders + notification count.
        // Detail popups are created below; placeholders let CC buttons reference them.
        let wifi_win: Rc<RefCell<Option<gtk4::ApplicationWindow>>> = Rc::new(RefCell::new(None));
        let bt_win: Rc<RefCell<Option<gtk4::ApplicationWindow>>> = Rc::new(RefCell::new(None));
        let tray_win: Rc<RefCell<Option<gtk4::ApplicationWindow>>> = Rc::new(RefCell::new(None));
        let cc = make_popup(app, "sshell:control-center", 450, 600, true);
        let cc_notif = gtk4::Label::new(Some("0 Notifications"));
        {
            let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
            box_.add_css_class("sshell-pill");
            box_.set_margin_top(16);
            box_.set_margin_bottom(16);
            box_.set_margin_start(16);
            box_.set_margin_end(16);
            let user = gtk4::Label::new(None);
            let who = std::env::var("USER").unwrap_or_else(|_| "user".into());
            let host: String = glib::host_name().to_string();
            user.set_text(&format!("{}@{}", who, host));
            user.set_halign(gtk4::Align::Start);
            box_.append(&user);
            let st = audio::refresh();
            let vol_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            let vol_icon = gtk4::Label::new(Some(if st.muted { "󰝟" } else { "󰕾" }));
            let vol = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 0.0, 1.0, 0.01);
            vol.set_value(st.volume01);
            vol.set_hexpand(true);
            let vol_pct = gtk4::Label::new(Some(&format!("{}%", (st.volume01 * 100.0).round() as i32)));
            vol_row.append(&vol_icon);
            vol_row.append(&vol);
            vol_row.append(&vol_pct);
            box_.append(&vol_row);
            let vol_pct_v = vol_pct.clone();
            vol.connect_value_changed(move |s| {
                audio::set_volume(s.value());
                vol_pct_v.set_text(&format!("{}%", (s.value() * 100.0).round() as i32));
            });
            let cur = sysfs::current_frac().unwrap_or(0.5);
            let bri_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            bri_row.append(&gtk4::Label::new(Some("󰃠")));
            let bri = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 0.0, 1.0, 0.01);
            bri.set_value(cur);
            bri.set_hexpand(true);
            let bri_pct = gtk4::Label::new(Some(&format!("{}%", (cur * 100.0).round() as i32)));
            bri_row.append(&bri);
            bri_row.append(&bri_pct);
            box_.append(&bri_row);
            bri.connect_value_changed(move |s| {
                let v = s.value().clamp(0.0, 1.0);
                if let Some(bl) = sysfs::candidates().into_iter().next() {
                    if let (Some(max), Some(_cur)) = (
                        sysfs::read_u32(&bl.join("max_brightness")),
                        sysfs::read_u32(&bl.join("brightness")),
                    ) {
                        let min = (max as f64 * sysfs::MIN_PERCENT_FLOOR) as i64;
                        let t = ((v * max as f64).round() as i64).clamp(min.max(1), max as i64);
                        let _ = std::fs::write(bl.join("brightness"), format!("{}\n", t));
                        let _ = std::fs::create_dir_all("/run/sshell");
                        let _ = std::fs::write("/run/sshell/backlight-override", b"1");
                    }
                }
                bri_pct.set_text(&format!("{}%", (s.value() * 100.0).round() as i32));
            });
            // Quick toggles row: wifi / bluetooth / mute / DND (user-action forks only)
            let quick = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            let wifi_btn = gtk4::Button::with_label("WiFi");
            let bt_btn = gtk4::Button::with_label("BT");
            let mute_btn = gtk4::Button::with_label("Mute");
            let dnd_btn = gtk4::Button::with_label("DND");
            quick.append(&wifi_btn);
            quick.append(&bt_btn);
            quick.append(&mute_btn);
            quick.append(&dnd_btn);
            box_.append(&quick);
            wifi_btn.connect_clicked(|_| {
                // Rust-backed toggle (no `sh -c` interpolation): read state, flip once.
                let cur = std::process::Command::new("nmcli")
                    .args(["radio", "wifi"])
                    .output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_default();
                let next = if cur == "enabled" { "off" } else { "on" };
                let _ = std::process::Command::new("nmcli")
                    .args(["radio", "wifi", next])
                    .output();
            });
            bt_btn.connect_clicked(|_| {
                let _ = std::process::Command::new("bluetoothctl").arg("power").arg("toggle").output();
            });
            let dnd_state = notif_state.clone();
            dnd_btn.connect_clicked(move |b| {
                let mut st = dnd_state.lock().unwrap();
                st.dnd = !st.dnd;
                b.set_label(if st.dnd { "DND on" } else { "DND" });
            });
            mute_btn.connect_clicked(move |b| {
                audio::toggle_mute();
                let st = audio::refresh();
                vol.set_value(st.volume01);
                vol_pct.set_text(&format!("{}%", (st.volume01 * 100.0).round() as i32));
                vol_icon.set_text(if st.muted { "󰝟" } else { "󰕾" });
                b.set_label(if st.muted { "Unmute" } else { "Mute" });
            });
            // Detail shortcuts (popups fill on open, never background)
            let det_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            let wifi_det = gtk4::Button::with_label("WiFi…");
            let bt_det = gtk4::Button::with_label("BT…");
            let tray_det = gtk4::Button::with_label("Tray…");
            det_row.append(&wifi_det);
            det_row.append(&bt_det);
            det_row.append(&tray_det);
            box_.append(&det_row);
            {
                let ww = wifi_win.clone();
                wifi_det.connect_clicked(move |_| {
                    if let Some(w) = ww.borrow().as_ref() {
                        w.set_visible(true);
                    }
                });
            }
            {
                let bw = bt_win.clone();
                bt_det.connect_clicked(move |_| {
                    if let Some(w) = bw.borrow().as_ref() {
                        w.set_visible(true);
                    }
                });
            }
            {
                let tw = tray_win.clone();
                tray_det.connect_clicked(move |_| {
                    if let Some(w) = tw.borrow().as_ref() {
                        w.set_visible(true);
                    }
                });
            }
            cc_notif.add_css_class("sshell-muted");
            box_.append(&cc_notif);
            cc.set_child(Some(&box_));
        }

        // Toast 350x80 top-right (driven by notification server wake, zero polling)
        let toast = make_popup(app, "sshell:notifications", 350, 80, true);
        let toast_label = gtk4::Label::new(Some(""));
        toast_label.set_wrap(true);
        toast_label.set_halign(gtk4::Align::Start);
        toast.set_child(Some(&toast_label));

        // OSD 300x60 top-center (brightness/volume CLI shows via socket)
        let osd = make_popup(app, "sshell:osd", 300, 60, false);
        let osd_label = gtk4::Label::new(Some(""));
        osd.set_child(Some(&osd_label));
        let osd_gen = Rc::new(Cell::new(0u64));

        // Module popups
        let mpris_popup = make_module_popup(app, "sshell:mpris-popup", 400, 165);
        let mpris_label = gtk4::Label::new(Some("No player"));
        mpris_label.set_wrap(true);
        {
            let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
            box_.add_css_class("sshell-pill");
            box_.set_margin_top(12);
            box_.set_margin_bottom(12);
            box_.set_margin_start(12);
            box_.set_margin_end(12);
            box_.append(&mpris_label);
            let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            row.set_halign(gtk4::Align::Center);
            for (label, act) in [("⏮", "prev"), ("⏯", "play-pause"), ("⏭", "next")] {
                let b = gtk4::Button::with_label(label);
                b.connect_clicked(move |_| mpris::action(act));
                row.append(&b);
            }
            box_.append(&row);
            mpris_popup.set_child(Some(&box_));
        }
        let battery_popup = make_module_popup(app, "sshell:battery-popup", 300, 200);
        let battery_label = gtk4::Label::new(Some(""));
        battery_label.set_halign(gtk4::Align::Start);
        battery_popup.set_child(Some(&battery_label));
        let clock_popup = make_module_popup(app, "sshell:clock-popup", 300, 300);
        {
            let cal = gtk4::Calendar::new();
            clock_popup.set_child(Some(&cal));
        }
        // Weather popup (same forecast role as QML WeatherPopup): cached details
        // + manual Refresh (single curl per click, 6h cache honored).
        let weather_popup = make_module_popup(app, "sshell:weather-popup", 320, 220);
        let weather_label = gtk4::Label::new(Some(""));
        weather_label.set_wrap(true);
        weather_label.set_halign(gtk4::Align::Start);
        {
            let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
            box_.add_css_class("sshell-pill");
            box_.set_margin_top(12);
            box_.set_margin_bottom(12);
            box_.set_margin_start(12);
            box_.set_margin_end(12);
            box_.append(&weather_label);
            let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            row.set_halign(gtk4::Align::Center);
            let refresh = gtk4::Button::with_label("Refresh");
            row.append(&refresh);
            box_.append(&row);
            weather_popup.set_child(Some(&box_));
            let wl = weather_label.clone();
            let wc = cfg.clone();
            refresh.connect_clicked(move |b| {
                b.set_label("…");
                let city = wc.borrow().weather_city();
                match weather::fetch_now(&city) {
                    Ok(s) => wl.set_text(&s),
                    Err(e) => wl.set_text(&format!("Refresh failed: {}", e)),
                }
                b.set_label("Refresh");
                if let Some(t) = weather::cached_temp() {
                    // bar picks it up on next refresh; force one via label is enough here
                    let _ = t;
                }
            });
        }
        // WiFi detail popup (list on open only — no background scans)
        let wifi_popup = make_module_popup(app, "sshell:wifi-popup", 380, 320);
        let wifi_label = gtk4::Label::new(Some(""));
        wifi_label.set_halign(gtk4::Align::Start);
        {
            let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
            box_.add_css_class("sshell-pill");
            box_.set_margin_top(12);
            box_.set_margin_bottom(12);
            box_.set_margin_start(12);
            box_.set_margin_end(12);
            box_.append(&wifi_label);
            let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            row.set_halign(gtk4::Align::Center);
            let rescan = gtk4::Button::with_label("Rescan");
            row.append(&rescan);
            box_.append(&row);
            wifi_popup.set_child(Some(&box_));
            *wifi_win.borrow_mut() = Some(wifi_popup.clone());
            // Fill on every open (user action) — Rescan button re-fills with 30s cooldown.
            let wl_show = wifi_label.clone();
            wifi_popup.connect_show(move |_| {
                wl_show.set_text(&network::wifi_list());
            });
            let wl = wifi_label.clone();
            rescan.connect_clicked(move |b| {
                b.set_sensitive(false);
                wl.set_text(&network::wifi_list());
                // 30s cooldown (same guard as QML WifiDetail)
                let b2 = b.clone();
                glib::timeout_add_local_once(Duration::from_secs(30), move || b2.set_sensitive(true));
            });
        }
        // Bluetooth detail popup (list on open only)
        let bt_popup = make_module_popup(app, "sshell:bluetooth-popup", 380, 300);
        let bt_label = gtk4::Label::new(Some(""));
        bt_label.set_halign(gtk4::Align::Start);
        {
            let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
            box_.add_css_class("sshell-pill");
            box_.set_margin_top(12);
            box_.set_margin_bottom(12);
            box_.set_margin_start(12);
            box_.set_margin_end(12);
            box_.append(&bt_label);
            bt_popup.set_child(Some(&box_));
            *bt_win.borrow_mut() = Some(bt_popup.clone());
            let bl_show = bt_label.clone();
            bt_popup.connect_show(move |_| {
                bl_show.set_text(&network::bt_list());
            });
        }

        // Tray popup (StatusNotifier items: title rows, click activates).
        // Filled on open + on watcher wake while visible. Icons intentionally
        // text-only (pixmap decode would cost wakeups for zero function).
        let tray_popup = make_module_popup(app, "sshell:tray-popup", 380, 300);
        let tray_list = gtk4::ListBox::new();
        {
            let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
            box_.add_css_class("sshell-pill");
            box_.set_margin_top(12);
            box_.set_margin_bottom(12);
            box_.set_margin_start(12);
            box_.set_margin_end(12);
            let head = gtk4::Label::new(Some("Tray"));
            head.add_css_class("sshell-muted");
            box_.append(&head);
            box_.append(&tray_list);
            tray_popup.set_child(Some(&box_));
            let fill_rc: Rc<dyn Fn()> = Rc::new({
                let st = tray_state.clone();
                move || {
                    while let Some(row) = tray_list.first_child() {
                        tray_list.remove(&row);
                    }
                    let items: Vec<(String, String)> = {
                        let s = st.lock().unwrap();
                        let mut v: Vec<_> = s.items.iter().map(|(k, t)| (k.clone(), t.clone())).collect();
                        v.sort();
                        v
                    };
                    if items.is_empty() {
                        tray_list.append(&gtk4::Label::new(Some("No tray items")));
                    }
                    for (svc, title) in items {
                        let label = if title.is_empty() { svc.clone() } else { format!("{} ({})", title, svc) };
                        let row = gtk4::Button::with_label(&label);
                        row.connect_clicked(move |_| tray::activate(&svc));
                        tray_list.append(&row);
                    }
                }
            });
            let fill_show = fill_rc.clone();
            tray_popup.connect_show(move |_| fill_show());
            *tray_win.borrow_mut() = Some(tray_popup.clone());
            // resolve titles lazily on open (one D-Bus read per item, user action)
            let st2 = tray_state.clone();
            let fill2 = fill_rc.clone();
            tray_popup.connect_show(move |_| {
                let svcs: Vec<String> = st2.lock().unwrap().items.keys().cloned().collect();
                for svc in svcs {
                    if st2.lock().unwrap().items.get(&svc).map(|t| t.is_empty()).unwrap_or(false) {
                        if let Some(title) = tray::item_title(&svc) {
                            st2.lock().unwrap().items.insert(svc, title);
                        }
                    }
                }
                fill2();
            });
        }
        {
            let mp = mpris_popup.clone();
            let ml = mpris_label.clone();
            let g = gtk4::GestureClick::new();
            g.connect_pressed(move |_, _, _, _| {
                if let Some(t) = mpris::current() {
                    ml.set_text(&format!("{} [{}]\n{} — {}", if t.playing { "Playing" } else { "Paused" }, t.player, t.artist, t.title));
                } else {
                    ml.set_text("No player");
                }
                mp.set_visible(!mp.is_visible());
            });
            widgets.mpris.add_controller(g);
        }
        {
            let bp = battery_popup.clone();
            let bl = battery_label.clone();
            let g = gtk4::GestureClick::new();
            g.connect_pressed(move |_, _, _, _| {
                let d = battery::details();
                bl.set_text(&format!(
                    "Battery: {}\nStatus: {}\nHealth: {}\nRate: {}\nTime: {}",
                    d.pct.map(|p| format!("{}%", p)).unwrap_or("--%".into()),
                    if d.status.is_empty() { "-" } else { &d.status },
                    d.health_pct.map(|h| format!("{:.1}%", h)).unwrap_or("-".into()),
                    d.rate_w.map(|r| format!("{:.1}W", r)).unwrap_or("-".into()),
                    d.time_hm.as_deref().unwrap_or("-"),
                ));
                bp.set_visible(!bp.is_visible());
            });
            widgets.battery.add_controller(g);
        }
        {
            let wp = wifi_popup.clone();
            let g = gtk4::GestureClick::new();
            g.connect_pressed(move |_, _, _, _| {
                wp.set_visible(!wp.is_visible());
            });
            widgets.net.add_controller(g);
        }
        {
            let cp = clock_popup.clone();
            let g = gtk4::GestureClick::new();
            g.connect_pressed(move |_, _, _, _| cp.set_visible(!cp.is_visible()));
            widgets.clock.add_controller(g);
        }
        {
            let wp = weather_popup.clone();
            let wl = weather_label.clone();
            let wc = cfg.clone();
            let g = gtk4::GestureClick::new();
            g.connect_pressed(move |_, _, _, _| {
                let city = wc.borrow().weather_city();
                if city.is_empty() {
                    wl.set_text("Weather disabled (empty city in settings)");
                } else {
                    wl.set_text(&weather::cached_summary(&city));
                }
                wp.set_visible(!wp.is_visible());
            });
            widgets.weather.add_controller(g);
        }

        // Session fullscreen overlay (same 4 buttons + uptime + hint as QML)
        let session = gtk4::ApplicationWindow::new(app);
        let session_uptime = gtk4::Label::new(Some(&uptime_hm()));
        {
            session.set_decorated(false);
            session.init_layer_shell();
            session.set_layer(Layer::Overlay);
            session.set_namespace("sshell:session");
            session.set_anchor(Edge::Top, true);
            session.set_anchor(Edge::Bottom, true);
            session.set_anchor(Edge::Left, true);
            session.set_anchor(Edge::Right, true);
            session.set_keyboard_mode(KeyboardMode::Exclusive);
            session.set_visible(false);
            let bg = gtk4::Box::new(gtk4::Orientation::Vertical, 30);
            bg.set_halign(gtk4::Align::Center);
            bg.set_valign(gtk4::Align::Center);
            let title = gtk4::Label::new(Some("Session"));
            session_uptime.add_css_class("sshell-muted");
            let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 20);
            row.set_halign(gtk4::Align::Center);
            for (label, act) in [("Shutdown", "poweroff"), ("Reboot", "reboot"), ("Suspend", "suspend"), ("Log Out", "logout")] {
                let b = gtk4::Button::with_label(label);
                b.connect_clicked(move |_| session_do(act));
                row.append(&b);
            }
            let hint = gtk4::Label::new(Some("Esc to cancel"));
            hint.add_css_class("sshell-muted");
            bg.append(&title);
            bg.append(&session_uptime);
            bg.append(&row);
            bg.append(&hint);
            session.set_child(Some(&bg));
            let sw = session.clone();
            let ec = gtk4::EventControllerKey::new();
            ec.connect_key_pressed(move |_, key, _, _| {
                if key == gtk4::gdk::Key::Escape {
                    sw.set_visible(false);
                    return true.into();
                }
                false.into()
            });
            session.add_controller(ec);
        }

        // Wallpaper selector 600x500 grid (256px thumbs via image crate, no convert fork)
        let wp_win = make_popup(app, "sshell:wallpaper", 600, 500, false);
        {
            let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
            box_.add_css_class("sshell-pill");
            box_.set_margin_top(12);
            box_.set_margin_bottom(12);
            box_.set_margin_start(12);
            box_.set_margin_end(12);
            let status = gtk4::Label::new(Some("Wallpapers"));
            status.add_css_class("sshell-muted");
            box_.append(&status);
            let scroll = gtk4::ScrolledWindow::new();
            scroll.set_vexpand(true);
            scroll.set_hexpand(true);
            let flow = gtk4::FlowBox::new();
            flow.set_selection_mode(gtk4::SelectionMode::Single);
            flow.set_max_children_per_line(4);
            scroll.set_child(Some(&flow));
            box_.append(&scroll);
            // Random button (same as QML randomWallpaper, minus current-repeat)
            let rand_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            rand_row.set_halign(gtk4::Align::Center);
            let rand_btn = gtk4::Button::with_label("Random");
            rand_row.append(&rand_btn);
            box_.append(&rand_row);
            wp_win.set_child(Some(&box_));
            let scanned: Rc<RefCell<Vec<std::path::PathBuf>>> = Rc::new(RefCell::new(vec![]));
            {
                let scanned = scanned.clone();
                let ww = wp_win.clone();
                let status_r = status.clone();
                rand_btn.connect_clicked(move |_| {
                    let list = scanned.borrow();
                    if list.is_empty() {
                        return;
                    }
                    // time-free pick without rand crate: /dev/urandom byte, no fork
                    let seed = std::fs::read("/dev/urandom").map(|v| v[0] as usize).unwrap_or(0);
                    let pick = &list[seed % list.len()];
                    let msg = wallpaper::apply(pick).unwrap_or_else(|e| e);
                    status_r.set_text(&msg);
                    ww.set_visible(false);
                });
            }
            let ww = wp_win.clone();
            let status_act = status.clone();
            flow.connect_child_activated(move |_, child| {
                if let Some(path) = child.widget_name().as_str().strip_prefix("wp:") {
                    let p = std::path::PathBuf::from(path);
                    let msg = wallpaper::apply(&p).unwrap_or_else(|e| e);
                    status_act.set_text(&msg);
                    ww.set_visible(false);
                }
            });
            // populate on open: instant placeholders + chunked thumbs (8/idle, no 12s freeze)
            let f2 = flow.clone();
            let s2 = status.clone();
            let scanned2 = scanned.clone();
            wp_win.connect_show(move |_| {
                while let Some(ch) = f2.first_child() {
                    f2.remove(&ch);
                }
                let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
                let paths = vec![format!("{}/Pictures/wallpapers", home), format!("{}/Pictures/gifs", home)];
                let list = wallpaper::scan(&paths);
                s2.set_text(&format!("{} wallpapers (loading…)", list.len()));
                let items: Vec<std::path::PathBuf> = list.iter().take(40).cloned().collect();
                *scanned2.borrow_mut() = items.clone();
                s2.set_text(&format!("{} wallpapers", list.len()));
                // placeholders first (instant open <100ms), thumbs fill 8 per idle tick
                let placeholders: Vec<gtk4::FlowBoxChild> = items.iter().map(|pp| {
                    let child = gtk4::FlowBoxChild::new();
                    child.set_widget_name(&format!("wp:{}", pp.display()));
                    let lbl = gtk4::Label::new(Some("…"));
                    lbl.set_size_request(128, 128);
                    child.set_child(Some(&lbl));
                    f2.append(&child);
                    child
                }).collect();
                let idx = std::rc::Rc::new(std::cell::RefCell::new(0usize));
                let items_rc = std::rc::Rc::new(items);
                glib::idle_add_local_full(glib::Priority::LOW, move || {
                    let mut i = idx.borrow_mut();
                    for _ in 0..8 {
                        if *i >= items_rc.len() {
                            return glib::ControlFlow::Break;
                        }
                        let pp = &items_rc[*i];
                        if let Some(child) = placeholders.get(*i) {
                            // cached thumb or source (full-res fallback only if decode fails)
                            let thumb = wallpaper::ensure_thumb(pp).unwrap_or_else(|| pp.clone());
                            let pic = gtk4::Picture::for_filename(thumb.to_string_lossy().as_ref());
                            pic.set_size_request(128, 128);
                            child.set_child(Some(&pic));
                        }
                        *i += 1;
                    }
                    glib::ControlFlow::Continue
                });
            });
        }

        // Settings 700x550 notebook (Bar / Modules / Theme / Weather)
        let settings = make_popup(app, "sshell:settings", 700, 550, false);
        {
            let nb = gtk4::Notebook::new();
            settings.set_child(Some(&nb));
            // Bar page
            {
                let grid = gtk4::Grid::new();
                grid.set_row_spacing(8);
                grid.set_column_spacing(12);
                grid.set_margin_top(16);
                grid.set_margin_bottom(16);
                grid.set_margin_start(16);
                grid.set_margin_end(16);
                let h_label = gtk4::Label::new(Some("Height (24-64)"));
                h_label.set_halign(gtk4::Align::Start);
                let h_spin = gtk4::SpinButton::with_range(24.0, 64.0, 1.0);
                h_spin.set_value(cfg.borrow().bar.height as f64);
                grid.attach(&h_label, 0, 0, 1, 1);
                grid.attach(&h_spin, 1, 0, 1, 1);
                let styles = ["floating", "full", "islands", "modules"];
                let style_items = gtk4::StringList::new(&styles);
                let style_drop = gtk4::DropDown::new(Some(style_items), gtk4::Expression::NONE);
                {
                    let cur = cfg.borrow().bar.style.clone();
                    if let Some(pos) = styles.iter().position(|s| *s == cur) {
                        style_drop.set_selected(pos as u32);
                    }
                }
                let s_label = gtk4::Label::new(Some("Style"));
                s_label.set_halign(gtk4::Align::Start);
                grid.attach(&s_label, 0, 1, 1, 1);
                grid.attach(&style_drop, 1, 1, 1, 1);
                nb.append_page(&grid, Some(&gtk4::Label::new(Some("Bar"))));
                let c = cfg.clone();
                let cp = cfg_path.clone();
                let save = gtk4::Button::with_label("Apply Bar");
                grid.attach(&save, 0, 2, 2, 1);
                save.connect_clicked(move |_| {
                    c.borrow_mut().bar.height = h_spin.value() as i32;
                    let sel = style_drop.selected() as usize;
                    if sel < styles.len() {
                        c.borrow_mut().bar.style = styles[sel].to_string();
                    }
                    save_config(&c.borrow(), &cp);
                });
            }
            // Modules page
            {
                let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
                box_.set_margin_top(16);
                box_.set_margin_start(16);
                let checks: Rc<RefCell<Vec<(String, String, gtk4::CheckButton)>>> = Rc::new(RefCell::new(vec![]));
                for (side, name) in [
                    ("left", "Launcher"), ("left", "Workspaces"), ("left", "Mpris"),
                    ("center", "Clock"), ("center", "Weather"),
                    ("right", "Battery"), ("right", "Tray"),
                ] {
                    let cb = gtk4::CheckButton::with_label(&format!("{}: {}", side, name));
                    cb.set_active(mod_enabled(&cfg.borrow(), side, name));
                    box_.append(&cb);
                    checks.borrow_mut().push((side.into(), name.into(), cb));
                }
                nb.append_page(&box_, Some(&gtk4::Label::new(Some("Modules"))));
                let c = cfg.clone();
                let cp = cfg_path.clone();
                let apply = gtk4::Button::with_label("Apply Modules");
                box_.append(&apply);
                apply.connect_clicked(move |_| {
                    let mut b = c.borrow_mut();
                    for (side, name, cb) in checks.borrow().iter() {
                        let list = match side.as_str() {
                            "left" => &mut b.bar.left,
                            "center" => &mut b.bar.center,
                            _ => &mut b.bar.right,
                        };
                        if let Some(m) = list.iter_mut().find(|m| m.module == *name) {
                            m.enabled = cb.is_active();
                        }
                    }
                    save_config(&b, &cp);
                });
            }
            // Theme page
            {
                let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
                box_.set_margin_top(16);
                box_.set_margin_start(16);
                let dark = gtk4::CheckButton::with_label("Dark mode");
                dark.set_active(true);
                box_.append(&dark);
                let note = gtk4::Label::new(Some("Colors come from matugen on wallpaper change."));
                note.add_css_class("sshell-muted");
                box_.append(&note);
                nb.append_page(&box_, Some(&gtk4::Label::new(Some("Theme"))));
            }
            // Weather page
            {
                let grid = gtk4::Grid::new();
                grid.set_row_spacing(8);
                grid.set_column_spacing(12);
                grid.set_margin_top(16);
                grid.set_margin_start(16);
                let city = gtk4::Entry::new();
                city.set_placeholder_text(Some("City (empty = disabled)"));
                city.set_text(&cfg.borrow().weather_city());
                grid.attach(&gtk4::Label::new(Some("City")), 0, 0, 1, 1);
                grid.attach(&city, 1, 0, 1, 1);
                nb.append_page(&grid, Some(&gtk4::Label::new(Some("Weather"))));
                let c = cfg.clone();
                let cp = cfg_path.clone();
                let apply = gtk4::Button::with_label("Apply Weather");
                grid.attach(&apply, 0, 1, 2, 1);
                apply.connect_clicked(move |_| {
                    c.borrow_mut().weather["city"] = serde_json::Value::String(city.text().to_string());
                    save_config(&c.borrow(), &cp);
                });
                let foot = gtk4::Label::new(Some("Saved as JSON (comments are dropped on save)."));
                foot.add_css_class("sshell-muted");
                grid.attach(&foot, 0, 2, 2, 1);
            }
        }

        // Toggle map for control socket (same names as `sshell-rs toggle …`)
        let wins: Rc<RefCell<HashMap<String, gtk4::ApplicationWindow>>> = Rc::new(RefCell::new(HashMap::new()));
        wins.borrow_mut().insert("launcher".into(), launcher_win.clone());
        wins.borrow_mut().insert("control-center".into(), cc.clone());
        wins.borrow_mut().insert("session".into(), session.clone());
        wins.borrow_mut().insert("settings".into(), settings.clone());
        wins.borrow_mut().insert("wallpaper".into(), wp_win.clone());
        wins.borrow_mut().insert("tray".into(), tray_popup.clone());

        // Single consumer: Refresh (fast always, slow debounced) | Toggle | Osd
        {
            let w = widgets.clone();
            let c = cfg.clone();
            let cp = cfg_path.clone();
            let ws = wins.clone();
            let osd_w = osd.clone();
            let osd_l = osd_label.clone();
            let osd_g = osd_gen.clone();
            let le = launcher_entry.clone();
            let su = session_uptime.clone();
            let ns = notif_state.clone();
            let ts = tray_state.clone();
            let cn = cc_notif.clone();
            let tl = toast_label.clone();
            let tw = toast.clone();
            let etx_w = etx.clone();
            let last_heavy = std::rc::Rc::new(std::cell::RefCell::new(std::time::Instant::now() - Duration::from_secs(10)));
            let last_cfg_mtime = std::rc::Rc::new(std::cell::RefCell::new(file_mtime(&cp)));
            let last_theme_mtime = std::rc::Rc::new(std::cell::RefCell::new(file_mtime(&theme::theme_json_path())));
            let last_weather_try = std::rc::Rc::new(std::cell::RefCell::new(std::time::Instant::now() - Duration::from_secs(36000)));
            let last_toast_id = std::rc::Rc::new(std::cell::RefCell::new(0u32));
            glib::MainContext::default().spawn_local(async move {
                while let Ok(ev) = erx.recv().await {
                    match ev {
                        ipc::Event::Refresh => {
                            // config reload only on mtime change (no reparse storm)
                            let cur_mtime = file_mtime(&cp);
                            if cur_mtime != *last_cfg_mtime.borrow() {
                                if let Ok(nc) = config::load_config(&cp) {
                                    *c.borrow_mut() = nc;
                                    *last_cfg_mtime.borrow_mut() = cur_mtime;
                                }
                            }
                            // theme reload only on mtime change (no CSS reparse storm)
                            let cur_theme = file_mtime(&theme::theme_json_path());
                            if cur_theme != *last_theme_mtime.borrow() {
                                theme::reload_into(&css_provider, FADE_CSS);
                                *last_theme_mtime.borrow_mut() = cur_theme;
                            }
                            // fast path always (clock + workspaces <10ms)
                            refresh_fast(&w, &c.borrow());
                            // notifications: keep history (do NOT consume), toast only new ids
                            let (pending, n, dnd) = {
                                let st = ns.lock().unwrap();
                                let last = st.list.last().cloned();
                                (last, st.list.len(), st.dnd)
                            };
                            // CC count + last-3 titles in tooltip (history preserved, max5)
                            cn.set_text(&format!("{} Notifications", n));
                            {
                                let titles: Vec<String> = ns.lock().unwrap().list.iter().rev().take(3)
                                    .map(|x| format!("{}: {}", x.app, x.title)).collect();
                                cn.set_tooltip_text(Some(&if titles.is_empty() { "No notifications".into() } else { titles.join("\n") }));
                            }
                            if let Some(nn) = pending {
                                let mut lid = last_toast_id.borrow_mut();
                                if nn.id != *lid {
                                    *lid = nn.id;
                                    if !dnd {
                                        tl.set_text(&format!("{} \u2014 {}\n{}", nn.app, nn.title, nn.body));
                                        tw.set_visible(true);
                                        let t = tw.clone();
                                        glib::timeout_add_local_once(
                                            Duration::from_millis(if nn.timeout_ms > 0 { nn.timeout_ms as u64 } else { 5000 }),
                                            move || t.set_visible(false),
                                        );
                                    }
                                }
                            }
                            // slow path debounced 300ms (battery/net/mpris/weather coalesced)
                            let heavy_due = last_heavy.borrow().elapsed() >= Duration::from_millis(300);
                            if heavy_due {
                                *last_heavy.borrow_mut() = std::time::Instant::now();
                                refresh_slow(&w, &c.borrow());
                                {
                                    let n = ts.lock().unwrap().items.len();
                                    w.net.set_tooltip_text(Some(&format!("{} tray items (Tray\u2026 in ControlCenter)", n)));
                                }
                                // weather auto-fetch when stale + online (threaded, never blocks UI)
                                {
                                    let city = c.borrow().weather_city();
                                    let stale = !weather::cached_valid(Duration::from_secs(6 * 3600));
                                    let online = {
                                        let s = network::snapshot_offline();
                                        s.wifi_connected || s.eth_connected
                                    };
                                    let retry_due = last_weather_try.borrow().elapsed() >= Duration::from_secs(600);
                                    if !city.is_empty() && stale && online && retry_due {
                                        *last_weather_try.borrow_mut() = std::time::Instant::now();
                                        let txw = etx_w.clone();
                                        std::thread::spawn(move || {
                                            let _ = weather::fetch_now(&city);
                                            let _ = txw.send_blocking(ipc::Event::Refresh);
                                        });
                                    }
                                }
                            }
                        }
                        ipc::Event::Toggle(name) => {
                            let map = ws.borrow();
                            if name == "clipboard" {
                                if let Some(lw) = map.get("launcher") {
                                    le.set_text(";");
                                    lw.set_visible(!lw.is_visible());
                                    le.grab_focus();
                                }
                                continue;
                            }
                            if name == "bar-visibility" {
                                let vis = w.win.is_visible();
                                w.win.set_visible(!vis);
                                continue;
                            }
                            if name == "background" {
                                wallpaper::toggle_visible();
                                continue;
                            }
                            if let Some(win) = map.get(&name) {
                                if name == "session" && !win.is_visible() {
                                    su.set_text(&uptime_hm());
                                }
                                win.set_visible(!win.is_visible());
                            }
                        }
                        ipc::Event::Osd { text, timeout_ms } => {
                            osd_l.set_text(&text);
                            osd_w.set_visible(true);
                            let g = osd_g.get().wrapping_add(1);
                            osd_g.set(g);
                            let ow = osd_w.clone();
                            let og = osd_g.clone();
                            glib::timeout_add_local_once(Duration::from_millis(timeout_ms), move || {
                                if og.get() == g {
                                    ow.set_visible(false);
                                }
                            });
                        }
                        ipc::Event::Quit => {}
                    }
                }
            });
        }

        // Notifications server wakes the same bus (bar refresh piggybacks free)
        notifications::serve(notif_state.clone(), {
            let tx = etx.clone();
            std::sync::Arc::new(move || {
                let _ = tx.send_blocking(ipc::Event::Refresh);
            })
        });
        // Tray watcher host (backs off silently if another host owns the name)
        tray::serve(tray_state.clone(), {
            let tx = etx.clone();
            std::sync::Arc::new(move || {
                let _ = tx.send_blocking(ipc::Event::Refresh);
            })
        });

        bar_present(&widgets);
    });
    app.run();
}

fn bar_present(w: &Rc<BarWidgets>) {
    w.win.present();
}
