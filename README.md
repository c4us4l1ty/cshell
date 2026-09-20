<div align="center">
<pre>
 ▗▄▄▖     ▗▄▄▖▗▖ ▗▖▗▄▄▄▖▗▖   ▗▖   
▐▌       ▐▌   ▐▌ ▐▌▐▌   ▐▌   ▐▌   
▐▌        ▝▀▚▖▐▛▀▜▌▐▛▀▀▘▐▌   ▐▌   
▝▚▄▄▖    ▗▄▄▞▘▐▌ ▐▌▐▙▄▄▖▐▙▄▄▖▐▙▄▄▖
cshell — Rust + GTK4 rewrite          v2.0
</pre>
</div>

> **This repo is now the Rust + GTK4 rewrite (same UI, event-driven, zero polling).
> Start here: [`README-FEDORA.md`](README-FEDORA.md) → `sudo ./fedora-setup.sh --yes`.**

## Install (Fedora Minimal → Hyprland, TTY one-shot)

```bash
# 1. Update Fedora
sudo dnf upgrade --refresh -y

# 2. Reboot into the updated system
sudo reboot

# 3. Back in TTY, install only git
sudo dnf install git -y

# 4. Get your setup
git clone https://github.com/c4us4l1ty/cshell
cd cshell

# 5. Run your installer
chmod +x fedora-setup.sh
sudo ./fedora-setup.sh --yes

# 6. Reboot
sudo reboot
```

Details: [`README-FEDORA.md`](README-FEDORA.md) · checklist: [`TTY-CHECKLIST.md`](TTY-CHECKLIST.md) · plan: [`Plan/Plan.md`](Plan/Plan.md)

## What runs

- `sshell-rs/` — Rust GTK4 layer-shell shell (bar, launcher, control-center,
  notifications, OSD, session, wallpaper selector, settings, tray). Same layout
  tokens as the old QML shell. `sshell-rs --check` validates without a GUI.
- `Plan/battery-dimmer.sh` — zero-fork backlight daemon: dims 30% at ≤50%
  battery, restores above 53%. Installed + enabled by `fedora-setup.sh`.
- `configs/hypr/hyprland/` — Hyprland config (blur/animations off, single-handler
  `sshell-rs` keybinds). `configs/matugen/`, `config.jsonc` — theme + shell config.

## Legacy QML (retired)

`shell.qml`, `services/`, `components/`, `settings/`, `installer.sh` (Arch-only)
are the retired Quickshell implementation, kept for reference. Nothing binds to
them anymore: all Hyprland keybinds call `sshell-rs`, `execs.conf` prefers the
Rust binary. Do not extend the QML tree — port to `sshell-rs/` instead.

## Screenshots (QML era, UI unchanged in Rust port)

<img width="1920" height="1080" alt="gumi-from-megpoid" src="https://github.com/user-attachments/assets/35e78cd6-c7d4-479f-8022-2d2ce2fd6d9f" />
<img width="1920" height="1080" alt="notification-center" src="https://github.com/user-attachments/assets/155e819b-a3c5-40aa-aea7-8e919d3d0ed5" />
<img width="1920" height="1080" alt="settings" src="https://github.com/user-attachments/assets/117d58f9-aab4-473c-8f19-d5531167b50f" />
<img width="1920" height="1080" alt="session-control" src="https://github.com/user-attachments/assets/4538d4d6-2323-4547-a427-625356bac72e" />

## Wallpaper

[Train Sideview](https://raw.githubusercontent.com/orangci/walls-catppuccin-mocha/master/train-sideview.png) from walls-catpuccin-mocha by orangci
