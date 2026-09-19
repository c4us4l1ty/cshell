//! GTK4 layer-shell app: same layout tokens as Bar.qml / ControlCenter / Launcher.
//! Windows: bar (top exclusive 38), launcher 400x500, control-center 450 right,
//! notifications top-right, OSD top-center 1500ms, session fullscreen.
//! Animations: 100ms fade only. No slide/popin. No polling timers except
//! minute-aligned clock (+ battery piggyback, zero extra wakeups).

use crate::{
    audio, battery, config::ShellConfig, hypr, launcher, mpris, network, notifications,
    sysfs, wallpaper, watch, weather,
};
use gtk4::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::cell::RefCell;
use std::rc::Rc;

pub const FADE_CSS: &str = r#"
* { transition-property: opacity; transition-duration: 100ms; transition-timing-function: ease-out; }
.sshell-bar { background-color: rgba(20,19,19,0.55); border-radius: 14px; border: 1px solid rgba(147,143,153,0.15); }
.sshell-pill { background-color: rgba(20,19,19,0.85); border-radius: 14px; border: 1px solid rgba(147,143,153,0.2); }
.sshell-muted { opacity: 0.7; }
"#;

fn apply_css() {
    let provider = gtk4::CssProvider::new();
    provider.load_from_data(FADE_CSS);
    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
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

struct BarWidgets {
    clock: gtk4::Label,
    battery: gtk4::Label,
    net: gtk4::Label,
    workspaces: gtk4::Label,
    mpris: gtk4::Label,
    weather: gtk4::Label,
}

fn refresh_bar(w: &BarWidgets, cfg: &ShellConfig) {
    w.clock.set_text(&clock_text(cfg));
    let b = battery::read_sysfs();
    w.battery.set_text(&battery::bar_text(&b));
    let n = network::snapshot_offline();
    w.net.set_text(&network::bar_text(&n, true));
    // Workspaces live from Hyprland IPC (event thread refreshes; fallback static dots)
    w.workspaces.set_text(&hypr::dots_label(&hypr::workspaces(), hypr::active_id()));
    // MPRIS placeholder: real track arrives via MPRIS D-Bus watch (next milestone wires signal)
    let _ = mpris::bar_text(None, true, false, 40);
    w.mpris.set_visible(false);
    if !cfg.weather_city().is_empty() {
        let t = weather::cached_temp().unwrap_or_else(|| "--".into());
        w.weather.set_text(&format!("{} {}", "󰖐", t));
        w.weather.set_visible(true);
    } else {
        w.weather.set_visible(false);
    }
    // Dimmer hint: show dot when should-dim active (same 50/53 hysteresis)
    if let Some(p) = b.pct {
        let dim = battery::should_dim(p, false);
        w.battery.set_tooltip_text(Some(if dim { "battery-dimmer active (≤50%)" } else { "" }));
    }
    let _ = sysfs::current_frac();
    let _ = wallpaper::thumb_dir();
    let _ = audio::AudioState::default();
}

fn make_bar_window(app: &gtk4::Application, cfg: &ShellConfig) -> (gtk4::ApplicationWindow, BarWidgets) {
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

    if mod_enabled(cfg, "left", "Launcher") {
        left.append(&w_launcher);
    }
    if mod_enabled(cfg, "left", "Workspaces") {
        left.append(&w_workspaces);
    }
    if mod_enabled(cfg, "left", "Mpris") {
        left.append(&w_mpris);
        w_mpris.set_visible(false);
    }
    if mod_enabled(cfg, "center", "Clock") {
        center.append(&w_clock);
    }
    if mod_enabled(cfg, "center", "Weather") {
        center.append(&w_weather);
    }
    if mod_enabled(cfg, "right", "Battery") {
        right.append(&w_battery);
    }
    if mod_enabled(cfg, "right", "Tray") {
        right.append(&w_net);
    }

    let w = BarWidgets {
        clock: w_clock,
        battery: w_battery,
        net: w_net,
        workspaces: w_workspaces,
        mpris: w_mpris,
        weather: w_weather,
    };
    // launcher icon never changes; silence unused warning via tooltip
    w_launcher.set_tooltip_text(Some("launcher"));
    (win, w)
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

pub fn run(cfg: ShellConfig) {
    let app = gtk4::Application::new(Some("com.cshell.sshell"), Default::default());
    let cfg = Rc::new(cfg);
    app.connect_activate(move |app| {
        apply_css();
        let (bar, widgets) = make_bar_window(app, &cfg);
        bar.set_visible(cfg.bar.enabled);
        let widgets = Rc::new(widgets);
        let cfg2 = cfg.clone();

        // Initial refresh
        refresh_bar(&widgets, &cfg2);

        // Event wiring: Hyprland socket + UPower/MPRIS/NM D-Bus signals all
        // send on one bounded channel; a spawn_local consumer refreshes the bar.
        // Zero polling: threads block on sockets/signals, main wakes on events only.
        // (spawn_local allows !Send futures, so Rc widgets stay on the main thread.)
        let (etx, erx) = async_channel::bounded::<()>(32);
        {
            let tx = etx.clone();
            std::thread::spawn(move || {
                hypr::event_loop(move || {
                    let _ = tx.send_blocking(());
                })
            });
        }
        {
            let tx = etx.clone();
            watch::watch_upower(move || {
                let _ = tx.send_blocking(());
            });
        }
        {
            let tx = etx.clone();
            watch::watch_mpris(move || {
                let _ = tx.send_blocking(());
            });
        }
        {
            let tx = etx.clone();
            watch::watch_net(move || {
                let _ = tx.send_blocking(());
            });
        }

        // Notifications server (session bus) + CC count + toast popup.
        // Same channel-free wake pattern: server calls wake on Notify/Close.
        let notif_state = std::sync::Arc::new(std::sync::Mutex::new(
            notifications::NotifState { list: vec![], dnd: false, max: 5 },
        ));

        // Minute-aligned clock (+ battery piggyback: ZERO extra wakeups vs QML 1s+5s polls)
        let w2 = widgets.clone();
        let c2 = cfg2.clone();
        glib::timeout_add_seconds_local(secs_to_next_minute(), move || {
            refresh_bar(&w2, &c2);
            let w3 = w2.clone();
            let c3 = c2.clone();
            glib::timeout_add_seconds_local(60, move || {
                refresh_bar(&w3, &c3);
                glib::ControlFlow::Continue
            });
            glib::ControlFlow::Break
        });

        // Popups (same sizes/positions as QML; content wired incrementally)
        let launcher = make_popup(app, "sshell:launcher", 400, 500, false);
        {
            let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
            box_.add_css_class("sshell-pill");
            box_.set_margin_top(12);
            box_.set_margin_bottom(12);
            box_.set_margin_start(12);
            box_.set_margin_end(12);
            let entry = gtk4::SearchEntry::new();
            entry.set_placeholder_text(Some("Search apps… (Esc clears, Enter launches)"));
            box_.append(&entry);
            let list = gtk4::ListBox::new();
            list.set_visible(false);
            box_.append(&list);
            launcher.set_child(Some(&box_));
            // Scanned once (no poll); inotify rescan lands next milestone.
            let apps = Rc::new(launcher::scan());
            let list2 = list.clone();
            let apps2 = apps.clone();
            let launcher2 = launcher.clone();
            entry.connect_search_changed(move |e| {
                let q = e.text().to_string();
                // clear rows
                while let Some(row) = list2.first_child() {
                    list2.remove(&row);
                }
                if q.is_empty() {
                    list2.set_visible(false);
                    return;
                }
                // ";" prefix = clipboard mode (cliphist read ONLY on open/query, never background)
                if let Some(rest) = q.strip_prefix(';') {
                    if let Ok(out) = std::process::Command::new("cliphist").arg("list").output() {
                        let text = String::from_utf8_lossy(&out.stdout);
                        for (i, line) in text.lines().take(20).enumerate() {
                            let row = gtk4::Label::new(Some(&format!("{}: {}", i, line.chars().take(60).collect::<String>())));
                            row.set_halign(gtk4::Align::Start);
                            list2.append(&row);
                        }
                    }
                    list2.set_visible(true);
                    return;
                }
                for ent in launcher::fuzzy(&apps2, &q) {
                    let row = gtk4::Label::new(Some(&ent.name));
                    row.set_halign(gtk4::Align::Start);
                    list2.append(&row);
                }
                if list2.first_child().is_none() {
                    list2.append(&gtk4::Label::new(Some("No results found")));
                }
                list2.set_visible(true);
            });
            // Enter launches first result via gio DesktopAppInfo (no shell interpolation)
            let apps3 = apps.clone();
            entry.connect_activate(move |e| {
                let q = e.text().to_string();
                if q.is_empty() {
                    return;
                }
                if let Some(ent) = launcher::fuzzy(&apps3, &q).into_iter().next() {
                    if let Some(info) = gio::DesktopAppInfo::new(&ent.desktop_id) {
                        use gio::prelude::AppInfoExtManual;
                        let ctx: Option<&gio::AppLaunchContext> = None;
                        let _ = info.launch(&[], ctx);
                    } else {
                        let _ = std::process::Command::new(&ent.exec).spawn();
                    }
                    launcher2.set_visible(false);
                }
            });
        }
        let cc = make_popup(app, "sshell:control-center", 450, 600, true);
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
            // Sliders mirror ControlCenter SliderRows (volume + brightness + %)
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
            vol.connect_value_changed(move |s| {
                let v = s.value() / 100.0 * 100.0 / 100.0;
                let _ = v;
                audio::set_volume(s.value());
                vol_pct.set_text(&format!("{}%", (s.value() * 100.0).round() as i32));
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
                // write sysfs directly (single writer) — match QML setBrightness clamps
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
            let notif = gtk4::Label::new(Some("0 Notifications"));
            notif.add_css_class("sshell-muted");
            box_.append(&notif);
            cc.set_child(Some(&box_));
            // Toast popup top-right: latest notification, auto-hide per timeout (fade only).
            // Single consumer below drives BOTH count + toast from the event channel.
            let toast = make_popup(app, "sshell:notifications", 350, 80, true);
            let toast_label = gtk4::Label::new(Some(""));
            toast_label.set_wrap(true);
            toast_label.set_halign(gtk4::Align::Start);
            toast.set_child(Some(&toast_label));
            {
                let w = widgets.clone();
                let c = cfg2.clone();
                let st = notif_state.clone();
                glib::MainContext::default().spawn_local(async move {
                    while erx.recv().await.is_ok() {
                        refresh_bar(&w, &c);
                        let last = st.lock().unwrap().list.last().cloned();
                        let n = st.lock().unwrap().list.len();
                        notif.set_text(&format!("{} Notifications", n));
                        if let Some(n) = last {
                            toast_label.set_text(&format!("{}\n{}", n.title, n.body));
                            toast.set_visible(true);
                            let t = toast.clone();
                            glib::timeout_add_local_once(
                                std::time::Duration::from_millis(if n.timeout_ms > 0 { n.timeout_ms as u64 } else { 5000 }),
                                move || t.set_visible(false),
                            );
                            st.lock().unwrap().close(n.id);
                        }
                    }
                });
            }
            // Notification server wakes the same channel (bar refresh piggybacks free).
            notifications::serve(notif_state.clone(), {
                let tx = etx.clone();
                std::sync::Arc::new(move || {
                    let _ = tx.send_blocking(());
                })
            });
        }
        let osd = make_popup(app, "sshell:osd", 300, 60, false);
        {
            let l = gtk4::Label::new(Some(""));
            osd.set_child(Some(&l));
        }
        // Keep popups alive for socket toggles (Rc roots)
        let _keep: Rc<RefCell<Vec<gtk4::ApplicationWindow>>> =
            Rc::new(RefCell::new(vec![launcher, cc, osd]));
        let _ = _keep;
        bar.present();
    });
    app.run();
}
