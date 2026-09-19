# sshell Rust + GTK4 Rewrite — Fedora Plan (Same UI, Zero Polling, Max Battery)

> Target machine: Lenovo IdeaPad Slim 3 14IRH10 (i5-13420H Raptor Lake-P, Intel UHD i915, eDP-1 1920x1200@60, Intel CNVi WiFi + AX201 BT, BAT0 50Wh, 16GiB, Micron NVMe, BTRFS + ZRAM).
> Source flow: Fedora Minimal ISO -> TTY login -> `git clone <your-repo>` -> `cd <repo>` -> `chmod +x ./fedora-setup.sh` -> `sudo ./fedora-setup.sh` -> reboot -> Hyprland.
> Stack: Rust + GTK4 + gtk4-layer-shell. No QML / Quickshell. Event-driven. No polling. Shell commands only on user action.
> Battery rule: `Plan/battery-dimmer.sh` works right off the bat after install: battery <=50% -> dim ~30%, restore at >53%.
> UI rule: pixel-faithful port of current QML UI. No redesign. Minimal animations (100ms fade only).
> No VM available: therefore static checks + dry-run + mock-sysfs tests must prove FLAWLESS before bare-metal run. User is new to Hyprland: every step is copy-paste.

## UI Freeze Spec (must not change in Rust port)

This is the contract. Any Rust widget that deviates fails review.

- `config.jsonc`: bar enabled, position top, style floating (support full/floating/islands/modules), height 38, margin 10, padding 5. Left: Launcher, Workspaces, Mpris. Center: Clock, Weather. Right: Battery, Tray. Workspaces persistent 1-5, style dot, radius circle. Weather interval 3600, metric, city configurable, hideLocation true. Tray showNetworkName/showBluetoothName true. OSD enabled, 1500ms, top-center. Background wallpaperPaths ~/Pictures/wallpapers + ~/Pictures/gifs, wallpaperMode image (shader path disabled by default). Mpris barVisualizer false, popupVisualizer true, hideOnPause true, maxWidthOnBar 666, showArtist false. Clock showDate true, format 12. ControlCenter width 450, position right, showPfp false. Notifications top-right, max 5, groupAt 3, timeout 5000. Launcher 400x500. Theme icons round, darkmode true, mainFont CaskaydiaCove Nerd Font, titleFont Inter, monoFont JetBrains Mono, iconFont Material Symbols Rounded.
- `Appearance.qml` tokens: padding 2/5/8/12/16/20/24, radius 6/10/14/22, barHeight 40, barMargin 10, launcherWidth 600, searchBar 500/400x36 radius 22, resultItem 50px, notification 350x80, mprisPopup 400x165, M3 dark palette background #141313 onSurface #DEE2E6 primary #D0BCFF etc, background alpha 0.45, surface 0.9, border alpha 0.1-0.2, font sizes 5/10/12/14/16/18/20/24/48/64, animation durations 100/200/300/500 (Rust uses only 100ms fade).
- `Bar.qml`: floating rounded bar, left/center/right Rows, islands/modules variants, TrayToggle separate loader.
- `ControlCenter.qml`: overlayBackground radius 14, userRow `user@host` + pfp 32px or emoji-cat fallback, 4 action buttons edit/settings/sync/power, 4-column controlGrid cellHeight 60, edit hint text, notifications empty state `notifications_paused` 64px, grouped notifications by title, footer count + clear + DND, bottom SliderRows volume + brightness with % label, Wifi/Bluetooth/Audio detail overlay at 50% height.
- `AppLauncher.qml`: centered pill radius searchBarHeight/2+padding, SearchBar + separator + ListView max 500px, Up/Down/Enter/Esc behavior, No results found state.
- `SessionScreen.qml`: fullscreen overlay, Session title massive bold + Uptime, 4 SessionButtons Shutdown/Reboot/Suspend/Logout in row with arrow-key nav, hint footer, Esc/click-outside closes.
- `Background.qml`: layer Background, black base, PreserveAspectCrop image/gif, 200ms out + 500ms in crossfade (Rust: instant or 100ms fade only), backgroundVisible toggle.
- `Notifications/OSD/WallpaperSelector/Settings`: top-right popups, top-center OSD, wallpaper grid 400x400 thumbs (Rust: 256px via image crate), Settings pages General/Bar/Modules/Quick/System/Keybinds.

## Brutal Audit Summary (why rewrite)

- 40x `Timer{}`: ResourceMonitor 1s x3 `cat /proc/*` forks, Battery 5s `get_battery.sh` + 4x `bc`, Network 5s 2x `nmcli`, Clock 1Hz, Weather `curl wttr.in` blocking, Cava 60fps, 11x procedural sky/cloud/star timers. Result: constant wakeups, no package C10.
- Fork-per-tick: `brightnessctl`, `wpctl`, `playerctl`, `hyprctl|jq|xargs`, `convert` per thumbnail, `find+md5sum`, `hostnamectl`. `bc` missing on Minimal. `get_battery.sh` BAT0-only, µWh/µAh confusion.
- `installer.sh` Arch-only: dies without pacman, Arch font/pkg names, AUR quickshell, no dnf, blind rsync overwrite, `rm -rf $VAR` unquoted, `confirm` hangs TTY, never installs dimmer, hardcodes kitty vs foot, SDDM copy needs root.
- Security: `bash -c "${path}"` RCE in WallpaperService/Weather/Network, GlobalShortcut unauthenticated, wttr HTTP + location leak, predictable /tmp matugen file + TOCTOU.
- Power: blur 8/2-pass, 12 Hyprland animations, vrr/vfr, gaps_workspaces 50, swallow typo `allacritty`, missing PPD/PSR/deep-sleep/NVMe/iwlwifi/bt/audio powersave.
- Correctness: duplicate launcher import, `screens[0]` multi-monitor bug, 50ms focus-grab race steals keyboard, JSONC regex mangles `https://`, silent defaults on parse fail, empty `restore(){}`, duplicate md5/extensions lines.

---

## Phase 1 — Freeze and baseline (on current Fedora Cinnamon, before Minimal)
- [ ] 1.1 Snapshot system and repo
  - Do: `inxi -Fxxxz > ~/sshell-baseline.txt; dnf list installed > ~/dnf-baseline.txt; cp -a ~/Downloads/sshell-main ~/sshell-backup-$(date +%Y%m%d); cd ~/Downloads/sshell-main && git status --short && git log --oneline -5; sudo bash Plan/battery-dimmer.sh --status | tee ~/dimmer-baseline.txt`
  - Test: backup dir exists, baseline contains IdeaPad + i915 + BAT0, dimmer output shows intel_backlight + BAT0 capacity. Fail = stop.
- [*] 1.2 Verify Minimal ISO plan on paper
  - Do: download Fedora Everything netinstall/Minimal, `sha256sum -c *-CHECKSUM`, decide: keep nvme0n1p1 EFI (mount /boot/efi, DO NOT FORMAT), BTRFS /, user von + wheel, hostname von-fedora, Minimal software, sshd off.
  - Test: checksum OK, can recite EFI-stays + BTRFS-root + wheel-user. Fail = reread docs.
- [*] 1.3 Push clone-ready repo
  - Do: `git add -A; git commit -m "freeze pre-rust"; git push origin main` to `https://github.com/<you>/cshell`. Use HTTPS (TTY has no SSH keys).
  - Test: `git ls-remote https://github.com/<you>/cshell` succeeds from phone/second machine.
- [ ] 1.4 Record backlight/power numbers
  - Do: save `cat /sys/class/backlight/*/max_brightness` = 21333, `cat /sys/class/power_supply/BAT0/{capacity,status,energy_full,energy_full_design}` = 70,   Discharging,   48280000,   50000000, `upower -d | head -40` = Device: /org/freedesktop/UPower/devices/battery_BAT0
    native-path:          BAT0
    vendor:               COSMX
    model:                L24X3PK2
    serial:               4253
    power supply:         yes
    updated:              Sat 19 Sep 2026 08:08:16 PM +0545 (6 seconds ago)
    has history:          yes
    has statistics:       yes
    battery
      present:             yes
      rechargeable:        yes
      state:               discharging
      warning-level:       none
      energy:              33.51 Wh
      energy-empty:        0 Wh
      energy-full:         48.28 Wh
      energy-full-design:  50 Wh
      voltage-min-design:  11.31 V
      capacity-level:      Normal
      energy-rate:         10.646 W
      voltage:             11.829 V
      charge-cycles:       534
      time to empty:       3.1 hours
      percentage:          69%
      capacity:            96.56%
      technology:          lithium-polymer
      charge-threshold-supported:    yes
      icon-name:          'battery-full-symbolic'
    History (charge):
      1789827796	69.000	discharging
    History (rate):
      1789827796	10.646	discharging
      1789827766	11.080	discharging
      1789827735	8.510	discharging
      1789827705	12.669	discharging
    History (voltage):
      1789827796	11.829	discharging
      1789827766	11.825	discharging
      1789827735	11.853	discharging.
  - Test: values noted for Phase 7 dim-math validation.
- [*] 1.5 Lock constraints
  - Do: confirm Rust+GTK4 only, same UI tokens above, animations fade-only, fedora-setup.sh idempotent + logged, dimmer defaults 50/30. (YES)
  - Test: can recite TTY one-shot command from memory.

## Phase 2 — fedora-setup.sh TTY bootstrap (one-shot, idempotent, logged)
- [ ] 2.1 Skeleton strictness
  - Do: create repo-root `fedora-setup.sh`: `set -euo pipefail`, `LOG=/var/log/sshell-setup.log` tee, `--yes` flag, `--help` no-op, root check, `dnf5||dnf` detect, `rpm -q || dnf install -y` per pkg, per-step resume file `/var/lib/sshell-setup.done`, `trap ERR` with line number, `${VAR:?}` guards, `chown $SUDO_USER` for user files.
  - Test: `shellcheck -S error fedora-setup.sh` clean, `bash -n` clean, `DRY_RUN=1 sudo ./fedora-setup.sh --yes` prints without writing, rerun is no-op.
- [ ] 2.2 Fedora 44 package stack (no Arch names)
  - Do: install: base `git curl wget rsync tar xz gcc clang pkgconf-pkg-config openssl-devel`; hypr `hyprland hyprlock hypridle hyprpaper hyprshot hyprpicker xdg-desktop-portal-hyprland xdg-desktop-portal-gtk greetd tuigreet`; gtk `gtk4 gtk4-devel gtk4-layer-shell gtk4-layer-shell-devel gobject-introspection`; net `NetworkManager-wifi iw wpa_supplicant bluez bluez-tools`; audio `pipewire pipewire-pulse wireplumber pamixer playerctl brightnessctl`; power `power-profiles-daemon powertop` (never TLP+PPD together); mesa `mesa-dri-drivers mesa-vulkan-drivers intel-media-driver libva-intel-driver`; fonts `cascadia-code-nf-fonts rsms-inter-fonts jetbrains-mono-fonts google-material-symbols-fonts google-noto-emoji-fonts`; utils `cliphist ImageMagick jq foot starship fish`; rust via `rustup-init -y --default-toolchain stable`; then `fc-cache -f; fc-match` verify.
  - Test: `rpm -q hyprland gtk4 power-profiles-daemon` ok, `hyprctl version` prints, `fc-match` resolves all 4 families, zero strings `pacman|yay|quickshell|qt6-5compat` in file.
- [ ] 2.3 Backup + deploy configs (never blind overwrite)
  - Do: backup `~/.config/hypr ~/.config/gtk-3.0 ~/.config/gtk-4.0 ~/.config/sshell` to `~/.local/state/sshell/backups/$TIMESTAMP` + `latest` symlink; deploy rewritten `configs/hypr/hyprland/*` + gtk css + `config.jsonc`; `mkdir -p ~/.local/state/sshell/wallpaper ~/.cache/sshell/thumbnails`; `restorecon -Rv ~/.config`.
  - Test: dry-run shows would-backup lines, restore round-trips, no root-owned files in $HOME.
- [ ] 2.4 Dimmer on-by-default (50% -> -30%)
  - Do: `install -m 0755 Plan/battery-dimmer.sh /usr/local/bin/battery-dimmer.sh`; write `/etc/battery-dimmer.env` with THRESHOLD=50 HYSTERESIS=3 DIM_BY_PERCENT=30 MIN_PERCENT=12 BASE_POLL_INTERVAL=60; ensure unit has `EnvironmentFile=/etc/battery-dimmer.env` + `StandardOutput=journal`; `battery-dimmer.sh --install-udev; battery-dimmer.sh --install-service; systemctl enable --now battery-dimmer power-profiles-daemon NetworkManager bluetooth`.
  - Test: `systemctl is-active battery-dimmer` active, `--status` shows topology, `journalctl -u battery-dimmer` shows Threshold 50% Dim 30%, unplug at 49% dims within 60s, AC restores.
- [ ] 2.5 Boot target + kernel powersave + first smoke
  - Do: `systemctl set-default graphical.target; systemctl enable greetd` with tuigreet -> Hyprland for von; `grubby --update-kernel=ALL --args="mem_sleep_default=deep i915.enable_psr=1 i915.enable_fbc=1 nvme_core.default_ps_max_latency_us=5500"`; powertop autotune via udev.
  - Test: reboot -> tuigreet -> Hyprland, `echo $XDG_CURRENT_DESKTOP` Hyprland, `cat /sys/power/mem_sleep` shows [deep], no `qs` processes.

## Phase 3 — Rust + GTK4 skeleton (same style tokens, ~0% idle)
- [ ] 3.1 Cargo workspace + layer-shell bar shell
  - Do: `cargo new --bin sshell-rs`; deps gtk4, gtk4-layer-shell, glib, serde/jsonc-parser, zbus, hyprland-rs, notify, tracing; single top-anchored bar height 38 margin 10 namespace `sshell:bar` exclusive-zone; CSS from matugen gtk.css using Appearance tokens; per-monitor bars via hyprland events (fix screens[0] bug).
  - Test: `cargo build --release` zero warnings, bar visible on eDP-1, HDMI hotplug follows, `top` idle <0.5%, wakeups <5/s, zero Command in hot path.
- [ ] 3.2 JSONC config same keys + live reload
  - Do: port all config.jsonc keys with serde defaults mirroring Config.qml; jsonc-parser (not regex strip); inotify debounce 150ms; atomic rename writes; fail-loud with file:line:col; corrupt file keeps last-good + notification.
  - Test: `cargo test config::` 10 cases (comments, trailing commas, https URL, missing keys), edit -> relayout <200ms no restart.
- [ ] 3.3 Clock + Workspaces event-driven
  - Do: clock timeout aligned to next minute boundary, 12/24 + showDate; workspaces subscribe hyprland event stream, persistent 1-5 dot/circle rendering.
  - Test: no wakeups between minutes, Super+1..5 updates <50ms.
- [ ] 3.4 Logging + --check for TTY debug
  - Do: tracing -> journald/user log + `~/.local/state/sshell/crash.log`, `sshell-rs --check` validates config + Hypr IPC without windows, reconnect backoff no busy loop.
  - Test: `--check` exit 0 good / 2 bad with line number; kill Hypr socket stays alive.
- [ ] 3.5 Replace execs `qs -c sshell`
  - Do: `exec-once = sshell-rs &` guarded by command -v, keep keyring, dbus-update-env, `wl-paste --watch cliphist store`, cursor with Adwaita fallback.
  - Test: fresh login shows Rust bar, `pgrep -c sshell-rs` =1, no quickshell.

## Phase 4 — Bar faithful port (exact layout, no polling)
- [ ] 4.1 Structure floating/full/islands/modules
  - Do: implement Bar.qml Rows left/center/right with Repeater from config, radius 14, background 0.45 alpha, border 0.1, padding tokens; islands pills per side, modules pill per item.
  - Test: switching style in config reproduces QML screenshots; empty side collapses.
- [ ] 4.2 Modules Clock/Weather/Workspaces/Launcher/Mpris/Battery/Tray + popups
  - Do: port each module + popups MprisPopup/ClockPopup/BatteryPopup/TrayPopup/WeatherPopup same widths (mpris 400x165 etc), same show/hide rules (hideOnPause, maxWidth 666, showArtist false, hideLocation).
  - Test: visual diff vs QML screenshots passes; popups open/close with 100ms fade only.
- [ ] 4.3 OSD top-center + notifications top-right
  - Do: OSD overlay 1500ms fade-only with volume/brightness slider; notifications max 5 groupAt 3 timeout 5000 width 350x80.
  - Test: volume key shows OSD once, hides 1.5s; `notify-send` stacks correctly.
- [ ] 4.4 Fonts/icons/theme same
  - Do: enforce Caskaydia/Inter/JetBrains/Material Symbols Round, M3 dark palette, icons round; matugen css watch via inotify 100ms debounce (not poll).
  - Test: no tofu, `fc-match` all families, theme change applies <2s.
- [ ] 4.5 Performance gate
  - Do: remove all bar Timers except minute-clock + OSD-hide; verify no execDetached in bar.
  - Test: `strace -e execve` idle 60s shows 0 execs; RSS <100MB.

## Phase 5 — Launcher ControlCenter Session Settings Wallpaper (same UX)
- [ ] 5.1 Launcher + Clipboard
  - Do: overlay 400x500 pill, Entry + ListView fuzzy in-memory, desktop scan once + inotify, gio launch, Esc clears-then-closes, Up/Down/Enter nav, clipboard reads cliphist only on Super+V open.
  - Test: cold open <200ms, 300 apps search <100ms, double Super+Space never steals keys, 10k clipboard rows <300ms virtualized.
- [ ] 5.2 ControlCenter same sections
  - Do: 450px right-anchored, userRow + 4 buttons, 4-col grid 60px cells + editMode add/resize, notifications grouped + DND + clear, SliderRows volume/brightness + % label, Wifi/Bluetooth/Audio details at 50% height.
  - Test: every toggle/slider matches QML behavior; editMode hint text exact.
- [ ] 5.3 Session + Settings + WallpaperSelector
  - Do: session overlay with 4 buttons + uptime + arrow nav + Esc; settings pages General/Bar/Modules/Quick/System/Keybinds writing JSONC atomically; wallpaper grid 256px thumbs via image crate (not convert), setWallpaper triggers single matugen.
  - Test: suspend/reboot/logout via logind work; settings persist reboot; same wallpaper twice = 0 matugen reruns.
- [ ] 5.4 Background same
  - Do: layer Background black base PreserveAspectCrop image/gif, visible toggle, instant/100ms fade (not 200+500 QML sequence); shader mode behind flag default off.
  - Test: wallpaper change no jank, GIF plays, toggle hides instantly.
- [ ] 5.5 Animations minimal
  - Do: delete ProceduralSky/StarField/Cloud/Storm/Precipitation loops, Cava off by default (toggle spawns/kills on demand), all Behaviors become 100ms fade or none.
  - Test: `rg Timer` in src shows only minute-clock + OSD-hide + 6h weather; GPU <15% fullscreen video.

## Phase 6 — Event-driven services (shell commands only when needed)
- [ ] 6.1 Battery UPower + backlight sysfs sole-writer
  - Do: zbus UPower DeviceChanged (multi-battery aggregate, health/time), sysfs intel_backlight read/write + udev watch, Stevens quadratic same as dimmer, MIN 12, override flag /run/sshell/backlight-override shared with dimmer.
  - Test: plug/unplug updates <2s zero forks, brightness key writes once OSD once, mock dual-battery passes.
- [ ] 6.2 Audio PipeWire + NetworkManager + BlueZ + MPRIS
  - Do: PipeWire/zbus volume/mute signals (wpctl fallback only if socket missing); NM StateChanged/DeviceChanged + BlueZ PropertiesChanged (no nmcli poll, rescan button +30s cooldown); MPRIS PropertiesChanged Next/Prev via D-Bus (no playerctl fork), ignore[] respected.
  - Test: 10x volume keys zero wpctl execs, wifi off shows off <3s, play/pause updates bar zero playerctl, airplane blocks weather.
- [ ] 6.3 System info + resources on-demand only
  - Do: os/user/host/chassis once via getpwuid + /etc/os-release + D-Bus chassis (not hostnamectl fork loop), uptime from /proc/uptime read on Settings open only; CPU/RAM/disk/graphs only when Settings System page visible (not 1s background).
  - Test: idle strace zero reads of /proc/stat/meminfo/cpuinfo; opening System page loads <500ms.
- [ ] 6.4 Weather cached HTTPS
  - Do: reqwest https wttr.in j1 10s timeout, 6h cache ~/.cache/sshell/weather.json, fetch only on Connected + manual, empty city = disabled, no GPS leak log.
  - Test: airplane 24h zero requests, city change refetches once.
- [ ] 6.5 No-interpolation exec rule
  - Do: forbid `Command("bash -c"+var)`; use fs/zbus/image/reqwest; allowlist matugen/rescan/cliphist only on click with canonicalize + prefix check + tempfile + rename.
  - Test: `rg "bash -c"` zero in src; clippy -D warnings green.

## Phase 7 — Battery singularity + dimmer integration
- [ ] 7.1 Dimmer integration exact
  - Do: keep battery-dimmer.sh zero-fork loop untouched; Rust respects BL_DIMMED/state.env, manual key sets override suspends auto-dim until AC; /etc/battery-dimmer.env authoritative.
  - Test: 49% dims to Stevens ~49% orig within 60s, 54% restores, manual up during dimmed sets override logged throttled.
- [ ] 7.2 System powersave Raptor Lake-P + i915
  - Do: PPD balanced AC / power-saver battery, deep sleep, PSR+FBC, NVMe 5500us, iwlwifi power_save, btusb autosuspend, snd_hda power_save, powertop autotune udev, vrr off battery.
  - Test: idle C10 >70%, package <3.5W, PSR=1, mem_sleep [deep], no TLP+PPD conflict.
- [ ] 7.3 Hyprland battery profile
  - Do: blur off, shadow off, dim_inactive off, gaps 3/8 rounding 8, animations off (see Phase 8), misc vfr on.
  - Test: `hyprctl getoption` blur 0 anim 0 on battery.
- [ ] 7.4 Verify numbers
  - Do: record upower, max_brightness, energy_full/design, powertop html before/after, systemd-analyze blame, free RSS.
  - Test: health ±1% old script, boot <25s, sshell-rs <100MB vs QML ~300MB.
- [ ] 7.5 Docs for new user
  - Do: document `battery-dimmer.sh --status`, `journalctl -u battery-dimmer -f`, `cat /etc/battery-dimmer.env`, override clear on AC.
  - Test: user can run 3 commands unaided.

## Phase 8 — Hyprland Fedora config (minimal anim, correct binds)
- [ ] 8.1 Split conf + battery profile
  - Do: rewrite env/execs/general/rules/keybinds/colors/hypridle/hyprlock; env TERMINAL=foot QT wayland XDG Hyprland (drop ILLOGICAL_IMPULSE); general blur off gaps 3/8 rounding 8 dim off animations off; layerrules sshell:.* fade only; `hyprland --check` clean.
  - Test: windows open instant/100ms fade max, no slide/popin.
- [ ] 8.2 Single-handler keybinds (fix double-step)
  - Do: one path per key: `sshell-rs brightness/volume/mpris` via sysfs/D-Bus + OSD; delete `brightnessctl s 5%` + `wpctl` + `playerctl` exec lines + quickshell globals; apps foot/thunar/librewolf/code/obsidian with fallback xterm/nautilus/firefox; launcher Super+Space, CC Super+N, clipboard Super+V, settings Super+I, session Ctrl+Alt+Del, shell toggle Super+X, wallpaper Super+W.
  - Test: one press = +5% once, journal single event, missing app notifies never hangs.
- [ ] 8.3 Idle/lock/suspend saves battery
  - Do: hypridle lock 300s hyprlock static wallpaper pam, dpms 330s, suspend 600s battery-only; logind lid/power suspend; session_lock_xray false.
  - Test: lid close suspends, resume reprobes dimmer brightness correct.
- [ ] 8.4 Portals/clipboard/cursor hardened
  - Do: dbus-update-env, keyring start, wl-paste watch guarded, cursor Bibita fallback Adwaita, guard hyprpm reload.
  - Test: portal active, grim|wl-copy works, reboot twice single daemons.
- [ ] 8.5 Monitor/input
  - Do: monitor preferred auto 1, eDP-1 1920x1200@60 verified, HDMI hotplug bar follows, kb us caps:escape, touchpad natural+disable-typing.
  - Test: `hyprctl monitors` ok, unplug/replug no stuck grab, wev caps=esc.

## Phase 9 — Security hardening (Fedora SELinux, least privilege)
- [ ] 9.1 Privileges + SELinux + secrets
  - Do: sshell-rs user never setuid, dimmer root strict NoNewPrivileges ProtectSystem=strict, restorecon, libsecret/keyring, city not logged, state 600.
  - Test: `systemd-analyze security` SAFE <=2 exposed, `ausearch -m avc` clean, ps shows user bar + root dimmer only.
- [ ] 9.2 Fail-safe matrix
  - Do: handle no BAT (desktop PCT -1 restore), no net cached weather, no online file discharging logic, design 0 health 100, max 0 skip, bl_power!=0 write+return, hot-unplug evict, EC spike hysteresis 3.
  - Test: airplane + unplug 50±1 + lid + HDMI + dock + mock sysfs overlay all no-crash throttled log.
- [ ] 9.3 Atomic writes + crash restore
  - Do: tmp+fsync+rename all state/config, dimmer cleanup restores ORIG on TERM, KillMode mixed Timeout 5s, Rust Drop releases grab/hides OSD.
  - Test: kill -9/-TERM restores correctly, power-pull never corrupts json_verify.
- [ ] 9.4 Input fuzz no panics
  - Do: clean_val port filter-digits strip-zeros clamp THRESHOLD 5-95 HYS 1-10 DIM 1-95, capacity 150 clamp 100, empty brightness skip, path canonicalize prefix check reject ../../../.
  - Test: `cargo test` 20 hostile inputs green, 60s fuzz no panic optional.
- [ ] 9.5 Supply chain
  - Do: commit Cargo.lock, `cargo audit/deny` clean, pinned stable, sha256 verify rustup-init, no curl|bash unpinned.
  - Test: `cargo build --locked` offline after fetch succeeds.

## Phase 10 — No-VM validation + flawless cutover
- [ ] 10.1 Static singularity
  - Do: `shellcheck` all sh, `bash -n`, `cargo clippy -D warnings`, `fmt --check`, `rg "pacman|yay|quickshell|qt6-5compat|execDetached|Process \\{|bc |curl -m"` zero in hot paths, `rg Timer` only allowed 3 with comments.
  - Test: all linters green.
- [ ] 10.2 Dry-run + mock tests
  - Do: `DRY_RUN=1 sudo ./fedora-setup.sh --yes` prints dnf/cp/systemctl; `cargo test` >=40 with fake sysfs covering dim 50 hysteresis 53 AC restore override suspend-drift hotplug.
  - Test: tests green, log contains THRESHOLD=50 DIM 30 sshell-rs dimmer lines, zero forks in harness.
- [ ] 10.3 TTY rehearsal checklist
  - Do: write TTY-CHECKLIST.md: Minimal boot login von, `ip a`, `sudo dnf install -y git`, `git clone https://github.com/<you>/cshell`, `cd`, `chmod +x fedora-setup.sh`, `sudo ./fedora-setup.sh --yes`, `sudo reboot`, tuigreet Hyprland, Super+Space/V/N/I, `battery-dimmer.sh --status`.
  - Test: can recite unaided, friend finds no missing step, every cmd has expected output.
- [ ] 10.4 Power bars
  - Do: define PASS sshell-rs <0.5% CPU <100MB <10 wakeups/s C10>70% boot<25s key<100ms launcher<200ms; record powertop CSV.
  - Test: 2x over bar = BLOCKER fix Timer/fork.
- [ ] 10.5 Push wipe boot accept lock-in
  - Do: repo has fedora-setup.sh sshell-rs configs Plan/battery-dimmer Plan/Plan.md TTY-CHECKLIST README-FEDORA; `git push tag v2.0-rs`; Anaconda keep EFI BTRFS; first-boot 10 interactions + `journalctl -p err -b` zero errors + C10>70% + 3x reboot stable; `--uninstall` restores latest backup; after 1 week move QML to attic (keep dimmer).
  - Test: fresh clone builds green, `inxi` shows Hyprland i915 BAT0 PPD+dimmer active, first-boot.log saved.

---

## Appendix A — TTY one-shot (memorize)
```bash
login: von
git clone https://github.com/<YOU>/cshell
cd cshell
chmod +x ./fedora-setup.sh
sudo ./fedora-setup.sh --yes
sudo reboot
# tuigreet -> Hyprland + sshell-rs bar + dimmer
systemctl is-active battery-dimmer power-profiles-daemon
battery-dimmer.sh --status
journalctl --user -u sshell-rs -n 20
```

## Appendix B — Rust non-negotiables
1. Signals only: UPower/NM/BlueZ/MPRIS/hyprland-IPC/inotify/udev. Allowed timers: minute-clock, OSD-hide 1500ms, weather 6h.
2. No bash -c interpolation. fs/zbus/hyprland-rs/image/reqwest. Forks only on click.
3. Fade 100ms max. No slide/popin/blur/shader loops.
4. Single backlight writer. Dimmer auto, Rust manual, shared override.
5. Atomic writes, validated config, logged errors. No silent ready=true.

## Appendix C — Arch -> Fedora map (for fedora-setup.sh)
hyprland(+hyprlock/hypridle/hyprpaper) official, DELETE qt6-5compat/quickshell -> gtk4/layer-shell, brightnessctl same sysfs-fallback, google-material-symbols-fonts, google-noto-emoji-fonts, cascadia-code-nf-fonts, rsms-inter-fonts, jetbrains-mono-fonts, cliphist ImageMagick matugen playerctl jq (COPR fallback documented), starship fish same, NetworkManager-wifi bluez-tools.

## Appendix D — Dimmer quick ref (works off bat)
```bash
sudo battery-dimmer.sh --status
cat /etc/battery-dimmer.env  # THRESHOLD=50 DIM_BY_PERCENT=30 MIN_PERCENT=12
journalctl -u battery-dimmer -f
```
Math: orig*(70/100) then Stevens quadratic ~49% orig perceived -30%. Dims <=50 restores >53. Manual key overrides until AC.
