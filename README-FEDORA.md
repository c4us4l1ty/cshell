# cshell — Rust + GTK4 Hyprland shell (Fedora Minimal, same UI)

Pixel-faithful Rust port of sshell (QML) for Fedora Minimal -> Hyprland on
Lenovo IdeaPad Slim 3 14IRH10. Event-driven, zero polling, 100ms fade only,
near-0 idle CPU, max battery, hardened.

## TTY one-shot (Fedora Minimal ISO -> TTY login)
```bash
git clone https://github.com/c4us4l1ty/cshell
cd cshell
chmod +x ./fedora-setup.sh
sudo ./fedora-setup.sh --yes
sudo reboot
# tuigreet -> Hyprland + sshell-rs bar + battery-dimmer (50% -> -30%)
systemctl is-active battery-dimmer power-profiles-daemon
battery-dimmer.sh --status
sshell-rs --check
```

## What runs
- `sshell-rs` (Rust GTK4 layer-shell): top bar h=38 floating (Launcher/Workspaces/Mpris |
  Clock/Weather | Battery/Tray), launcher 400x500, control-center 450 right,
  notifications top-right max5/group3/5s, OSD top-center 1500ms, session screen.
  Clock minute-aligned; battery piggybacks same tick (no 5s poll). Sliders write
  sysfs/wpctl once per gesture (no double-step).
- `battery-dimmer.sh` (root daemon): dims ~30% at <=50%, restores >53%.
  `cat /etc/battery-dimmer.env`, `journalctl -u battery-dimmer -f`.
- Hyprland: blur off, animations off (fade-only), single-handler keybinds
  (`sshell-rs brightness-up/down volume-up/down mute mpris ...` with
  brightnessctl/wpctl/playerctl fallback).

## Keybinds
Super+Space launcher, Super+N control-center, Super+V clipboard, Super+I settings,
Ctrl+Alt+Del session, Super+X shell toggle, Super+W wallpaper, XF86 keys via sshell-rs.

## Verify (no VM)
```bash
bash -n fedora-setup.sh && bash -n Plan/battery-dimmer.sh
DRY_RUN=1 ./fedora-setup.sh --yes | head -60
cd sshell-rs && cargo test
sshell-rs --check
```

## Uninstall / rollback
```bash
sudo ./fedora-setup.sh --uninstall   # restores ~/.local/state/sshell/backups/latest
```

## Layout
`fedora-setup.sh`, `sshell-rs/` (Rust), `configs/hypr/hyprland/*`, `config.jsonc`,
`Plan/battery-dimmer.sh`, `Plan/Plan.md`, `TTY-CHECKLIST.md`.
Legacy QML (`shell.qml`, `services/`, `components/`, `installer.sh`) stays during
transition; Hyprland prefers `sshell-rs`, falls back to `qs -c sshell`.
