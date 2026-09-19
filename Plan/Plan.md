# cshell Rust + GTK4 Singularity Plan — Fedora Minimal TTY to Hyprland, flawless first boot

> Target: Lenovo IdeaPad Slim 3 14IRH10 (i5-13420H Raptor Lake-P 8c/12t, Intel UHD i915, eDP-1 1920x1200@60 AUO B140UAN08.0, Intel CNVi WiFi + AX201 BT, BAT0 COSMX 50Wh design / 48.3Wh full / 96.6% health, 16GiB, Micron MTFDKCD512QGN NVMe, BTRFS + 8G ZRAM, Fedora 44 Cinnamon now).
> Flow: Fedora Minimal ISO -> TTY login as von -> `git clone https://github.com/<YOU>/cshell` -> `cd cshell` -> `chmod +x ./fedora-setup.sh` -> `sudo ./fedora-setup.sh` -> `sudo reboot` -> tuigreet -> Hyprland + sshell-rs + battery-dimmer.
> Stack: Rust + GTK4 + gtk4-layer-shell only. No QML/Quickshell in hot path. Event-driven (Hypr IPC + UPower/NM/BlueZ/MPRIS D-Bus + inotify + udev + control socket). No polling. Shell forks only on explicit user action. Animations: 100ms opacity fade max, everything else instant.
> Battery law: `Plan/battery-dimmer.sh` works off-bat after install: <=50% dims ~30% perceived (Stevens quadratic ~49% linear), restores >53%. `/etc/battery-dimmer.env` is authoritative: THRESHOLD=50 HYSTERESIS=3 DIM_BY_PERCENT=30 MIN_PERCENT=12.
> No-VM law: no bare-metal run until static + dry-run + mock-sysfs + `sshell-rs --check` are all green. User is new: every step copy-paste with expected output.

## UI Freeze (do not redesign, Rust must match QML pixels)
- `config.jsonc`: bar top floating h=38 margin=10 padding=5. Left Launcher/Workspaces/Mpris, Center Clock/Weather, Right Battery/Tray. Workspaces 1-5 dot/circle. Weather 3600s metric hideLocation. Tray show names. OSD 1500ms top-center. Wallpaper ~/Pictures/wallpapers+gifs image mode. Mpris popupVis true hideOnPause maxW 666 no-artist. Clock date 12h. CC 450 right no-pfp. Notif top-right max5/group3/5s. Launcher 400x500. Theme round dark Caskaydia/Inter/JetBrains/MaterialSymbolsRounded.
- Tokens: pad 2/5/8/12/16/20/24 radius 6/10/14/22 barH 40 barM 10 M3 dark bg #141313 onSurface #DEE2E6 primary #D0BCFF bg-alpha .45 surface .9 border .1-.2 fonts 5-64. Rust uses only 100ms fade.
- Bar floating rounded left/center/right. CC overlay r=14 userRow user@host + 4 buttons + 4-col grid 60px + edit hint + grouped notifs + DND + sliders vol/bri + Wifi/BT/Audio 50% detail. Launcher pill search+list Up/Down/Enter/Esc + No-results. Session fullscreen title+uptime+4 buttons+hint+Esc. Background black PreserveAspectCrop instant/100ms. Popups: mpris 400x165 battery clock weather wifi/BT tray 380x300 settings 700x550 wallpaper 600x500 OSD 300x60 toast 350x80.

## Brutal audit (git 2ffe0b8..5ca645a, 3584-line Rust + 988-line dimmer + 307-line setup + 6 hypr confs)

### fedora-setup.sh BLOCKERS (fail bare-metal today)
- B1 No arg parsing: ASSUME_YES/DONE_MARKER/UNINSTALL declared never used. `--yes/--uninstall/--help` ignored. README `--uninstall` rollback is a lie. No resume, no trap ERR line.
- B2 Rawhide COPR hardcoded (`.../fedora-rawhide/...repo`) on Fedora 44 + `curl -sL` no `-f` + second `|| true` masks 404 -> broken repo file, hyprland resolve fail. No `rpm -E %fedora` detect, no makecache.
- B3 Missing pkgs on Minimal: `rsync` used never installed, `rustup-init` never installed (ensure_rust assumes it), `upower` missing, `matugen` silently skipped via --skip-unavailable (theme dead), `mate-polkit` heavy vs polkit-gnome, service `tuned` vs `tuned-ppd` name wrong.
- B4 Wrong-user writes: `cp -a starship/config.jsonc` as root into $HOME (root-owned), `HOME` expanded as /root inside `bash -lc "source $HOME/.cargo/env"`, `cat > /etc/...` and `cat > .../sshell-rs.service` bypass `run()` so DRY_RUN still writes to /etc.
- B5 Dimmer env never loaded: dimmer unit has no `EnvironmentFile=/etc/battery-dimmer.env`, no `StandardOutput=journal` (docs `journalctl -u battery-dimmer` empty, logs only /run file). No `daemon-reload`, user service never `enable --user`.
- B6 sed mutates Cargo.toml in place every run (destroys git clean, not idempotent). No --locked, no build-time note (LTO=z 3-5min).
- B7 greetd overwrite no backup, `user=greeter` + `--cmd Hyprland` version-fragile. grubby `iwlwifi.power_save/btusb/snd_hda` as kernel args are module params (no-op, need modprobe.d). All-USB autosuspend udev can suspend AX201 BT. No backlight udev `GROUP=video` + `usermod -aG video` so user `sshell-rs brightness-up` gets EACCES. verify_install always exit 0, never checks sshell-rs/hypr check.

### battery-dimmer.sh MAJOR (strong core, 7 integration lies)
- M1 udev `ACTION=="add|change|remove"` invalid (no | alternation) -> wake pipe never fires, falls back to 60s poll only. RUN forks sh+dd+echo per event (zero-fork claim is daemon-loop only).
- M2 Stevens math correct (70% linear x2 = 49% linear = -30% perceived) but already-dim edge skips ORIG save -> restore leaves user at manual 10% after AC. MIN 12% matches Rust 0.12 good.
- M3 `/run/sshell/backlight-override` never read by dimmer (dead letter). Override relies only on delta>TOL (25 steps at 21333 max). Exact-target slider write causes re-dim race.
- M4 Suspend drift `DRIFT > INTERVAL+10` misses short suspends on AC (30s < 130s). Need PrepareForSleep or fixed 15s threshold.
- M5 Journal lie: writes to /run/daemon.log not stdout, unit no StandardOutput -> journalctl empty. State LOCK PID reuse (no cmdline verify). `ACTIVE_STATE rm` on clean exit ok, kill -9 stale ok via flock.
- M6 Telemetry overflow-safe (/1000 prescale), multi-battery aggregate, HID filter, dGPU suspend skip all correct. `BAT=-1` desktop restores good. Missing bounds on RESCAN/DEBOUNCE, `clean_val` strips `-` so -5 becomes 5.
- M7 ProtectSystem=strict without ReadOnlyPaths /proc + /sys/class/power_supply risks denials. Needs ReadOnlyPaths + EnvironmentFile in unit template.

### sshell-rs MAJOR (event architecture right, 8 perf/correctness holes)
- R1 `refresh_bar` on glib main thread does mpris::current (new tokio runtime + list_names + props ~50ms) + 2x hypr sockets + battery + net per Refresh. Hypr event + MPRIS spam + minute tick = jank, workspace <50ms fails. Need fast/slow path split + debounce.
- R2 watch_mpris/watch_net match ALL PropertiesChanged then filter in Rust (session+system bus noisy) -> wake storm. Need path_namespace/sender MatchRule in bus daemon.
- R3 Single bar window, no per-monitor bars, ignores monitoradded/removed -> HDMI has no bar (screens[0] bug moved, not fixed). Document or fix.
- R4 `wallpaper connect_show` builds 60x 256px thumbs synchronously (Triangle ~200ms each = 12s freeze). Need async + cache. `replace('~')` replaces all tildes, symlink cycle infinite (no visited set). webp listed but image crate has no webp feature.
- R5 theme::find_key misses real matugen `{"primary":{"default":{"hex":"#..."}}}` (object not string) -> accent None. save_config drops comments + unknown keys. Validation only bar height/pos/style, other Values silent-default.
- R6 audio default 0.5 phantom when no sink, set_volume does not unmute (muted+vol confusing). weather never auto-fetches (only manual Refresh; stale shows -- forever). launcher Exec quoted-path split broken, gio preferred ok, duplicates by ID not Name.
- R7 notifications: toast consumes list (`close(id)` after read) so CC history never grows; CC shows only count label, no list widget (parity gap vs QML grouped). Tray serve ok but icons text-only documented, item path assumes /StatusNotifierItem.
- R8 FADE `* {transition 100ms}` + theme::reload_into on EVERY Refresh (CSS reparse ~10ms + restyle). Need mtime guard. ipc ack no timeout (key CLI blocks if main loop hung). brightness sysfs needs video group (see B7). `*` CSS ok (opacity only).

### hypr configs MAJOR
- H1 execs runs BOTH `sshell-rs &` and `qs -c sshell &` when both present (double bar + triple backlight writers). Must be `sshell-rs || qs` or drop qs (retired). `hyprpm reload` blocks startup, no hypridle/nm-applet/polkit exec-once (tray empty, auth fails).
- H2 general: `gaps_workspaces=50` wastes 14in panel, `vrr=1` on fixed 60Hz AUO (warn), `allow_tearing` + steam immediate gaming-centric vs battery. blur off good but `layerrule sshell-all blur=on` re-enables ~1W shader. animations off good but 12 dead curves + layerrule slide/popin contradict fade-only (ignored while disabled, fix to fade).
- H3 keybinds `sh -c 'command -v sshell-rs ... || brightnessctl/wpctl/playerctl'` safe (no var) but forks sh per press (user action ok). `Super+V clipboard` handled (launcher `;` mode) ok. Apps foot/thunar/librewolf/code/obsidian no fallback (Minimal missing -> silent dead key, need notify). `btop && foot` silent fail.
- H4 Missing hypridle/hyprlock confs (lock 300/dpms 330/suspend 600 battery-only unimplemented), no logind lid drop-in (relies on Fedora default). env has KDE leftovers, missing MOZ_ENABLE_WAYLAND=1 GDK_BACKEND SDL (XWayland power). colors hyprbars dead (no plugin install).

## Phase 1 — Freeze and baseline (Cinnamon, before wipe)
- [ ] 1.1 Snapshot system and repo
  - Do: `inxi -Fxxxz > ~/sshell-baseline.txt; cat /sys/class/backlight/*/max_brightness; cat /sys/class/power_supply/BAT0/{capacity,status,energy_full,energy_full_design}; upower -d | head -40; cd ~/Downloads/sshell-main && git status --short && git log --oneline -5; sudo Plan/battery-dimmer.sh --status | tee ~/dimmer-baseline.txt`
  - Test: baseline has IdeaPad+i915+BAT0, max 21333, dimmer shows intel_backlight+BAT0. Fail=stop.
- [ ] 1.2 Verify Minimal ISO on paper
  - Do: download Fedora Everything netinstall, `sha256sum -c *-CHECKSUM`, plan: keep nvme0n1p1 EFI mount /boot/efi DO NOT FORMAT, BTRFS /, von+wheel, hostname von-fedora, Minimal, sshd off.
  - Test: checksum OK, can recite EFI-stays + BTRFS + wheel. Fail=reread docs.
- [ ] 1.3 Push clone-ready repo
  - Do: `git add -A; git commit -m "freeze pre-singularity"; git push origin main` (HTTPS, TTY has no SSH keys).
  - Test: `git ls-remote https://github.com/<YOU>/cshell` ok from second machine.
- [ ] 1.4 Lock constraints
  - Do: confirm Rust+GTK4 only, same UI tokens, fade-only, dimmer 50/30, one-shot `sudo ./fedora-setup.sh` then reboot.
  - Test: recite TTY 5-liner unaided.

## Phase 2 — fedora-setup.sh TTY bootstrap (fix B1-B7, idempotent+logged)
- [ ] 2.1 Arg parsing + resume + dry-run honesty
  - Do: parse --yes/--help/--uninstall/--dry-run, ASSUME_YES gate, DONE_MARKER per-step resume, trap ERR with line no, `${VAR:?}` guards, LOG /var/log/cshell-setup.log tee. Route ALL writes (ENV/greetd/service) via run().
  - Test: `bash -n` clean, `DRY_RUN=1 sudo ./fedora-setup.sh --yes` writes nothing to /etc, rerun no-op, `--help` exits 0.
- [ ] 2.2 Version-aware repos + Minimal pkg set
  - Do: `VER=$(rpm -E %fedora)`, COPR url with $VER not rawhide, `curl -fSL` fail-fast + makecache, install rsync rustup-init upower + hypr/gtk/net/audio/power/mesa/fonts/utils, COPR fallback note for matugen, `fc-cache -f; fc-match` verify.
  - Test: `rpm -q hyprland gtk4 rsync rustup-init upower` ok, zero `pacman|quickshell` strings.
- [ ] 2.3 Root/user split + backup/restore
  - Do: run_as_user for all $HOME writes, backup hypr/gtk/sshell to ~/.local/state/sshell/backups/$TS+latest, deploy configs/hypr+gtk+config.jsonc, mkdir wallpaper/thumbs, restorecon, implement --uninstall from latest.
  - Test: no root-owned files in $HOME (`find ~ -uid 0` empty), restore round-trips.
- [ ] 2.4 Dimmer on-by-default 50/30 + backlight perms
  - Do: install -m0755 dimmer, write /etc/battery-dimmer.env 50/3/30/12/60, patch dimmer unit EnvironmentFile+StandardOutput=journal+ReadOnlyPaths, `--install-udev/service`, `udev GROUP=video MODE=0664 backlight` + `usermod -aG video von`, enable battery-dimmer tuned-ppd NetworkManager bluetooth.
  - Test: `is-active battery-dimmer` active, `--status` shows intel_backlight+BAT0, env 50/30, journal shows Threshold 50% Dim 30%, user can write brightness.
- [ ] 2.5 Boot + powersave + smoke
  - Do: set-default graphical + enable greetd (backup old config, tuigreet --cmd Hyprland for von), grubby deep+PSR/FBC+NVMe5500 only, modprobe.d for iwlwifi/btusb/snd_hda, narrow USB autosuspend allowlist, verify_install checks Hyprland+dimmer+sshell-rs+`Hyprland --check` and fails loud.
  - Test: reboot->tuigreet->Hyprland, `mem_sleep [deep]`, no qs proc, log has zero X mark.

## Phase 3 — Rust skeleton (~0% idle, same tokens)
- [ ] 3.1 Cargo + layer-shell bar + --check
  - Do: keep gtk4/layer-shell/glib/gio/serde/zbus/tokio/tracing/notify/image/chrono, remove sed hack, --locked build, single top bar h38 margin10 ns sshell:bar exclusive, CSS matugen tokens, `sshell-rs --check` validates config+sysfs+dimmer50/30 without GUI.
  - Test: `cargo build --release` zero warnings, `--check` exit0 good/2 bad with line, idle <0.5% <100MB.
- [ ] 3.2 JSONC strict + live reload
  - Do: strip_jsonc preserves https, atomic rename writes, inotify 150ms debounce, fail-loud file:line:col, corrupt keeps last-good+notify, preserve unknown keys on save.
  - Test: 10 cases (comments/trailing-comma/https/missing) green, edit relayout <200ms.
- [ ] 3.3 Clock minute-aligned + workspaces events
  - Do: timeout to next minute boundary (only timer) + battery piggyback, hypr socket2 workspace/focusedmon/monitor hotplug events, persistent 1-5 dot fallback.
  - Test: zero wakeups between minutes, Super+1..5 <50ms, HDMI plug follows or documented single-bar.
- [ ] 3.4 Single instance + logging
  - Do: control sock XDG first (/run/user/1000) 0600, second instance exits 0, tracing->journald + crash.log, Hypr socket loss no spin (thread exits).
  - Test: double launch exits 0, kill Hypr socket stays alive, socket perms 600.

## Phase 4 — Bar faithful (no polling)
- [ ] 4.1 Layout styles + modules + popups
  - Do: full/floating/islands/modules radius14 alpha .45 border .1, Launcher/Workspaces/Mpris left Clock/Weather center Battery/Tray right, popups mpris400x165 battery clock weather wifi/BT/tray380 settings700 wallpaper600 OSD300 toast350 same show rules (hideOnPause max666 no-artist hideLoc).
  - Test: visual diff vs QML screenshots, empty side collapses, popups 100ms fade only.
- [ ] 4.2 Fast/slow refresh split
  - Do: workspace-only label update on hypr event (<10ms, no D-Bus), full refresh (mpris/battery/net) debounced 100ms coalesced, mpris cached Track async (no runtime per Refresh), theme reload only on mtime change.
  - Test: 5x Super+1 no jank, `strace -e execve` idle60s zero execs, RSS<100MB.
- [ ] 4.3 D-Bus narrow filters
  - Do: MPRIS path_namespace /org/mpris/MediaPlayer2, NM sender org.freedesktop.NetworkManager, BlueZ org.bluez (bus-side filter, not Rust if).
  - Test: session/system bus spam (notify-send loop) causes <5 wakeups/s, top idle <0.5%.
- [ ] 4.4 OSD + notifications parity
  - Do: OSD top-center 1500ms gen-counter (no stacking), toast top-right DND-aware WITHOUT consuming list, CC count + list rows (title/body/time) + clear, max5/group3/5s same as QML.
  - Test: volume key OSD once hides1.5s, notify-send stacks 5 then evicts oldest, DND suppresses toast keeps list.

## Phase 5 — Launcher CC Session Settings Wallpaper (same UX, zero background)
- [ ] 5.1 Launcher + clipboard on-demand
  - Do: overlay 400x500 pill Entry+List fuzzy in-mem, scan .desktop once+inotify, gio launch (no shlex), Esc clears-then-closes, Up/Down/Enter, `;` runs cliphist list max20 virtualized only on Super+V.
  - Test: cold open <200ms, 300 apps <100ms, double Super+Space never steals, 10k clip rows <300ms.
- [ ] 5.2 ControlCenter 450 right
  - Do: userRow+4 buttons, 4-col grid60px+edit hint, notif grouped+DND+clear, vol/bri SliderRows +% (bri writes sysfs once+override flag XDG path, vol unmutes on up), Wifi/BT/Tray details fill-on-open + Rescan 30s cooldown.
  - Test: every toggle matches QML, one slider drag = one write + one OSD.
- [ ] 5.3 Session + Settings atomic
  - Do: session overlay 4 buttons+uptime+arrow+Esc via logind/systemctl, settings Bar/Modules/Theme/Weather pages write tmp+rename, footer notes comments-dropped.
  - Test: suspend/reboot/logout work, settings survive reboot, same wallpaper twice = 0 matugen.
- [ ] 5.4 Wallpaper async + Background instant
  - Do: fix `~/` expand + visited-inode cycle guard, webp feature or drop filter, thumbs async worker + cache (no 12s freeze), apply persists+hyprpaper reload+matugen once if changed, toggle_visible via hyprpaper unload/all.
  - Test: first open <500ms (placeholders then fill), GIF plays, toggle instant, shader behind flag off.
- [ ] 5.5 Animations cull
  - Do: delete sky/star/cloud/storm/cava loops (Cava toggle spawns/kills on demand), all Behaviors 100ms opacity or none, layerrules fade only.
  - Test: `rg Timer` only minute+OSD-hide+6h-weather comment, fullscreen video GPU<15%.

## Phase 6 — Services (forks only on click, no bash -c var)
- [ ] 6.1 Battery UPower signals + sysfs sole-writer
  - Do: zbus DeviceChanged, multi-battery aggregate (energy/charge/capacity same as dimmer), sysfs intel primary, MIN12, XDG override flag shared with dimmer (both read/write same path).
  - Test: plug/unplug <2s zero forks, brightness key once+OSD once, dual-battery mock passes.
- [ ] 6.2 Audio PipeWire + NM/BlueZ + MPRIS D-Bus
  - Do: wpctl only on key/slider/open (no timer), NM StateChanged + BlueZ PropertiesChanged narrow, MPRIS method Next/Prev/PlayPause D-Bus first playerctl fallback once per click, ignore[] respected.
  - Test: 10x vol keys zero extra execs, wifi off <3s, play updates bar zero playerctl poll.
- [ ] 6.3 Sysinfo on-demand + weather HTTPS cached
  - Do: user/host/chassis once (getpwuid/os-release/D-Bus, no hostnamectl loop), uptime /proc on Settings open only, CPU/RAM/disk only when System page visible, reqwest/curl https wttr.in j1 10s 6h cache manual+Connected-stale fetch, empty city disabled no leak.
  - Test: idle strace zero /proc/stat reads, airplane24h zero requests, city change refetch once.
- [ ] 6.4 Exec allowlist + fuzz
  - Do: forbid `bash -c+var`, fs/zbus/image only, allow matugen/rescan/cliphist/hyprctl on click with canonicalize+prefix+tmp+rename, clean_val port clamp THRESH5-95 HYS1-10 DIM1-95 cap150->100.
  - Test: `rg "bash -c"` zero hot path, `cargo test` 20 hostile inputs green, clippy -D warnings green.

## Phase 7 — Battery singularity + dimmer handshake
- [ ] 7.1 Dimmer exact 50->30% + override shared
  - Do: keep zero-fork loop untouched except 3 fixes: split udev into 3 rules (add/change/remove) or `ACTION!="remove"`, add EnvironmentFile+StandardOutput+ReadOnlyPaths to unit template, read XDG override flag (Rust manual suspends auto until AC). Already-dim edge saves ORIG.
  - Test: 49% dims to ~10453/21333 within 60s (AC off), 54% restores, manual up during dim sets override logged throttled, AC clears.
- [ ] 7.2 Raptor Lake-P + i915 powersave
  - Do: PPD balanced AC / power-saver battery (never TLP+PPD), deep sleep, PSR+FBC (test PSR2 if flicker on AUO), NVMe5500, modprobe iwlwifi/btusb/snd_hda powersave, narrow USB autosuspend (allowlist mouse/BT quirk), vfr on vrr off battery.
  - Test: C10>70% package<3.5W PSR=1 mem_sleep[deep] no TLP conflict, powertop html before/after.
- [ ] 7.3 Hypr battery profile
  - Do: blur off shadow off dim_inactive off gaps3/8 rounding8 anim off misc vfr on, layerrule blur off fade only, `hyprctl getoption` pinned.
  - Test: blur0 anim0 on battery, windows instant/100ms fade max.
- [ ] 7.4 Numbers + docs
  - Do: record upower energy_full/design health, max_brightness, powertop csv, blame, RSS (<100MB vs QML ~300MB), document `--status` + `journalctl -u battery-dimmer -f` + `cat /etc/battery-dimmer.env` + override clear on AC.
  - Test: user runs 3 cmds unaided, health +-1% old script, boot<25s.

## Phase 8 — Hyprland Fedora (minimal anim, correct binds)
- [ ] 8.1 Split conf + portals/clipboard/cursor
  - Do: env TERMINAL=foot QT-wayland XDG + MOZ_ENABLE_WAYLAND GDK_BACKEND SDL (drop kde prefix), execs `sshell-rs || qs` single (drop dual), dbus-update-env keyring wl-paste-guarded Bibita/Adwaita, hypridle/hyprlock confs lock300 dpms330 suspend600 battery-only + logind lid suspend, `Hyprland --check` clean.
  - Test: fresh login one bar `pgrep sshell-rs`=1 no qs, portal grim|wl-copy works, lid suspends resume brightness correct.
- [ ] 8.2 Single-handler keys (no double-step)
  - Do: one path per XF86 via `sshell-rs brightness/volume/mpris` + OSD, delete brightnessctl/wpctl/playerctl exec + qs globals, apps with fallback notify (foot/xterm thunar/nautilus librewolf/firefox), Super+Space/N/V/I/X/W Ctrl+Alt+Del XF86 as spec.
  - Test: one press +5% once journal single event, missing app notifies never hangs.
- [ ] 8.3 Monitor/input
  - Do: monitor preferred auto1 eDP1920x1200@60 verified HDMI hotplug bar follows or documented, kb us caps:escape touchpad natural+disable-typing gestures minimal.
  - Test: `hyprctl monitors` ok, unplug/replug no stuck grab, wev caps=esc.

## Phase 9 — Security hardening (SELinux least-priv)
- [ ] 9.1 Privs + SELinux + secrets
  - Do: sshell-rs user never setuid, dimmer root NoNewPriv ProtectSystem-strict + ReadWrite /run /var/lib /sys/class/backlight /sys/devices + ReadOnly /proc /sys/class/power_supply, restorecon binaries+configs, libsecret/keyring city never logged state 600.
  - Test: `systemd-analyze security` <=2 exposed, `ausearch -m avc` clean, ps user bar + root dimmer only.
- [ ] 9.2 Fail-safe matrix (mock sysfs, no VM)
  - Do: no BAT PCT-1 restore, no net cached weather, online-file missing discharging logic, design0 health100, max0 skip, bl_power!=0 write+return, hot-unplug evict, EC spike HYS3, stale bootID purge, corrupt keeps last-good.
  - Test: airplane+unplug50+-1+lid+HDMI+dock+overlay all no-crash throttled log, `cargo test` >=40 incl dim50 HYS53 AC-restore override suspend-drift hotplug.
- [ ] 9.3 Atomic + crash restore
  - Do: tmp+fsync+rename all state/config, dimmer TERM restores ORIG KillMode-mixed Timeout5s, Rust Drop releases grab/hides OSD, stale XDG sock unlink only if dead.
  - Test: kill -9/-TERM restores, power-pull json_verify clean, ipc ack timeout 500ms never blocks keys.

## Phase 10 — No-VM validation + flawless cutover
- [ ] 10.1 Static singularity
  - Do: `bash -n` both sh, python shellcheck-substitute (no pacman/quickshell/execDetached/bc hot), `cargo fmt --check clippy -D warnings test`, `rg Timer` only allowed3, `rg "bash -c"` zero hot.
  - Test: all linters green, zero forks harness.
- [ ] 10.2 Dry-run + mock
  - Do: `DRY_RUN=1 sudo ./fedora-setup.sh --yes` prints dnf/cp/systemctl zero /etc writes, mock overlay BAT/capacity/online + backlight max/cur asserts Stevens 21333->10453 MIN2559 HYS.
  - Test: log has THRESHOLD=50 DIM30 sshell-rs dimmer lines, tests green.
- [ ] 10.3 TTY rehearsal + bars + push
  - Do: TTY-CHECKLIST 0-6 with expected outputs, PASS sshell-rs<0.5% <100MB <10wake C10>70% boot<25s key<100ms launcher<200ms, repo has setup+sshell-rs+configs+dimmer+Plan+TTYX+README-FEDORA, tag v2.0-rs, Anaconda keep-EFI BTRFS, 3x reboot stable + `journalctl -p err -b` zero + `--uninstall` restores latest, QML to attic after 1wk (keep dimmer).
  - Test: fresh clone builds locked offline after fetch, inxi Hyprland i915 BAT0 PPD+dimmer active, first-boot.log saved.

---
## Appendix A — TTY one-shot (memorize)
```bash
login: von
git clone https://github.com/<YOU>/cshell; cd cshell
chmod +x ./fedora-setup.sh
sudo ./fedora-setup.sh
sudo reboot
# tuigreet -> Hyprland + sshell-rs + dimmer
systemctl is-active battery-dimmer power-profiles-daemon NetworkManager bluetooth
battery-dimmer.sh --status
cat /etc/battery-dimmer.env  # THRESHOLD=50 DIM_BY_PERCENT=30 MIN_PERCENT=12
journalctl -u battery-dimmer -f
sshell-rs --check
```

## Appendix B — Rust non-negotiables
1. Signals only: UPower/NM/BlueZ/MPRIS/hypr-IPC/inotify/udev. Timers: minute-clock, OSD-hide 1500ms, weather 6h only.
2. No bash -c var. fs/zbus/hypr-raw/image/curl-argv. Forks only on click.
3. Fade 100ms max. No slide/popin/blur/shader loops. Layerrules fade.
4. Single backlight writer: dimmer auto, Rust manual, XDG override shared.
5. Atomic writes, validated config, logged errors. No silent ready=true.

## Appendix C — Arch->Fedora map
hyprland/lock/idle/paper official, DELETE qt6-5compat/quickshell->gtk4/layer-shell, brightnessctl same sysfs-fallback + video group, google-material-symbols + noto-emoji-color + cascadia-code-nf + rsms-inter + jetbrains-mono, cliphist ImageMagick playerctl jq foot starship fish (COPR note matugen), NetworkManager-wifi bluez-tools, power-profiles-daemon NOT tlp, mesa intel-media libva.

## Appendix D — Dimmer quick ref
```bash
sudo battery-dimmer.sh --status
cat /etc/battery-dimmer.env
journalctl -u battery-dimmer -f
```
Math: orig*70/100 then *70/100 Stevens = ~49% orig = -30% perceived. Dims <=50 restores >53. Manual overrides till AC.
