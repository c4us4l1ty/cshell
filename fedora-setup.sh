#!/usr/bin/env bash
# cshell fedora-setup.sh — one-shot TTY bootstrap for Fedora Minimal -> Hyprland + Rust shell
# Usage (from TTY after Minimal install):
#   git clone https://github.com/c4us4l1ty/cshell
#   cd cshell
#   chmod +x ./fedora-setup.sh
#   sudo ./fedora-setup.sh --yes
# Idempotent: safe to re-run. Logs to /var/log/cshell-setup.log
set -euo pipefail

VERSION="2.0.0-rs"
LOG="/var/log/cshell-setup.log"
ASSUME_YES=0
DRY_RUN="${DRY_RUN:-0}"
UNINSTALL=0

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONFIG_SRC="$PROJECT_ROOT/configs"
DIMMER_SRC="$PROJECT_ROOT/Plan/battery-dimmer.sh"
RUST_SRC="$PROJECT_ROOT/sshell-rs"

# Resolve real user when run via sudo (TTY flow uses sudo)
if [[ -n "${SUDO_USER:-}" ]]; then
  TARGET_USER="$SUDO_USER"
  TARGET_HOME="$(getent passwd "$SUDO_USER" | cut -d: -f6)"
else
  TARGET_USER="$(whoami)"
  TARGET_HOME="$HOME"
fi
: "${TARGET_HOME:?TARGET_HOME empty}"

BACKUP_ROOT="$TARGET_HOME/.local/state/sshell/backups"
TIMESTAMP="$(date +%Y%m%d-%H%M%S)"
BACKUP_DIR="$BACKUP_ROOT/$TIMESTAMP"
DONE_MARKER="/var/lib/cshell-setup.done"
ENV_FILE="/etc/battery-dimmer.env"
DIMMER_BIN="/usr/local/bin/battery-dimmer.sh"

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[0;33m'; BLUE='\033[0;34m'
MAGENTA='\033[0;35m'; CYAN='\033[0;36m'; BOLD='\033[1m'; DIM='\033[2m'; NC='\033[0m'

log()  { echo -e "${BLUE}${BOLD}→${NC} $*" | tee -a "$LOG"; }
ok()   { echo -e "${GREEN}${BOLD}✓${NC} $*" | tee -a "$LOG"; }
warn() { echo -e "${YELLOW}${BOLD}!${NC} $*" | tee -a "$LOG"; }
dim()  { echo -e "${DIM}  $*${NC}"; }
die()  { echo -e "${RED}${BOLD}✗${NC} $*" | tee -a "$LOG" >&2; exit 1; }

run() {
  if [[ "$DRY_RUN" == "1" ]]; then
    echo "[DRY_RUN] $*"
    return 0
  fi
  "$@"
}

run_as_user() {
  if [[ "$DRY_RUN" == "1" ]]; then
    echo "[DRY_RUN as $TARGET_USER] $*"
    return 0
  fi
  if [[ "$(whoami)" == "$TARGET_USER" ]]; then
    "$@"
  else
    sudo -u "$TARGET_USER" "$@"
  fi
}

banner() {
  echo -e "${CYAN}"
  cat <<'EOF'
               █             ▀▀█    ▀▀█
  ▄▄▄    ▄▄▄   █ ▄▄    ▄▄▄     █      █
 █   ▀  █   ▀  █▀  █  █▀  █    █      █
  ▀▀▀▄   ▀▀▀▄  █   █  █▀▀▀▀    █      █
 ▀▄▄▄▀  ▀▄▄▄▀  █   █  ▀█▄▄▀    ▀▄▄    ▀▄▄
EOF
  echo -e "${NC}${DIM} cshell fedora-setup  v$VERSION  (Rust + GTK4, Fedora Minimal -> Hyprland)${NC}"
}

usage() {
  banner
  cat <<EOF
$(echo -e "${BOLD}Usage:${NC}") sudo ./fedora-setup.sh [--yes] [--uninstall] [--help]

$(echo -e "${BOLD}TTY one-shot:${NC}")
  git clone https://github.com/c4us4l1ty/cshell
  cd cshell
  chmod +x ./fedora-setup.sh
  sudo ./fedora-setup.sh --yes
  sudo reboot

$(echo -e "${BOLD}What it does:${NC}")
  1. dnf install Hyprland + GTK4 + portals + audio/net/bt + PPD + mesa + fonts
  2. backup ~/.config/hypr,gtk-3.0,gtk-4.0,sshell to $BACKUP_ROOT/<ts> + latest
  3. deploy configs/hypr + gtk + config.jsonc (same UI tokens)
  4. install Plan/battery-dimmer.sh -> $DIMMER_BIN with THRESHOLD=50 DIM=30 (works off-bat)
  5. build + install sshell-rs (Rust GTK4 bar, event-driven, fade-only)
  6. enable greetd->Hyprland, NetworkManager, bluetooth, PPD, dimmer; grubby powersave args
  7. fc-cache + verifications

$(echo -e "${BOLD}Flags:${NC}")
  --yes         skip confirm prompt (required for TTY one-shot)
  --uninstall   restore latest backup, disable dimmer/greetd (packages kept)
  --help        this message
  DRY_RUN=1     print without executing (e.g. DRY_RUN=1 sudo ./fedora-setup.sh --yes)
EOF
}

confirm() {
  if [[ "$ASSUME_YES" == "1" ]]; then return 0; fi
  local ans=""
  read -rp "$(echo -e "${MAGENTA}${BOLD}?${NC} $1 ${DIM}[y/N]${NC} ")" ans || true
  [[ "$ans" =~ ^[Yy]$ ]]
}

mark_done() { if [[ "$DRY_RUN" == "1" ]]; then echo "[DRY_RUN] mark_done $1"; return 0; fi; mkdir -p "$(dirname "$DONE_MARKER")"; echo "$1 $(date -Is)" >> "$DONE_MARKER"; }
already_done() { [[ -f "$DONE_MARKER" ]] && grep -q "^$1 " "$DONE_MARKER"; }

DNF_BIN=""
detect_dnf() {
  if command -v dnf5 &>/dev/null; then DNF_BIN="dnf5";
  elif command -v dnf &>/dev/null; then DNF_BIN="dnf";
  else die "dnf not found. Run on Fedora Minimal."; fi
}

# Fedora 44 package map (no Arch names, no quickshell/Qt6-shell)
BASE_PKGS=(git curl wget rsync tar xz gcc clang pkgconf-pkg-config openssl-devel make patch)
HYPR_PKGS=(hyprland hyprlock hypridle hyprpaper hyprshot hyprpicker xdg-desktop-portal-hyprland xdg-desktop-portal-gtk greetd tuigreet)
GTK_PKGS=(gtk4 gtk4-devel gtk4-layer-shell gtk4-layer-shell-devel gobject-introspection gsettings-desktop-schemas)
NET_PKGS=(NetworkManager-wifi iw wpa_supplicant bluez bluez-tools)
AUDIO_PKGS=(pipewire pipewire-pulse pipewire-alsa wireplumber pamixer playerctl brightnessctl)
POWER_PKGS=(power-profiles-daemon powertop)
MESA_PKGS=(mesa-dri-drivers mesa-vulkan-drivers intel-media-driver libva-intel-driver)
FONT_PKGS=(cascadia-code-nf-fonts rsms-inter-fonts jetbrains-mono-fonts google-material-symbols-fonts google-noto-emoji-fonts)
UTIL_PKGS=(cliphist ImageMagick jq foot starship fish grim wl-clipboard slurp polkit-gnome gnome-keyring)
# matugen is not in Fedora repos as of F44; try dnf, fallback to cargo install
OPTIONAL_COPR_PKGS=(matugen)

ALL_PKGS=("${BASE_PKGS[@]}" "${HYPR_PKGS[@]}" "${GTK_PKGS[@]}" "${NET_PKGS[@]}" "${AUDIO_PKGS[@]}" "${POWER_PKGS[@]}" "${MESA_PKGS[@]}" "${FONT_PKGS[@]}" "${UTIL_PKGS[@]}")

install_packages() {
  log "Installing Fedora packages via $DNF_BIN (missing only, idempotent)..."
  local missing=()
  local p
  for p in "${ALL_PKGS[@]}"; do
    if ! rpm -q "$p" &>/dev/null; then missing+=("$p"); fi
  done
  if [[ "${#missing[@]}" -gt 0 ]]; then
    log "Installing ${#missing[@]} packages: ${missing[*]}"
    run "$DNF_BIN" install -y "${missing[@]}" || warn "Some packages failed; continuing (check $LOG)"
  else
    ok "All Fedora packages already installed"
  fi
  # matugen best-effort (themer; Rust falls back if absent)
  if ! command -v matugen &>/dev/null; then
    warn "matugen not in repos; trying COPR/cargo fallback (non-fatal)"
    run "$DNF_BIN" copr enable -y ehbello/matugen 2>/dev/null && run "$DNF_BIN" install -y matugen 2>/dev/null || true
    if ! command -v matugen &>/dev/null && command -v cargo &>/dev/null; then
      run_as_user cargo install matugen 2>/dev/null || true
    fi
  fi
  ok "Package step done"
}

ensure_rust() {
  if command -v cargo &>/dev/null && command -v rustc &>/dev/null; then
    ok "cargo $(cargo --version 2>/dev/null) present"
    return 0
  fi
  log "Installing Rust via rustup (stable, user $TARGET_USER)..."
  if [[ "$DRY_RUN" == "1" ]]; then echo "[DRY_RUN] rustup-init -y --default-toolchain stable"; return 0; fi
  run_as_user bash -c 'curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup-init.sh && sh /tmp/rustup-init.sh -y --default-toolchain stable --profile minimal && rm -f /tmp/rustup-init.sh'
  # shellcheck disable=SC1090
  export PATH="$TARGET_HOME/.cargo/bin:$PATH"
  command -v cargo &>/dev/null || die "cargo still missing after rustup"
  ok "Rust installed: $(cargo --version)"
}

backup_path() {
  local src="$1" name="$2"
  if [[ -e "$src" ]]; then
    local dst="$BACKUP_DIR/$name"
    run mkdir -p "$(dirname "$dst")"
    run cp -a "$src" "$dst"
    dim "Backed up $name"
  fi
}

backup_configs() {
  log "Backing up existing configs..."
  run mkdir -p "$BACKUP_DIR"
  for d in hypr gtk-3.0 gtk-4.0 sshell fish matugen; do
    backup_path "$TARGET_HOME/.config/$d" "configs/$d"
  done
  if [[ -f "$TARGET_HOME/.config/starship.toml" ]]; then
    backup_path "$TARGET_HOME/.config/starship.toml" "configs/starship.toml"
  fi
  # legacy quickshell path if present
  if [[ -d "$TARGET_HOME/.config/quickshell/sshell" ]]; then
    backup_path "$TARGET_HOME/.config/quickshell/sshell" "quickshell-sshell"
  fi
  if [[ "$DRY_RUN" != "1" ]]; then
    rm -f "$BACKUP_ROOT/latest"
    ln -s "$BACKUP_DIR" "$BACKUP_ROOT/latest"
    chown -R "$TARGET_USER:$(id -gn "$TARGET_USER")" "$BACKUP_ROOT" || true
  else
    echo "[DRY_RUN] ln -s $BACKUP_DIR $BACKUP_ROOT/latest"
  fi
  ok "Backup saved to $BACKUP_DIR"
}

deploy_configs() {
  log "Deploying cshell configs (same UI tokens)..."
  for d in hypr gtk-3.0 gtk-4.0 matugen fish; do
    if [[ -d "$CONFIG_SRC/$d" ]]; then
      run_as_user mkdir -p "$TARGET_HOME/.config/$d"
      run rsync -a "$CONFIG_SRC/$d/" "$TARGET_HOME/.config/$d/"
      dim "Installed $d"
    fi
  done
  if [[ -f "$CONFIG_SRC/starship.toml" ]]; then
    run cp -a "$CONFIG_SRC/starship.toml" "$TARGET_HOME/.config/starship.toml"
    run chown "$TARGET_USER:$(id -gn "$TARGET_USER")" "$TARGET_HOME/.config/starship.toml" || true
  fi
  # shell config.jsonc -> ~/.config/sshell/config.jsonc (same keys as QML Config)
  if [[ -f "$PROJECT_ROOT/config.jsonc" ]]; then
    run_as_user mkdir -p "$TARGET_HOME/.config/sshell"
    run cp -a "$PROJECT_ROOT/config.jsonc" "$TARGET_HOME/.config/sshell/config.jsonc"
    dim "Installed sshell/config.jsonc"
  fi
  run_as_user mkdir -p "$TARGET_HOME/.local/state/sshell/wallpaper" "$TARGET_HOME/.cache/sshell/thumbnails" "$TARGET_HOME/Pictures/wallpapers" "$TARGET_HOME/Pictures/gifs"
  # fix ownership (script runs as root)
  if [[ "$DRY_RUN" != "1" ]]; then
    chown -R "$TARGET_USER:$(id -gn "$TARGET_USER")" "$TARGET_HOME/.config/hypr" "$TARGET_HOME/.config/gtk-3.0" "$TARGET_HOME/.config/gtk-4.0" "$TARGET_HOME/.config/sshell" "$TARGET_HOME/.local/state/sshell" "$TARGET_HOME/.cache/sshell" 2>/dev/null || true
    restorecon -Rv "$TARGET_HOME/.config" 2>/dev/null || true
  fi
  ok "Configs deployed"
}

install_dimmer() {
  log "Installing battery-dimmer (50% -> -30%, works off-bat)..."
  [[ -f "$DIMMER_SRC" ]] || die "Missing $DIMMER_SRC"
  run install -m 0755 "$DIMMER_SRC" "$DIMMER_BIN"
  if [[ "$DRY_RUN" == "1" ]]; then
    echo "[DRY_RUN] write $ENV_FILE THRESHOLD=50 HYSTERESIS=3 DIM_BY_PERCENT=30 MIN_PERCENT=12 BASE_POLL_INTERVAL=60"
  else
    cat > "$ENV_FILE" <<'EOF'
# cshell battery-dimmer defaults — dim 30% at 50% battery, restore at 53%
THRESHOLD=50
HYSTERESIS=3
DIM_BY_PERCENT=30
MIN_PERCENT=12
BASE_POLL_INTERVAL=60
SUSPEND_THRESHOLD=10
LOG_THROTTLE=30
RESCAN_EVERY=10
UDEV_DEBOUNCE_MS=150
EOF
    chmod 644 "$ENV_FILE"
  fi
  # Ensure systemd unit picks up EnvironmentFile + journal (patch if upstream unit lacks it)
  run "$DIMMER_BIN" --install-udev || warn "udev install reported issue"
  run "$DIMMER_BIN" --install-service || warn "service install reported issue"
  if [[ "$DRY_RUN" != "1" ]]; then
    local unit="/etc/systemd/system/battery-dimmer.service"
    if [[ -f "$unit" ]] && ! grep -q "EnvironmentFile=$ENV_FILE" "$unit"; then
      # Insert EnvironmentFile + journal output under [Service]
      awk -v envf="$ENV_FILE" '
        /^\[Service\]/ {print; print "EnvironmentFile=" envf; print "StandardOutput=journal"; print "StandardError=journal"; next}
        {print}
      ' "$unit" > "$unit.tmp" && mv -f "$unit.tmp" "$unit"
      systemctl daemon-reload || true
      systemctl restart battery-dimmer.service || true
      dim "Patched $unit with EnvironmentFile + journal"
    fi
    systemctl enable --now battery-dimmer.service || warn "could not start battery-dimmer"
    systemctl enable --now power-profiles-daemon NetworkManager bluetooth 2>/dev/null || true
  else
    echo "[DRY_RUN] patch /etc/systemd/system/battery-dimmer.service EnvironmentFile + journal; enable dimmer/PPD/NM/bt"
  fi
  ok "Dimmer installed (THRESHOLD=50 DIM_BY_PERCENT=30)"
}

build_rust_shell() {
  if [[ ! -d "$RUST_SRC" ]]; then
    warn "No $RUST_SRC yet (Rust port not scaffolded) — skipping build, QML configs remain for transition"
    return 0
  fi
  log "Building sshell-rs (Rust + GTK4, release)..."
  if [[ "$DRY_RUN" == "1" ]]; then echo "[DRY_RUN] cargo build --locked --release in $RUST_SRC"; return 0; fi
  export PATH="$TARGET_HOME/.cargo/bin:/usr/local/cargo/bin:$PATH"
  run_as_user bash -lc "cd '$RUST_SRC' && cargo build --locked --release"
  run install -m 0755 "$RUST_SRC/target/release/sshell-rs" /usr/local/bin/sshell-rs
  # user service for bar (graphical session)
  local svc_dir="$TARGET_HOME/.config/systemd/user"
  run_as_user mkdir -p "$svc_dir"
  if [[ "$DRY_RUN" != "1" ]]; then
    cat > "$svc_dir/sshell-rs.service" <<'EOF'
[Unit]
Description=cshell Rust GTK4 shell bar
PartOf=graphical-session.target
After=graphical-session.target

[Service]
Type=simple
ExecStart=/usr/local/bin/sshell-rs
Restart=on-failure
RestartSec=2

[Install]
WantedBy=graphical-session.target
EOF
    chown "$TARGET_USER:$(id -gn "$TARGET_USER")" "$svc_dir/sshell-rs.service"
  fi
  ok "sshell-rs installed to /usr/local/bin/sshell-rs"
}

system_tuning() {
  log "Boot target + powersave tuning (Intel Raptor Lake-P + i915)..."
  run systemctl set-default graphical.target || true
  # greetd -> Hyprland for target user (lighter than SDDM, Minimal-friendly)
  if [[ "$DRY_RUN" != "1" ]]; then
    mkdir -p /etc/greetd
    cat > /etc/greetd/config.toml <<EOF
[terminal]
vt = 1

[default_session]
command = "tuigreet --time --remember --asterisks --user-menu --cmd Hyprland"
user = "greeter"
EOF
    # allow greeter to launch Hyprland as target user via tuigreet session
    systemctl enable greetd 2>/dev/null || warn "greetd enable failed"
  else
    echo "[DRY_RUN] write /etc/greetd/config.toml tuigreet --cmd Hyprland; enable greetd"
  fi
  # kernel powersave args (deep sleep + PSR/FBC + NVMe latency); idempotent via grubby
  if command -v grubby &>/dev/null; then
    run grubby --update-kernel=ALL --args="mem_sleep_default=deep i915.enable_psr=1 i915.enable_fbc=1 nvme_core.default_ps_max_latency_us=5500 iwlwifi.power_save=1 btusb.enable_autosuspend=1 snd_hda_intel.power_save=1" || warn "grubby args failed"
  else
    warn "grubby missing; skipping kernel args (install grubby to enable deep/PSR)"
  fi
  # powertop autotune via udev (not cron) — minimal rule
  if [[ "$DRY_RUN" != "1" ]]; then
    cat > /etc/udev/rules.d/99-cshell-powertop.rules <<'EOF'
# cshell: USB autosuspend + powertop-style tunables on battery-friendly defaults
ACTION=="add", SUBSYSTEM=="usb", TEST=="power/control", ATTR{power/control}="auto"
ACTION=="add", SUBSYSTEM=="pci", TEST=="power/control", ATTR{power/control}="auto"
EOF
    udevadm control --reload-rules 2>/dev/null || true
  else
    echo "[DRY_RUN] write /etc/udev/rules.d/99-cshell-powertop.rules"
  fi
  # fonts + verification (non-fatal)
  run fc-cache -f 2>/dev/null || true
  ok "System tuning done"
}

verify_install() {
  log "Verifying..."
  local fail=0
  command -v Hyprland &>/dev/null || { warn "Hyprland missing"; fail=1; }
  command -v "$DIMMER_BIN" &>/dev/null || { warn "battery-dimmer missing"; fail=1; }
  [[ -f "$ENV_FILE" ]] || { warn "$ENV_FILE missing"; fail=1; }
  if [[ -f "$ENV_FILE" ]]; then
    grep -q "^THRESHOLD=50" "$ENV_FILE" || { warn "THRESHOLD!=50"; fail=1; }
    grep -q "^DIM_BY_PERCENT=30" "$ENV_FILE" || { warn "DIM_BY_PERCENT!=30"; fail=1; }
  fi
  if command -v fc-match &>/dev/null; then
    fc-match "CaskaydiaCove Nerd Font" &>/dev/null || warn "CaskaydiaCove Nerd Font not resolved"
    fc-match "Inter" &>/dev/null || warn "Inter not resolved"
  fi
  if [[ -x /usr/local/bin/sshell-rs ]]; then
    /usr/local/bin/sshell-rs --check 2>/dev/null || warn "sshell-rs --check reported issue (see journal)"
  else
    dim "sshell-rs binary not yet installed (transition: Hyprland will use qs fallback if present)"
  fi
  if [[ "$DRY_RUN" != "1" ]]; then
    "$DIMMER_BIN" --status 2>/dev/null | tee -a "$LOG" || true
  fi
  if [[ "$fail" == "1" ]]; then warn "Verification had warnings (see above)"; else ok "Verification passed"; fi
}

do_uninstall() {
  banner
  warn "Uninstall: restore latest backup, disable dimmer/greetd (packages kept)."
  confirm "Continue with uninstall?" || exit 0
  local src="$BACKUP_ROOT/latest"
  [[ -d "$src" ]] || die "No backup at $src"
  log "Restoring from $(readlink -f "$src")"
  for d in hypr gtk-3.0 gtk-4.0 sshell fish matugen; do
    if [[ -d "$src/configs/$d" ]]; then
      run rm -rf "${TARGET_HOME:?}/.config/$d"
      run cp -a "$src/configs/$d" "$TARGET_HOME/.config/$d"
      dim "Restored $d"
    fi
  done
  if [[ "$DIMMER_BIN" == "/usr/local/bin/battery-dimmer.sh" ]]; then
    run "$DIMMER_BIN" --remove-service 2>/dev/null || true
    run "$DIMMER_BIN" --remove-udev 2>/dev/null || true
  fi
  run systemctl disable greetd 2>/dev/null || true
  run rm -f /usr/local/bin/sshell-rs 2>/dev/null || true
  ok "Uninstall complete. Packages NOT removed."
}

main() {
  for a in "$@"; do
    case "$a" in
      --yes) ASSUME_YES=1 ;;
      --uninstall) UNINSTALL=1 ;;
      --help|-h) usage; exit 0 ;;
      *) die "Unknown arg $a (see --help)" ;;
    esac
  done
  mkdir -p "$(dirname "$LOG")"
  touch "$LOG" 2>/dev/null || LOG="/tmp/cshell-setup.log"
  banner | tee -a "$LOG"
  if [[ $EUID -ne 0 && "$DRY_RUN" != "1" ]]; then die "Run with sudo: sudo ./fedora-setup.sh --yes"; fi
  if [[ $EUID -ne 0 && "$DRY_RUN" == "1" ]]; then warn "DRY_RUN without root: system writes skipped, commands printed only"; fi
  detect_dnf
  if [[ "$UNINSTALL" == "1" ]]; then do_uninstall; exit 0; fi
  log "Target user: $TARGET_USER ($TARGET_HOME) | repo: $PROJECT_ROOT"
  if [[ "$ASSUME_YES" != "1" ]]; then
    echo ""
    log "Will: dnf packages, backup->deploy configs, dimmer 50/30, build sshell-rs, greetd+grubby tuning."
    confirm "Continue?" || exit 0
  fi
  install_packages
  ensure_rust
  backup_configs
  deploy_configs
  install_dimmer
  build_rust_shell
  system_tuning
  verify_install
  mark_done "install"
  echo ""
  echo -e "${GREEN}${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}" | tee -a "$LOG"
  ok "Install complete! Reboot now: sudo reboot"
  dim "After reboot: tuigreet -> Hyprland; check: systemctl is-active battery-dimmer; battery-dimmer.sh --status" | tee -a "$LOG"
  dim "Backups: $BACKUP_ROOT (latest symlink); uninstall: sudo ./fedora-setup.sh --uninstall" | tee -a "$LOG"
  dim "Log: $LOG" | tee -a "$LOG"
  echo -e "${GREEN}${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}" | tee -a "$LOG"
}

main "$@"
