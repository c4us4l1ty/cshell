#!/usr/bin/env bash
# cshell fedora-setup.sh — one-shot TTY bootstrap for Fedora Minimal -> Hyprland + Rust shell
set -euo pipefail

VERSION="2.1.0-rs"
LOG="/var/log/cshell-setup.log"
ASSUME_YES=0
DRY_RUN="${DRY_RUN:-0}"
UNINSTALL=0

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
CONFIG_SRC="$PROJECT_ROOT/configs"
DIMMER_SRC="$PROJECT_ROOT/Plan/battery-dimmer.sh"
RUST_SRC="$PROJECT_ROOT/sshell-rs"

if [[ -n "${SUDO_USER:-}" ]]; then
  TARGET_USER="$SUDO_USER"
  TARGET_HOME="$(getent passwd "$SUDO_USER" | cut -d: -f6)"
else
  TARGET_USER="$(whoami)"
  TARGET_HOME="$HOME"
fi
: "${TARGET_HOME:?TARGET_HOME empty}"
TARGET_UID="$(id -u "$TARGET_USER" 2>/dev/null || echo 1000)"

BACKUP_ROOT="$TARGET_HOME/.local/state/sshell/backups"
TIMESTAMP="$(date +%Y%m%d-%H%M%S)"
BACKUP_DIR="$BACKUP_ROOT/$TIMESTAMP"
DONE_MARKER="/var/lib/cshell-setup.done"
ENV_FILE="/etc/battery-dimmer.env"
DIMMER_BIN="/usr/local/bin/battery-dimmer.sh"

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[0;33m'; BLUE='\033[0;34m'
CYAN='\033[0;36m'; BOLD='\033[1m'; DIM='\033[2m'; NC='\033[0m'

log()  { echo -e "${BLUE}${BOLD}->${NC} $*"; echo -e "-> $*" >>"$LOG" 2>/dev/null || true; }
ok()   { echo -e "${GREEN}${BOLD}OK${NC} $*"; echo -e "OK $*" >>"$LOG" 2>/dev/null || true; }
warn() { echo -e "${YELLOW}${BOLD}!${NC} $*" >&2; echo -e "! $*" >>"$LOG" 2>/dev/null || true; }
dim()  { echo -e "${DIM}  $*${NC}"; }
die()  { printf "%s\n" "X $*" >&2; printf "%s\n" "X $*" >>"$LOG" 2>/dev/null || true; exit 1; }

on_err() { die "Failed at line $1: $BASH_COMMAND (see $LOG)"; }
trap 'on_err $LINENO' ERR

run() {
  if [[ "$DRY_RUN" == "1" ]]; then echo "[DRY_RUN] $*"; return 0; fi
  "$@"
}

run_as_user() {
  if [[ "$DRY_RUN" == "1" ]]; then echo "[DRY_RUN as $TARGET_USER] $*"; return 0; fi
  if [[ "$(whoami)" == "$TARGET_USER" ]]; then "$@"; else sudo -u "$TARGET_USER" "$@"; fi
}

# Root-owned file writer honoring DRY_RUN. Usage: write_root_file DEST MODE <<'EOF'
write_root_file() {
  local dest="$1"; local mode="${2:-644}"
  if [[ "$DRY_RUN" == "1" ]]; then echo "[DRY_RUN write $dest mode $mode]"; cat > /dev/null; return 0; fi
  mkdir -p "$(dirname "$dest")"
  cat > "$dest"
  chmod "$mode" "$dest"
}

# User-owned file writer honoring DRY_RUN. Usage: write_user_file DEST <<'EOF'
write_user_file() {
  local dest="$1"
  if [[ "$DRY_RUN" == "1" ]]; then echo "[DRY_RUN write $dest as $TARGET_USER]"; cat > /dev/null; return 0; fi
  run_as_user mkdir -p "$(dirname "$dest")"
  run_as_user tee "$dest" > /dev/null
  chown "$TARGET_USER:$(id -gn "$TARGET_USER")" "$dest" 2>/dev/null || true
}

mark_done() {
  if [[ "$DRY_RUN" == "1" ]]; then return 0; fi
  mkdir -p "$(dirname "$DONE_MARKER")"
  echo "$1 $TIMESTAMP" >> "$DONE_MARKER"
}
step_done() { [[ -f "$DONE_MARKER" ]] && grep -q "^$1 " "$DONE_MARKER" 2>/dev/null; }

usage() {
  cat <<'EOF'
Usage: sudo ./fedora-setup.sh --yes [--dry-run] [--uninstall] [--help]
  --yes        non-interactive (required on TTY)
  --dry-run    print actions, write nothing to /etc or $HOME (also DRY_RUN=1)
  --uninstall  restore ~/.local/state/sshell/backups/latest, remove dimmer+sshell-rs
  --help       this help
Env: DRY_RUN=1 ./fedora-setup.sh --yes
EOF
}

parse_args() {
  for a in "$@"; do
    case "$a" in
      --yes|-y) ASSUME_YES=1 ;;
      --dry-run) DRY_RUN=1 ;;
      --uninstall) UNINSTALL=1 ;;
      --help|-h) usage; exit 0 ;;
      *) die "Unknown arg: $a (see --help)" ;;
    esac
  done
  if [[ "$DRY_RUN" == "1" ]]; then ASSUME_YES=1; fi
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
  local ver
  ver="$(rpm -E %fedora 2>/dev/null || echo 44)"
  ver="${ver//[^0-9]/}"
  [[ -z "$ver" ]] && ver=44
  log "Detected Fedora $ver"
  local hypr_repo="https://copr.fedorainfracloud.org/coprs/solopasha/hyprland/repo/fedora-${ver}/solopasha-hyprland-fedora-${ver}.repo"
  local star_repo="https://copr.fedorainfracloud.org/coprs/atim/starship/repo/fedora-${ver}/atim-starship-fedora-${ver}.repo"
  if [[ "$DRY_RUN" == "1" ]]; then
    echo "[DRY_RUN] curl -fSL $hypr_repo -o /etc/yum.repos.d/_copr_solopasha-hyprland.repo"
    echo "[DRY_RUN] curl -fSL $star_repo -o /etc/yum.repos.d/_copr_atim-starship.repo || true"
    echo "[DRY_RUN] dnf makecache"
    return 0
  fi
  if ! curl -fSL --max-time 60 "$hypr_repo" -o /etc/yum.repos.d/_copr_solopasha-hyprland.repo; then
    die "Could not fetch Hyprland COPR for Fedora $ver. Check network (ip a) and retry."
  fi
  curl -fSL --max-time 60 "$star_repo" -o /etc/yum.repos.d/_copr_atim-starship.repo 2>/dev/null || warn "starship COPR unavailable (starship falls back to dnf official, continuing)"
  run dnf makecache || warn "makecache had warnings (continuing)"
  mark_done repos
}

install_packages() {
  log "Installing base build tools..."
  run dnf install -y rsync gcc gcc-c++ clang pkgconf-pkg-config openssl-devel make patch git curl wget upower rustup 2>/dev/null \
    || run dnf install -y --skip-unavailable gcc gcc-c++ clang pkgconf-pkg-config openssl-devel make patch git curl wget rsync upower

  local HYPR_PKGS=(hyprland hyprlock hypridle hyprpaper hyprshot hyprpicker xdg-desktop-portal-hyprland xdg-desktop-portal-gtk greetd tuigreet polkit-gnome)
  local GTK_PKGS=(gtk4 gtk4-devel gtk4-layer-shell gtk4-layer-shell-devel gobject-introspection gsettings-desktop-schemas)
  local NET_PKGS=(NetworkManager-wifi iw wpa_supplicant bluez bluez-tools)
  local AUDIO_PKGS=(pipewire pipewire-pulseaudio pipewire-alsa wireplumber pamixer playerctl brightnessctl)
  local POWER_PKGS=(tuned-ppd powertop)
  local MESA_PKGS=(mesa-dri-drivers mesa-vulkan-drivers intel-media-driver libva-intel-driver)
  local FONT_PKGS=(cascadia-code-nf-fonts rsms-inter-fonts jetbrains-mono-fonts google-noto-emoji-color-fonts google-noto-emoji-fonts)
  local UTIL_PKGS=(cliphist ImageMagick jq foot starship fish grim wl-clipboard slurp gnome-keyring libnotify btop fastfetch)

  local ALL_PKGS=("${HYPR_PKGS[@]}" "${GTK_PKGS[@]}" "${NET_PKGS[@]}" "${AUDIO_PKGS[@]}" "${POWER_PKGS[@]}" "${MESA_PKGS[@]}" "${FONT_PKGS[@]}" "${UTIL_PKGS[@]}")

  log "Installing desktop stack via dnf (matugen via COPR fallback if missing)..."
  run dnf install -y --skip-unavailable --skip-broken "${ALL_PKGS[@]}"
  if ! rpm -q matugen &>/dev/null; then
    warn "matugen not in repos — wallpaper theming will be best-effort. Install manually later: cargo install matugen (optional)."
  fi
  if ! rpm -q hyprland &>/dev/null && [[ "$DRY_RUN" != "1" ]]; then
    die "hyprland install failed (COPR for this Fedora version?). See $LOG."
  fi
  run fc-cache -f 2>/dev/null || true
  ok "Packages installed successfully"
  mark_done pkgs
}

ensure_rust() {
  log "Checking Rust environment..."
  local cargo_env="$TARGET_HOME/.cargo/env"
  if ! run_as_user bash -lc "command -v cargo >/dev/null && command -v rustc >/dev/null"; then
    log "Rust not found for $TARGET_USER — installing via rustup-init (minimal profile)..."
    if [[ "$DRY_RUN" == "1" ]]; then
      echo "[DRY_RUN as $TARGET_USER] rustup-init -y --profile minimal --default-toolchain stable"
    else
      run_as_user bash -lc "curl -fSL --max-time 120 --proto '=https' --tlsv1.2 https://sh.rustup.rs -o /tmp/rustup-init.sh && sh /tmp/rustup-init.sh -y --profile minimal --default-toolchain stable && rm -f /tmp/rustup-init.sh"
    fi
  fi
  run_as_user bash -lc "source \"$cargo_env\" 2>/dev/null || true; rustup default stable 2>/dev/null || true; rustc --version; cargo --version" || warn "rustc/cargo not yet on PATH (re-login sources .cargo/env)"
  mark_done rust
}
fix_user_permissions() {
  if [[ "$DRY_RUN" == "1" ]]; then return 0; fi
  run_as_user mkdir -p "$TARGET_HOME/.config" "$TARGET_HOME/.local/state" "$TARGET_HOME/.cache"
  chown -R "$TARGET_USER:$(id -gn "$TARGET_USER")" "$TARGET_HOME/.config" "$TARGET_HOME/.local" "$TARGET_HOME/.cache" 2>/dev/null || true
}

backup_configs() {
  log "Backing up existing configs..."
  fix_user_permissions
  if [[ "$DRY_RUN" == "1" ]]; then echo "[DRY_RUN as $TARGET_USER] backup hypr gtk-3.0 gtk-4.0 sshell fish matugen starship.toml to $BACKUP_DIR"; return 0; fi
  run_as_user mkdir -p "$BACKUP_DIR"
  for d in hypr gtk-3.0 gtk-4.0 sshell fish matugen; do
    if [[ -e "$TARGET_HOME/.config/$d" ]]; then
      run_as_user mkdir -p "$BACKUP_DIR/configs"
      run_as_user cp -a "$TARGET_HOME/.config/$d" "$BACKUP_DIR/configs/$d"
      dim "Backed up $d"
    fi
  done
  if [[ -f "$TARGET_HOME/.config/starship.toml" ]]; then
    run_as_user mkdir -p "$BACKUP_DIR/configs"
    run_as_user cp -a "$TARGET_HOME/.config/starship.toml" "$BACKUP_DIR/configs/starship.toml"
  fi
  if [[ -f /etc/greetd/config.toml ]]; then
    mkdir -p "$BACKUP_DIR/system"
    cp -a /etc/greetd/config.toml "$BACKUP_DIR/system/greetd-config.toml" 2>/dev/null || true
  fi
  rm -f "$BACKUP_ROOT/latest"
  run_as_user ln -s "$BACKUP_DIR" "$BACKUP_ROOT/latest"
  chown -h "$TARGET_USER:$(id -gn "$TARGET_USER")" "$BACKUP_ROOT/latest" 2>/dev/null || true
  ok "Backup saved to $BACKUP_DIR"
  mark_done backup
}

deploy_configs() {
  log "Deploying cshell configs..."
  command -v rsync &>/dev/null || die "rsync missing (install_packages failed)"
  for d in hypr gtk-3.0 gtk-4.0 matugen fish; do
    if [[ -d "$CONFIG_SRC/$d" ]]; then
      if [[ "$DRY_RUN" == "1" ]]; then echo "[DRY_RUN] rsync $CONFIG_SRC/$d/ -> $TARGET_HOME/.config/$d/"; continue; fi
      run_as_user mkdir -p "$TARGET_HOME/.config/$d"
      rsync -a "$CONFIG_SRC/$d/" "$TARGET_HOME/.config/$d/"
      chown -R "$TARGET_USER:$(id -gn "$TARGET_USER")" "$TARGET_HOME/.config/$d" 2>/dev/null || true
      dim "Installed $d"
    fi
  done
  if [[ -f "$CONFIG_SRC/starship.toml" ]]; then
    if [[ "$DRY_RUN" == "1" ]]; then echo "[DRY_RUN] install starship.toml"; else
      run_as_user mkdir -p "$TARGET_HOME/.config"
      run_as_user cp -a "$CONFIG_SRC/starship.toml" "$TARGET_HOME/.config/starship.toml"
      dim "Installed starship.toml"
    fi
  fi
  if [[ -f "$PROJECT_ROOT/config.jsonc" ]]; then
    if [[ "$DRY_RUN" == "1" ]]; then echo "[DRY_RUN] install sshell/config.jsonc"; else
      run_as_user mkdir -p "$TARGET_HOME/.config/sshell"
      run_as_user cp -a "$PROJECT_ROOT/config.jsonc" "$TARGET_HOME/.config/sshell/config.jsonc"
      dim "Installed sshell/config.jsonc"
    fi
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
  mark_done deploy
}
install_dimmer() {
  log "Installing battery-dimmer..."
  [[ -f "$DIMMER_SRC" ]] || die "Missing $DIMMER_SRC"
  run install -m 0755 "$DIMMER_SRC" "$DIMMER_BIN"
  if command -v restorecon &>/dev/null; then run restorecon -v "$DIMMER_BIN" 2>/dev/null || true; fi

  write_root_file "$ENV_FILE" 644 <<'EOF'
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

  run "$DIMMER_BIN" --install-udev || warn "udev install reported issue"
  run "$DIMMER_BIN" --install-service || warn "service install reported issue"

  run systemctl daemon-reload 2>/dev/null || true
  run systemctl enable --now battery-dimmer.service 2>/dev/null || warn "Could not start battery-dimmer (check journalctl -u battery-dimmer)"
  # tuned-ppd replaces power-profiles-daemon on Fedora: enable the right unit
  if rpm -q tuned-ppd &>/dev/null; then
    run systemctl enable --now tuned-ppd 2>/dev/null || run systemctl enable --now tuned 2>/dev/null || true
  fi
  run systemctl enable --now NetworkManager 2>/dev/null || true
  run systemctl enable --now bluetooth 2>/dev/null || true
  # verify unit actually picked up env + journal wiring
  if [[ "$DRY_RUN" != "1" ]]; then
    grep -q "EnvironmentFile=$ENV_FILE" /etc/systemd/system/battery-dimmer.service 2>/dev/null \
      || warn "dimmer unit missing EnvironmentFile (re-run --install-service)"
  fi
  ok "Dimmer active"
  mark_done dimmer
}

build_rust_shell() {
  if [[ ! -d "$RUST_SRC" || ! -f "$RUST_SRC/Cargo.toml" ]]; then
    warn "sshell-rs source missing at $RUST_SRC — skipping build"
    return 0
  fi
  log "Building sshell-rs (release, locked, ~3-5min on i5)..."
  local cargo_env="$TARGET_HOME/.cargo/env"
  if [[ "$DRY_RUN" == "1" ]]; then
    echo "[DRY_RUN as $TARGET_USER] cargo build --locked --release in $RUST_SRC"
  else
    run_as_user bash -lc "source \"$cargo_env\" 2>/dev/null || export PATH=\"\$HOME/.cargo/bin:\$PATH\"; cd \"$RUST_SRC\" && cargo build --locked --release"
  fi
  if [[ "$DRY_RUN" == "1" ]]; then return 0; fi
  if [[ -f "$RUST_SRC/target/release/sshell-rs" ]]; then
    run install -m 0755 "$RUST_SRC/target/release/sshell-rs" /usr/local/bin/sshell-rs
    if command -v restorecon &>/dev/null; then run restorecon -v /usr/local/bin/sshell-rs 2>/dev/null || true; fi
    local svc_dir="$TARGET_HOME/.config/systemd/user"
    write_user_file "$svc_dir/sshell-rs.service" <<'EOF'
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
    # enable user service for lingered user (best-effort; exec-once is primary)
    run loginctl enable-linger "$TARGET_USER" 2>/dev/null || true
    run_as_user env XDG_RUNTIME_DIR="/run/user/$TARGET_UID" systemctl --user daemon-reload 2>/dev/null || true
    run_as_user env XDG_RUNTIME_DIR="/run/user/$TARGET_UID" systemctl --user enable sshell-rs.service 2>/dev/null \
      || dim "user service written (exec-once will start it; enable needs first login)"
    ok "sshell-rs installed to /usr/local/bin/sshell-rs"
  else
    die "cargo build produced no binary (see log). Do NOT reboot; fix toolchain first."
  fi
  mark_done rustbuild
}

install_fonts() {
  log "Installing icon fonts (Material Symbols, best-effort)..."
  local fdir="$TARGET_HOME/.local/share/fonts"
  run_as_user mkdir -p "$fdir"
  local dst="$fdir/MaterialSymbolsRounded.ttf"
  if [[ "$DRY_RUN" == "1" ]]; then echo "[DRY_RUN] fetch MaterialSymbolsRounded.ttf -> $dst + fc-cache"; return 0; fi
  if [[ ! -s "$dst" ]]; then
    if run_as_user bash -c "curl -fSL --max-time 60 'https://github.com/google/material-design-icons/raw/master/variablefont/MaterialSymbolsRounded%5BFILL%2CGRAD%2Copsz%2Cwght%5D.ttf' -o '$dst' && test -s '$dst'"; then
      # validate TTF magic (00 01 00 00 / wOF2 / OTTO), reject HTML 404 pages
      if run_as_user bash -c "head -c 4 '$dst' | od -An -tx1 | grep -qiE '00 01 00 00|77 4f 46 32|4f 54 54 4f|74 72 75 65'"; then
        ok "Material Symbols installed"
      else
        warn "Material Symbols download was HTML/error (removed)"
        run_as_user rm -f "$dst"
      fi
    else
      warn "Material Symbols download failed (offline?) — continuing"
    fi
  else
    ok "Material Symbols already present"
  fi
  run_as_user fc-cache -f "$fdir" 2>/dev/null || true
  run_as_user fc-match "Material Symbols Rounded" 2>/dev/null || warn "Material Symbols not resolved (cosmetic only)"
  mark_done fonts
}
system_tuning() {
  log "Configuring greetd and power optimizations..."
  run systemctl set-default graphical.target || true

  if [[ -f /etc/greetd/config.toml && ! -f "$BACKUP_DIR/system/greetd-config.toml" && "$DRY_RUN" != "1" ]]; then
    mkdir -p "$BACKUP_DIR/system" 2>/dev/null || true
    cp -a /etc/greetd/config.toml "$BACKUP_DIR/system/greetd-config.toml" 2>/dev/null || true
  fi
  write_root_file /etc/greetd/config.toml 644 <<EOF
[terminal]
vt = 1

[default_session]
command = "tuigreet --time --remember --asterisks --user-menu --cmd Hyprland"
user = "greeter"
EOF
  run systemctl enable greetd 2>/dev/null || warn "Failed to enable greetd"

  # video group so user sshell-rs can write /sys/class/backlight (sole manual writer)
  if ! id -nG "$TARGET_USER" 2>/dev/null | tr ' ' '\n' | grep -qx video; then
    run usermod -aG video "$TARGET_USER" || warn "usermod video failed"
    log "Added $TARGET_USER to video group (re-login needed for brightness keys)"
  fi
  write_root_file /etc/udev/rules.d/99-cshell-backlight.rules 644 <<'EOF'
# cshell: let video group write backlight (sshell-rs sole manual writer, dimmer is root)
SUBSYSTEM=="backlight", GROUP="video", MODE="0664"
EOF

  # kernel cmdline: only true kernel args (module opts go to modprobe.d)
  if command -v grubby &>/dev/null; then
    run grubby --update-kernel=ALL --args="mem_sleep_default=deep i915.enable_psr=1 i915.enable_fbc=1 nvme_core.default_ps_max_latency_us=5500" 2>/dev/null || true
  fi
  write_root_file /etc/modprobe.d/cshell-powersave.conf 644 <<'EOF'
# cshell powersave (module params, not kernel cmdline)
options iwlwifi power_save=1
options btusb enable_autosuspend=1
options snd_hda_intel power_save=1 power_save_controller=Y
EOF

  # narrow USB/PCI autosuspend: exclude BT dongle + input (avoid AX201 disconnect)
  write_root_file /etc/udev/rules.d/99-cshell-powertop.rules 644 <<'EOF'
# cshell autosuspend (narrow: skip BT + HID to avoid disconnects)
ACTION=="add", SUBSYSTEM=="usb", TEST=="power/control", ATTR{idVendor}!="8087", ATTR{bInterfaceClass}!="03", ATTR{power/control}="auto"
ACTION=="add", SUBSYSTEM=="pci", TEST=="power/control", ATTR{power/control}="auto"
EOF
  write_root_file /etc/systemd/logind.conf.d/cshell-lid.conf 644 <<'EOF'
# cshell: lid close suspends (battery), power key suspends, resume reprobes dimmer
[Login]
HandleLidSwitch=suspend
HandleLidSwitchExternalPower=suspend
HandlePowerKey=suspend
IdleAction=suspend
IdleActionSec=15min
EOF
  run udevadm control --reload-rules 2>/dev/null || true
  run udevadm trigger --subsystem-match=backlight 2>/dev/null || true
  run fc-cache -f 2>/dev/null || true
  ok "System tuning applied"
  mark_done tuning
}

verify_install() {
  log "Verifying setup..."
  local fail=0
  if [[ "$DRY_RUN" == "1" ]]; then echo "[DRY_RUN] verify Hyprland dimmer sshell-rs backlight env"; return 0; fi
  command -v Hyprland &>/dev/null || { warn "Hyprland missing"; fail=1; }
  [[ -x "$DIMMER_BIN" ]] || { warn "battery-dimmer missing/not executable"; fail=1; }
  command -v sshell-rs &>/dev/null || { warn "sshell-rs missing (build failed?)"; fail=1; }
  grep -q "^THRESHOLD=50$" "$ENV_FILE" 2>/dev/null || { warn "$ENV_FILE not 50 (dimmer law broken)"; fail=1; }
  grep -q "^DIM_BY_PERCENT=30$" "$ENV_FILE" 2>/dev/null || { warn "$ENV_FILE not 30"; fail=1; }
  ls /sys/class/backlight/*/brightness &>/dev/null || { warn "no backlight node (VM? dimmer will no-op)"; }
  Hyprland --version &>/dev/null || warn "Hyprland --version failed (COPR issue?)"
  if [[ -f "$RUST_SRC/Cargo.toml" ]] && git -C "$PROJECT_ROOT" status --short 2>/dev/null | grep -q "sshell-rs/Cargo.toml"; then
    warn "Cargo.toml dirty (build mutated source)"
  fi
  if [[ "$fail" == "1" ]]; then
    die "Verification FAILED — do NOT reboot. Fix log $LOG first."
  else
    ok "All critical components verified!"
  fi
  mark_done verify
}

cmd_uninstall() {
  log "Uninstall: restoring latest backup..."
  local latest="$BACKUP_ROOT/latest"
  if [[ ! -e "$latest" ]]; then die "No backup at $latest"; fi
  local src; src="$(readlink -f "$latest")"
  log "Restoring from $src"
  for d in hypr gtk-3.0 gtk-4.0 sshell fish matugen; do
    if [[ -d "$src/configs/$d" ]]; then
      run rm -rf "$TARGET_HOME/.config/$d"
      run_as_user cp -a "$src/configs/$d" "$TARGET_HOME/.config/$d"
      dim "Restored $d"
    fi
  done
  if [[ -f "$src/configs/starship.toml" ]]; then
    run_as_user cp -a "$src/configs/starship.toml" "$TARGET_HOME/.config/starship.toml"
  fi
  if [[ -f "$src/system/greetd-config.toml" ]]; then
    run cp -a "$src/system/greetd-config.toml" /etc/greetd/config.toml
  fi
  run "$DIMMER_BIN" --remove-service 2>/dev/null || run systemctl disable --now battery-dimmer.service 2>/dev/null || true
  run "$DIMMER_BIN" --remove-udev 2>/dev/null || true
  run rm -f /usr/local/bin/sshell-rs /usr/local/bin/battery-dimmer.sh
  run rm -f "$DONE_MARKER"
  fix_user_permissions
  ok "Uninstall complete (packages kept, configs restored)"
}

main() {
  parse_args "$@"
  mkdir -p "$(dirname "$LOG")" 2>/dev/null || LOG="/tmp/cshell-setup.log"
  touch "$LOG" 2>/dev/null || LOG="/tmp/cshell-setup.log"
  banner | tee -a "$LOG" 2>/dev/null || banner
  if [[ $EUID -ne 0 && "$DRY_RUN" != "1" ]]; then
    die "Run as root: sudo ./fedora-setup.sh --yes"
  fi
  if [[ "$ASSUME_YES" != "1" && "$UNINSTALL" != "1" ]]; then
    die "Need --yes on TTY (sudo ./fedora-setup.sh --yes)"
  fi
  if [[ "$UNINSTALL" == "1" ]]; then cmd_uninstall; exit 0; fi
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
  echo -e "${GREEN}${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}" | tee -a "$LOG" 2>/dev/null || true
  ok "Installation Complete! Reboot now: sudo reboot"
  echo -e "${GREEN}${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}" | tee -a "$LOG" 2>/dev/null || true
}

main "$@"
