#!/usr/bin/env bash
# cshell fedora-setup.sh — one-shot TTY bootstrap for Fedora Minimal -> Hyprland + Rust shell
set -euo pipefail

VERSION="2.0.0-rs"
LOG="/var/log/cshell-setup.log"
ASSUME_YES=0
DRY_RUN="${DRY_RUN:-0}"
UNINSTALL=0

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
CONFIG_SRC="$PROJECT_ROOT/configs"
DIMMER_SRC="$PROJECT_ROOT/Plan/battery-dimmer.sh"
RUST_SRC="$PROJECT_ROOT/sshell-rs"

# Resolve real user when run via sudo
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
CYAN='\033[0;36m'; BOLD='\033[1m'; DIM='\033[2m'; NC='\033[0m'

log()  { echo -e "${BLUE}${BOLD}→${NC} $*" | tee -a "$LOG"; }
ok()   { echo -e "${GREEN}${BOLD}✓${NC} $*" | tee -a "$LOG"; }
warn() { echo -e "${YELLOW}${BOLD}!${NC} $*" | tee -a "$LOG"; }
dim()  { echo -e "${DIM}  $*${NC}"; }
die()  { echo -e "${RED}${BOLD}✗${NC} $*" | tee -a "$LOG" >&2; exit 1; }

run() {
  if [[ "$DRY_RUN" == "1" ]]; then echo "[DRY_RUN] $*"; return 0; fi
  "$@"
}

run_as_user() {
  if [[ "$DRY_RUN" == "1" ]]; then echo "[DRY_RUN as $TARGET_USER] $*"; return 0; fi
  if [[ "$(whoami)" == "$TARGET_USER" ]]; then "$@"; else sudo -u "$TARGET_USER" "$@"; fi
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

setup_repos() {
  log "Enabling required COPR repositories..."
  # Download repo file directly to avoid DNF5 chroot syntax mismatches on Rawhide
  curl -sL https://copr.fedorainfracloud.org/coprs/solopasha/hyprland/repo/fedora-rawhide/solopasha-hyprland-fedora-rawhide.repo \
    -o /etc/yum.repos.d/_copr_solopasha-hyprland.repo

  curl -sL https://copr.fedorainfracloud.org/coprs/atim/starship/repo/fedora-rawhide/atim-starship-fedora-rawhide.repo \
    -o /etc/yum.repos.d/_copr_atim-starship.repo 2>/dev/null || true
}

install_packages() {
  log "Installing base build tools..."
  run dnf install -y gcc gcc-c++ clang pkgconf-pkg-config openssl-devel make patch git curl wget

  local HYPR_PKGS=(hyprland hyprlock hypridle hyprpaper hyprshot hyprpicker xdg-desktop-portal-hyprland xdg-desktop-portal-gtk greetd tuigreet mate-polkit)
  local GTK_PKGS=(gtk4 gtk4-devel gtk4-layer-shell gtk4-layer-shell-devel gobject-introspection gsettings-desktop-schemas)
  local NET_PKGS=(NetworkManager-wifi iw wpa_supplicant bluez bluez-tools)
  local AUDIO_PKGS=(pipewire pipewire-pulseaudio pipewire-alsa wireplumber pamixer playerctl brightnessctl)
  local POWER_PKGS=(tuned-ppd powertop)
  local MESA_PKGS=(mesa-dri-drivers mesa-vulkan-drivers)
  local FONT_PKGS=(cascadia-code-nf-fonts rsms-inter-fonts jetbrains-mono-fonts google-noto-emoji-fonts)
  local UTIL_PKGS=(cliphist ImageMagick jq foot starship fish grim wl-clipboard slurp gnome-keyring matugen libnotify)

  local ALL_PKGS=("${HYPR_PKGS[@]}" "${GTK_PKGS[@]}" "${NET_PKGS[@]}" "${AUDIO_PKGS[@]}" "${POWER_PKGS[@]}" "${MESA_PKGS[@]}" "${FONT_PKGS[@]}" "${UTIL_PKGS[@]}")

  log "Installing desktop stack via dnf..."
  run dnf install -y --skip-unavailable --skip-broken "${ALL_PKGS[@]}"
  ok "Packages installed successfully"
}

ensure_rust() {
  log "Checking Rust environment..."
  run_as_user bash -lc "source \"\$HOME/.cargo/env\" 2>/dev/null || true; rustup default stable || true"
  ok "Rust ready: $(run_as_user bash -lc "source \"\$HOME/.cargo/env\" 2>/dev/null; rustc --version || true")"
}

fix_user_permissions() {
  if [[ "$DRY_RUN" != "1" ]]; then
    run_as_user mkdir -p "$TARGET_HOME/.config" "$TARGET_HOME/.local/state" "$TARGET_HOME/.cache"
    chown -R "$TARGET_USER:$(id -gn "$TARGET_USER")" "$TARGET_HOME/.config" "$TARGET_HOME/.local" "$TARGET_HOME/.cache" 2>/dev/null || true
  fi
}

backup_configs() {
  log "Backing up existing configs..."
  fix_user_permissions
  run_as_user mkdir -p "$BACKUP_DIR"

  for d in hypr gtk-3.0 gtk-4.0 sshell fish matugen; do
    if [[ -e "$TARGET_HOME/.config/$d" ]]; then
      run_as_user mkdir -p "$BACKUP_DIR/configs"
      run_as_user cp -a "$TARGET_HOME/.config/$d" "$BACKUP_DIR/configs/$d"
      dim "Backed up $d"
    fi
  done

  if [[ -f "$TARGET_HOME/.config/starship.toml" ]]; then
    run_as_user cp -a "$TARGET_HOME/.config/starship.toml" "$BACKUP_DIR/configs/starship.toml"
  fi

  rm -f "$BACKUP_ROOT/latest"
  run_as_user ln -s "$BACKUP_DIR" "$BACKUP_ROOT/latest"
  ok "Backup saved to $BACKUP_DIR"
}

deploy_configs() {
  log "Deploying cshell configs..."
  for d in hypr gtk-3.0 gtk-4.0 matugen fish; do
    if [[ -d "$CONFIG_SRC/$d" ]]; then
      run_as_user mkdir -p "$TARGET_HOME/.config/$d"
      run rsync -a "$CONFIG_SRC/$d/" "$TARGET_HOME/.config/$d/"
      dim "Installed $d"
    fi
  done

  if [[ -f "$CONFIG_SRC/starship.toml" ]]; then
    run cp -a "$CONFIG_SRC/starship.toml" "$TARGET_HOME/.config/starship.toml"
  fi

  if [[ -f "$PROJECT_ROOT/config.jsonc" ]]; then
    run_as_user mkdir -p "$TARGET_HOME/.config/sshell"
    run cp -a "$PROJECT_ROOT/config.jsonc" "$TARGET_HOME/.config/sshell/config.jsonc"
    dim "Installed sshell/config.jsonc"
  fi

  run_as_user mkdir -p \
    "$TARGET_HOME/.local/state/sshell/wallpaper" \
    "$TARGET_HOME/.cache/sshell/thumbnails" \
    "$TARGET_HOME/Pictures/wallpapers" \
    "$TARGET_HOME/Pictures/gifs"

  fix_user_permissions
  if command -v restorecon &>/dev/null; then
    restorecon -Rv "$TARGET_HOME/.config" 2>/dev/null || true
  fi
  ok "Configs deployed"
}

install_dimmer() {
  log "Installing battery-dimmer..."
  [[ -f "$DIMMER_SRC" ]] || die "Missing $DIMMER_SRC"
  run install -m 0755 "$DIMMER_SRC" "$DIMMER_BIN"

  cat > "$ENV_FILE" <<'EOF'
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

  run "$DIMMER_BIN" --install-udev || warn "udev install reported issue"
  run "$DIMMER_BIN" --install-service || warn "service install reported issue"

  systemctl enable --now battery-dimmer.service 2>/dev/null || warn "Could not start battery-dimmer"
  systemctl enable --now tuned NetworkManager bluetooth 2>/dev/null || true
  ok "Dimmer active"
}

build_rust_shell() {
  if [[ ! -d "$RUST_SRC" || ! -f "$RUST_SRC/Cargo.toml" ]]; then
    warn "sshell-rs source missing at $RUST_SRC — skipping build"
    return 0
  fi

  log "Patching Cargo.toml and building sshell-rs..."
  # Remove nonexistent "journald" feature from tracing-subscriber
  sed -i -E 's/"journald",?//g; s/,\s*"journald"//g' "$RUST_SRC/Cargo.toml"

  run_as_user bash -lc "source \"\$HOME/.cargo/env\" && cd '$RUST_SRC' && cargo build --release"

  if [[ -f "$RUST_SRC/target/release/sshell-rs" ]]; then
    run install -m 0755 "$RUST_SRC/target/release/sshell-rs" /usr/local/bin/sshell-rs
    local svc_dir="$TARGET_HOME/.config/systemd/user"
    run_as_user mkdir -p "$svc_dir"
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
    ok "sshell-rs installed to /usr/local/bin/sshell-rs"
  fi
}

install_fonts() {
  # Material Symbols Rounded is NOT in Fedora repos; fetch variable TTF once
  # (best-effort, non-fatal — Nerd Font codepoints in the bar work regardless).
  log "Installing icon fonts (Material Symbols, best-effort)..."
  local fdir="$TARGET_HOME/.local/share/fonts"
  run_as_user mkdir -p "$fdir"
  local dst="$fdir/MaterialSymbolsRounded.ttf"
  if [[ ! -s "$dst" ]]; then
    run_as_user bash -c "curl -sSL --max-time 60 'https://github.com/google/material-design-icons/raw/master/variablefont/MaterialSymbolsRounded%5BFILL%2CGRAD%2Copsz%2Cwght%5D.ttf' -o '$dst' && test -s '$dst'" \
      && ok "Material Symbols installed" \
      || warn "Material Symbols download failed (offline?) — continuing"
  else
    ok "Material Symbols already present"
  fi
  run fc-cache -f "$fdir" 2>/dev/null || true
  run_as_user fc-match "Material Symbols Rounded" 2>/dev/null || warn "Material Symbols not resolved (cosmetic only)"
}

system_tuning() {
  log "Configuring greetd and power optimizations..."
  run systemctl set-default graphical.target || true

  mkdir -p /etc/greetd
  cat > /etc/greetd/config.toml <<EOF
[terminal]
vt = 1

[default_session]
command = "tuigreet --time --remember --asterisks --user-menu --cmd Hyprland"
user = "greeter"
EOF
  systemctl enable greetd 2>/dev/null || warn "Failed to enable greetd"

  if command -v grubby &>/dev/null; then
    run grubby --update-kernel=ALL --args="mem_sleep_default=deep i915.enable_psr=1 i915.enable_fbc=1 nvme_core.default_ps_max_latency_us=5500 iwlwifi.power_save=1 btusb.enable_autosuspend=1 snd_hda_intel.power_save=1" 2>/dev/null || true
  fi

  cat > /etc/udev/rules.d/99-cshell-powertop.rules <<'EOF'
ACTION=="add", SUBSYSTEM=="usb", TEST=="power/control", ATTR{power/control}="auto"
ACTION=="add", SUBSYSTEM=="pci", TEST=="power/control", ATTR{power/control}="auto"
EOF
  udevadm control --reload-rules 2>/dev/null || true
  fc-cache -f 2>/dev/null || true
  ok "System tuning applied"
}

verify_install() {
  log "Verifying setup..."
  local fail=0
  command -v Hyprland &>/dev/null || { warn "Hyprland missing"; fail=1; }
  command -v "$DIMMER_BIN" &>/dev/null || { warn "battery-dimmer missing"; fail=1; }
  if [[ "$fail" == "1" ]]; then
    warn "Verification finished with warnings."
  else
    ok "All critical components verified!"
  fi
}

main() {
  mkdir -p "$(dirname "$LOG")"
  banner | tee -a "$LOG"

  if [[ $EUID -ne 0 ]]; then
    die "Run as root: sudo ./fedora-setup.sh --yes"
  fi

  setup_repos
  install_packages
  install_fonts
  ensure_rust
  backup_configs
  deploy_configs
  install_dimmer
  build_rust_shell
  system_tuning
  verify_install

  echo ""
  echo -e "${GREEN}${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}" | tee -a "$LOG"
  ok "Installation Complete! Reboot now: sudo reboot"
  echo -e "${GREEN}${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}" | tee -a "$LOG"
}

main "$@"
