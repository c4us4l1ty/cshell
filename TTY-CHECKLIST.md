# TTY Checklist — Fedora Minimal -> cshell (no VM, flawless first boot)

Read aloud in TTY. Do not skip. Expected output noted per step.

## 0. Before reboot to ISO
- [ ] `sha256sum -c *-CHECKSUM` => OK
- [ ] Backup: `cp -a ~/Downloads/sshell-main ~/sshell-backup-$(date +%Y%m%d)` done
- [ ] Repo pushed: `git ls-remote https://github.com/c4us4l1ty/cshell` succeeds

## 1. Fedora Minimal install (Anaconda)
- [ ] Keep `/dev/nvme0n1p1` EFI mount `/boot/efi` DO NOT FORMAT
- [ ] BTRFS `/`, hostname `von-fedora`, user `von` + wheel, Minimal software, sshd off
- [ ] First TTY login as `von`

## 2. One-shot (copy-paste)
```bash
git clone https://github.com/c4us4l1ty/cshell
cd cshell
chmod +x ./fedora-setup.sh
sudo ./fedora-setup.sh --yes
sudo reboot
```
- [ ] `sudo ./fedora-setup.sh --yes` ends with `Install complete! Reboot now`
- [ ] Log exists: `cat /var/log/cshell-setup.log | tail -20` no `✗`

## 3. First boot verify (5 min)
```bash
systemctl is-active battery-dimmer power-profiles-daemon NetworkManager bluetooth
battery-dimmer.sh --status
cat /etc/battery-dimmer.env
journalctl -u battery-dimmer -n 20
echo $XDG_CURRENT_DESKTOP
hyprctl monitors
```
- [ ] All services `active`
- [ ] `--status` shows `intel_backlight` + `BAT0`
- [ ] ENV shows `THRESHOLD=50` + `DIM_BY_PERCENT=30`
- [ ] Journal shows `Threshold: 50% (+3% Hysteresis), Dim: 30%`
- [ ] `XDG_CURRENT_DESKTOP=Hyprland`, eDP-1 1920x1200@60

## 4. UI accept (10 min, same UI as QML)
- [ ] Bar visible top floating h=38, Launcher/Workspaces/Mpris left, Clock/Weather center, Battery/Tray right
- [ ] `Super+Space` launcher <200ms, Esc clears-then-closes
- [ ] `Super+N` control center 450px right, sliders work
- [ ] `Super+V` clipboard, `Super+I` settings, `Ctrl+Alt+Del` session
- [ ] Volume/brightness keys OSD once (not double-step), `notify-send hi` appears
- [ ] Wallpaper selector works, theme recolors

## 5. Battery dim test
- [ ] Unplug AC at <=49% => brightness drops ~30% within 60s
- [ ] Plug AC or charge to >53% => restores
- [ ] `journalctl -u battery-dimmer -f` shows Dimmed/Restored lines

## 6. Fail? TTY rescue
- [ ] `Ctrl+Alt+F3`, login, `cat /var/log/cshell-setup.log`, `journalctl -u greetd -b`, `journalctl --user -u sshell-rs -b`
- [ ] Rollback: `sudo ./fedora-setup.sh --uninstall` restores `~/.local/state/sshell/backups/latest`

## Dry-run rehearsal (on Cinnamon before wipe)
```bash
DRY_RUN=1 sudo ./fedora-setup.sh --yes | tee /tmp/dryrun.log
grep -q "THRESHOLD=50" /tmp/dryrun.log
bash -n fedora-setup.sh && bash -n Plan/battery-dimmer.sh && echo OK
sshell-rs --check (after cargo build)
```
