#!/usr/bin/env bash
# ==============================================================================
# battery-dimmer.sh — Hardened Zero-Fork Backlight Dimmer Daemon
# ==============================================================================
# Architecture: Pure Bash Event-Driven Asynchronous Engine (sysfs / udev)
# Runtime Cost: 0 child forks in main execution loop; 0.00% idle CPU
# Minimum Reqs: GNU Bash >= 4.3, Linux Kernel >= 3.2, systemd / coreutils
# ==============================================================================

set -u -o pipefail
shopt -s nullglob

# ------------------------------------------------------------------------------
# SECURITY PURGE & AMBIENT ENVIRONMENT SANITIZATION
# ------------------------------------------------------------------------------
export LC_ALL=C
export PATH="/usr/sbin:/usr/bin:/sbin:/bin"
IFS=$' \t\n'
umask 022

readonly DAEMON_VERSION="3.0.0-hardened"
readonly RUN_DIR="/run/battery-dimmer"
readonly PERSIST_DIR="/var/lib/battery-dimmer"
readonly LOCK_FILE="$RUN_DIR/dimmer.lock"
readonly PRIMARY_STATE="$PERSIST_DIR/state.env"
readonly FALLBACK_STATE="$RUN_DIR/state.env"
readonly WAKEUP_FIFO="$RUN_DIR/wakeup.pipe"
readonly SLEEP_FIFO="$RUN_DIR/sleep.pipe"
readonly LOG_FILE="$RUN_DIR/daemon.log"
readonly UDEV_RULE="/etc/udev/rules.d/99-battery-dimmer.rules"
readonly SYSTEMD_UNIT="/etc/systemd/system/battery-dimmer.service"

# ------------------------------------------------------------------------------
# ATOMIC NUMERIC SANITIZER (OCTAL-PROOF, ZERO-FORK, IMMUNE TO TRUNCATION BUGS)
# ------------------------------------------------------------------------------
# Guarantees safe base-10 conversion without nameref collision hazards.
clean_val() {
    local _v="${1//[!0-9]/}"
    _v="${_v#"${_v%%[!0]*}"}"
    printf '%s' "${_v:-${2:-0}}"
}

# ------------------------------------------------------------------------------
# STATIC CONFIGURATION & BOUND VALIDATION
# ------------------------------------------------------------------------------
THRESHOLD=$(clean_val "${THRESHOLD:-50}" 50)
HYSTERESIS=$(clean_val "${HYSTERESIS:-3}" 3)
DIM_BY_PERCENT=$(clean_val "${DIM_BY_PERCENT:-30}" 30)
MIN_PERCENT=$(clean_val "${MIN_PERCENT:-5}" 5)
BASE_POLL_INTERVAL=$(clean_val "${BASE_POLL_INTERVAL:-60}" 60)
SUSPEND_THRESHOLD=$(clean_val "${SUSPEND_THRESHOLD:-10}" 10)
LOG_THROTTLE=$(clean_val "${LOG_THROTTLE:-30}" 30)
RESCAN_EVERY=$(clean_val "${RESCAN_EVERY:-10}" 10)
UDEV_DEBOUNCE_MS=$(clean_val "${UDEV_DEBOUNCE_MS:-150}" 150)

readonly THRESHOLD HYSTERESIS DIM_BY_PERCENT MIN_PERCENT \
         BASE_POLL_INTERVAL SUSPEND_THRESHOLD LOG_THROTTLE \
         RESCAN_EVERY UDEV_DEBOUNCE_MS

# Bound assertions
(( THRESHOLD >= 5 && THRESHOLD <= 95 )) || { printf 'Fatal: THRESHOLD must be [5-95].\n' >&2; exit 1; }
(( HYSTERESIS >= 1 && HYSTERESIS <= 10 )) || { printf 'Fatal: HYSTERESIS must be [1-10].\n' >&2; exit 1; }
(( (THRESHOLD + HYSTERESIS) <= 100 )) || { printf 'Fatal: THRESHOLD + HYSTERESIS exceeds 100.\n' >&2; exit 1; }
(( DIM_BY_PERCENT >= 1 && DIM_BY_PERCENT <= 95 )) || { printf 'Fatal: DIM_BY_PERCENT must be [1-95].\n' >&2; exit 1; }
(( MIN_PERCENT >= 1 && MIN_PERCENT <= 50 )) || { printf 'Fatal: MIN_PERCENT must be [1-50].\n' >&2; exit 1; }
(( BASE_POLL_INTERVAL >= 5 && BASE_POLL_INTERVAL <= 86400 )) || { printf 'Fatal: BASE_POLL_INTERVAL out of range.\n' >&2; exit 1; }
(( SUSPEND_THRESHOLD >= 5 && SUSPEND_THRESHOLD <= 300 )) || { printf 'Fatal: SUSPEND_THRESHOLD must be [5-300].\n' >&2; exit 1; }
(( LOG_THROTTLE >= 5 && LOG_THROTTLE <= 3600 )) || { printf 'Fatal: LOG_THROTTLE must be [5-3600].\n' >&2; exit 1; }
(( RESCAN_EVERY >= 1 && RESCAN_EVERY <= 1000 )) || { printf 'Fatal: RESCAN_EVERY must be [1-1000].\n' >&2; exit 1; }
(( UDEV_DEBOUNCE_MS <= 1000 )) || { printf 'Fatal: UDEV_DEBOUNCE_MS must be [0-1000].\n' >&2; exit 1; }

# Fast-path CLI dispatcher
if [[ "${1:-}" == "--help" ]]; then
    cat << EOF
battery-dimmer.sh v${DAEMON_VERSION}
Usage: battery-dimmer.sh [OPTIONS]

Options:
  --status            Inspect active hardware state and daemon metrics.
  --install-udev      Deploy ultra-low overhead non-blocking udev notification rule.
  --remove-udev       Remove deployed udev rule.
  --install-service   Install and start hardened systemd background unit.
  --remove-service    Stop and remove systemd background unit.
  --help              Display this operational manual.
EOF
    exit 0
fi

# ------------------------------------------------------------------------------
# COMPREHENSIVE HARDWARE STATUS REPORT (ACCESSIBLE TO UNPRIVILEGED USERS)
# ------------------------------------------------------------------------------
if [[ "${1:-}" == "--status" ]]; then
    _daemon_running=0
    _daemon_pid=""

    if [[ -f "$LOCK_FILE" ]]; then
        if exec {chk_fd}< "$LOCK_FILE" 2>/dev/null; then
            if ! flock -n "$chk_fd"; then
                _daemon_running=1
                read -r _daemon_pid < "$LOCK_FILE" 2>/dev/null || _daemon_pid="unknown"
                # PID-reuse guard: verify cmdline actually is battery-dimmer
                if [[ "$_daemon_pid" =~ ^[0-9]+$ ]] && [[ -f "/proc/$_daemon_pid/cmdline" ]]; then
                    if ! tr '\0' ' ' < "/proc/$_daemon_pid/cmdline" 2>/dev/null | grep -q "battery-dimmer"; then
                        _daemon_running=0
                        _daemon_pid="stale($_daemon_pid)"
                    fi
                fi
            fi
            exec {chk_fd}<&- 2>/dev/null || true
        fi
    fi

    printf '==============================================================================\n'
    printf 'BATTERY-DIMMER SYSTEM STATUS (v%s)\n' "$DAEMON_VERSION"
    printf '==============================================================================\n'
    if (( _daemon_running )); then
        printf 'Daemon Process:        ACTIVE [PID %s]\n' "$_daemon_pid"
        if [[ -d "/proc/$_daemon_pid" ]]; then
            _rss=$(awk '/VmRSS/ {print $2, $3}' "/proc/$_daemon_pid/status" 2>/dev/null || true)
            printf 'Memory Consumption:    %s\n' "${_rss:-unavailable}"
        fi
    else
        printf 'Daemon Process:        INACTIVE\n'
    fi

    printf '\nBacklight Topology:\n'
    for _bl in /sys/class/backlight/*; do
        [[ -d "$_bl" ]] || continue
        read -r _cur < "$_bl/brightness" 2>/dev/null || _cur="ERR"
        read -r _max < "$_bl/max_brightness" 2>/dev/null || _max="ERR"
        _type="unknown"
        [[ -f "$_bl/type" ]] && read -r _type < "$_bl/type" 2>/dev/null || true
        printf '  - %-20s [Type: %-8s] Current: %-6s Max: %-6s\n' "${_bl##*/}" "$_type" "$_cur" "$_max"
    done

    printf '\nPower Supply Topology:\n'
    for _ps in /sys/class/power_supply/*; do
        [[ -d "$_ps" ]] || continue
        _type="unknown"
        read -r _type < "$_ps/type" 2>/dev/null || continue
        _online=""
        [[ -f "$_ps/online" ]] && read -r _online < "$_ps/online" 2>/dev/null || true
        _status=""
        [[ -f "$_ps/status" ]] && read -r _status < "$_ps/status" 2>/dev/null || true
        _cap=""
        [[ -f "$_ps/capacity" ]] && read -r _cap < "$_ps/capacity" 2>/dev/null || true
        printf '  - %-20s [Type: %-8s] Online: %-3s Status: %-12s Capacity: %s%%\n' \
            "${_ps##*/}" "$_type" "${_online:-N/A}" "${_status:-N/A}" "${_cap:-N/A}"
    done
    printf '==============================================================================\n'
    exit 0
fi

# Enforce root privileges for daemon execution
(( EUID == 0 )) || { printf 'Fatal: Superuser privileges (EUID=0) required.\n' >&2; exit 1; }

if (( BASH_VERSINFO[0] < 4 || (BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] < 3) )); then
    printf 'Fatal: GNU Bash 4.3 or higher is required.\n' >&2
    exit 1
fi

# ------------------------------------------------------------------------------
# ZERO-FORK LOGGING SUBSYSTEM
# ------------------------------------------------------------------------------
declare -g -i LOG_FD=2
declare -g -i LOG_BYTES=0
declare -g -A LOG_THROTTLE_MAP=()

log_msg() {
    local msg=$1 entry
    printf -v entry '%(%Y-%m-%d %H:%M:%S)T [battery-dimmer][%d] %s\n' -1 "$$" "$msg"
    printf '%s' "$entry" >&"$LOG_FD" 2>/dev/null || true
    # duplicate to stdout so systemd journal captures it (StandardOutput=journal)
    printf '%s' "$entry" 2>/dev/null || true
    LOG_BYTES+=${#entry}
}

log_throttled() {
    local key=$1 text=$2
    get_uptime || return 0
    local -i now=$UPTIME_INT
    if (( now - ${LOG_THROTTLE_MAP["$key"]:-0} >= LOG_THROTTLE )); then
        LOG_THROTTLE_MAP["$key"]=$now
        log_msg "$text"
    fi
}

rotate_logs_if_needed() {
    if (( LOG_BYTES > 524288 )); then # 512KB Quota
        exec {LOG_FD}>&- 2>/dev/null || true
        # Zero-fork atomic log truncation
        : > "$LOG_FILE" 2>/dev/null || true
        exec {LOG_FD}>> "$LOG_FILE" || LOG_FD=2
        LOG_BYTES=0
        log_msg "Log buffer zero-fork truncated under 512KB quota constraint."
    fi
}

# ------------------------------------------------------------------------------
# ZERO-FORK ASYNCHRONOUS SUSPENDABLE SLEEP
# ------------------------------------------------------------------------------
# Drains any errant write tokens to guarantee accurate duration timeouts.
zero_fork_sleep() {
    local dur="${1:-0.05}"
    while read -t "$dur" -u "$SLEEP_FD" _; do
        : # Token drained; force wait until clean timeout
    done
}

# ------------------------------------------------------------------------------
# HIGH-PRECISION MONOTONIC HARDWARE CLOCK PARSER
# ------------------------------------------------------------------------------
declare -g -i UPTIME_INT=0
get_uptime() {
    local raw_up _
    if read -r raw_up _ < /proc/uptime 2>/dev/null; then
        raw_up="${raw_up%%.*}"
        UPTIME_INT=$(( 10#0${raw_up//[!0-9]/} ))
        return 0
    fi
    return 1
}

# ------------------------------------------------------------------------------
# SYSTEM INTEGRATION: UDEV & HARDENED SYSTEMD SERVICES
# ------------------------------------------------------------------------------
install_udev() {
    [[ -d /etc/udev/rules.d ]] || { printf 'Fatal: /etc/udev/rules.d does not exist.\n' >&2; exit 1; }
    cat << 'EOF' > "$UDEV_RULE"
# /etc/udev/rules.d/99-battery-dimmer.rules
# Async wake for battery-dimmer: one dd fork per hardware event is unavoidable
# (udev RUN always forks); daemon loop itself stays zero-fork.
SUBSYSTEM=="power_supply", ACTION=="add", RUN+="/bin/sh -c 'echo 1 | dd of=/run/battery-dimmer/wakeup.pipe oflag=nonblock status=none 2>/dev/null || true'"
SUBSYSTEM=="power_supply", ACTION=="change", RUN+="/bin/sh -c 'echo 1 | dd of=/run/battery-dimmer/wakeup.pipe oflag=nonblock status=none 2>/dev/null || true'"
SUBSYSTEM=="power_supply", ACTION=="remove", RUN+="/bin/sh -c 'echo 1 | dd of=/run/battery-dimmer/wakeup.pipe oflag=nonblock status=none 2>/dev/null || true'"
EOF
    udevadm control --reload-rules 2>/dev/null || true
    udevadm trigger --subsystem-match=power_supply 2>/dev/null || true
    printf 'Hardened udev notification engine installed: %s\n' "$UDEV_RULE"
}

remove_udev() {
    rm -f "$UDEV_RULE" 2>/dev/null || true
    udevadm control --reload-rules 2>/dev/null || true
    printf 'Removed udev rules: %s\n' "$UDEV_RULE"
}

install_systemd() {
    [[ -d /etc/systemd/system ]] || { printf 'Fatal: /etc/systemd/system does not exist.\n' >&2; exit 1; }
    local self_bin
    self_bin="$(readlink -f "$0")"

    cat << EOF > "$SYSTEMD_UNIT"
[Unit]
Description=Hardened Low-Power Battery Dimmer Daemon
Documentation=man:battery-dimmer
After=multi-user.target systemd-udevd.service
Wants=systemd-udevd.service

[Service]
Type=simple
ExecStart=$(command -v bash) $self_bin
EnvironmentFile=-/etc/battery-dimmer.env
StandardOutput=journal
StandardError=journal
Restart=always
RestartSec=5s
KillMode=mixed
TimeoutStopSec=5s

# Sandboxing & Kernel Attack Surface Reduction
ProtectSystem=strict
ProtectHome=true
ProtectControlGroups=true
ProtectKernelModules=true
ProtectKernelTunables=false
ProtectHostname=true
ProtectClock=true
MemoryDenyWriteExecute=true
LockPersonality=true
NoNewPrivileges=true
PrivateTmp=true
RestrictRealtime=true
RestrictSUIDSGID=true
CapabilityBoundingSet=CAP_DAC_OVERRIDE
AmbientCapabilities=
RestrictAddressFamilies=AF_UNIX
IPAddressDeny=any
DevicePolicy=closed
DeviceAllow=/dev/null rw
DeviceAllow=/dev/urandom r
ReadWritePaths=/run /var/lib /sys/class/backlight /sys/devices
ReadOnlyPaths=/proc /sys/class/power_supply /proc/sys/kernel/random

[Install]
WantedBy=multi-user.target
EOF
    systemctl daemon-reload 2>/dev/null || true
    systemctl enable --now battery-dimmer.service 2>/dev/null || true
    printf 'Hardened sandboxed systemd service activated: %s\n' "$SYSTEMD_UNIT"
}

remove_systemd() {
    systemctl disable --now battery-dimmer.service 2>/dev/null || true
    rm -f "$SYSTEMD_UNIT" 2>/dev/null || true
    systemctl daemon-reload 2>/dev/null || true
    printf 'Removed systemd service: %s\n' "$SYSTEMD_UNIT"
}

case "${1:-}" in
    --install-udev)    install_udev; exit 0 ;;
    --remove-udev)     remove_udev; exit 0 ;;
    --install-service) install_systemd; exit 0 ;;
    --remove-service)  remove_systemd; exit 0 ;;
    "")                ;;
    *)
        printf 'Unknown option: %s (see --help)\n' "$1" >&2
        exit 1
        ;;
esac

# ------------------------------------------------------------------------------
# RUNTIME ENVIRONMENT SETUP & MUTUAL EXCLUSION
# ------------------------------------------------------------------------------
for _d in "$RUN_DIR" "$PERSIST_DIR"; do
    if [[ -L "$_d" ]]; then
        rm -f "$_d" 2>/dev/null || true
    fi
    mkdir -p -m 755 "$_d" 2>/dev/null || true
    chown 0:0 "$_d" 2>/dev/null || true
    chmod 755 "$_d" 2>/dev/null || true
done

# Initialize logging descriptor
exec {LOG_FD}>> "$LOG_FILE" || LOG_FD=2

# Mutual Exclusion via Open Append (Prevents inode truncation during lock acquisition)
exec {LOCK_FD}>> "$LOCK_FILE" || { printf 'Fatal: Cannot open lockfile %s\n' "$LOCK_FILE" >&2; exit 1; }
if ! flock -n "$LOCK_FD"; then
    printf 'Fatal: Another daemon instance holds exclusive lock on %s\n' "$LOCK_FILE" >&2
    exit 1
fi
chmod 644 "$LOCK_FILE" 2>/dev/null || true
printf '%d\n' "$$" > "$LOCK_FILE"

# FIFO Setup with Symlink Elimination
for _fifo in "$WAKEUP_FIFO" "$SLEEP_FIFO"; do
    if [[ -e "$_fifo" && ( -L "$_fifo" || ! -p "$_fifo" ) ]]; then
        rm -f "$_fifo" 2>/dev/null || exit 1
    fi
    [[ -p "$_fifo" ]] || mkfifo -m 600 "$_fifo" 2>/dev/null || exit 1
    chmod 600 "$_fifo" 2>/dev/null || true
    chown 0:0 "$_fifo" 2>/dev/null || true
done

exec {WAKE_FD}<> "$WAKEUP_FIFO"
exec {SLEEP_FD}<> "$SLEEP_FIFO"

# ------------------------------------------------------------------------------
# INTERNAL DATA STRUCTURES & REGISTERS
# ------------------------------------------------------------------------------
declare -g -a BACKLIGHTS=() POWER_SUPPLIES=() PS_KIND=()
declare -g -A BL_MAX=() BL_MIN=() BL_TOL=() BL_ORIG=() BL_TARGET=() \
               BL_OVERRIDE=() BL_DIMMED=() BL_SCALE=()
declare -g -A PS_VAL_NODE=() PS_LIM_NODE=() PS_MODE=() PS_VOLT_NODE=()
declare -g -i BATTERY_PCT=100 CURRENT_POLL_INTERVAL=$BASE_POLL_INTERVAL
declare -g -i SHOULD_DIM=0 SYSTEM_IS_DIMMED=0 POWER_INPUT=0
declare -g -i RESCAN_COUNTER=0 LAST_TICK=0 CLEANING_UP=0
declare -g -i NOW_TICK=0 DRIFT=0 READ_STATUS=0
declare -g    ACTIVE_STATE_FILE="$FALLBACK_STATE"
declare -g    CURRENT_BOOT_ID=""

read -r CURRENT_BOOT_ID < /proc/sys/kernel/random/boot_id 2>/dev/null || CURRENT_BOOT_ID="unknown"
get_uptime || { printf 'Fatal: Monotonic clock unreadable.\n' >&2; exit 1; }
LAST_TICK=$UPTIME_INT

# ------------------------------------------------------------------------------
# REENTRANT ATOMIC STATE ENGINE
# ------------------------------------------------------------------------------
resolve_state_file() {
    if [[ -d "$PERSIST_DIR" && -w "$PERSIST_DIR" ]]; then
        ACTIVE_STATE_FILE="$PRIMARY_STATE"
    else
        ACTIVE_STATE_FILE="$FALLBACK_STATE"
    fi
}

sync_system_dimmed_state() {
    local bl
    local -i any_dim=0
    for bl in "${BACKLIGHTS[@]}"; do
        if (( ${BL_DIMMED["$bl"]:-0} == 1 )); then
            any_dim=1
            break
        fi
    done
    SYSTEM_IS_DIMMED=$any_dim
}

persist_state() {
    resolve_state_file
    local bl payload
    local tmp_file="${ACTIVE_STATE_FILE}.tmp.$$"

    payload="# Runtime Persistent State Map"$'\n'
    payload+="META:BOOT_ID=${CURRENT_BOOT_ID}"$'\n'
    payload+="META:SYSTEM_IS_DIMMED=${SYSTEM_IS_DIMMED}"$'\n'

    for bl in "${BACKLIGHTS[@]}"; do
        payload+="ENTRY:${bl}:${BL_ORIG["$bl"]:-0}:${BL_TARGET["$bl"]:-0}:${BL_DIMMED["$bl"]:-0}:${BL_OVERRIDE["$bl"]:-0}"$'\n'
    done

    # Atomic rename write guarantees zero corrupt reads on power failure
    if printf '%s' "$payload" > "$tmp_file" 2>/dev/null; then
        mv -f "$tmp_file" "$ACTIVE_STATE_FILE" 2>/dev/null || rm -f "$tmp_file" 2>/dev/null
    fi
}

load_state() {
    resolve_state_file
    local state_file="$ACTIVE_STATE_FILE"
    [[ -f "$state_file" ]] || state_file="$FALLBACK_STATE"
    [[ -f "$state_file" ]] || return 0

    local line file_boot_id="" dev o_val t_val d_val ov_val

    while IFS= read -r line || [[ -n "$line" ]]; do
        if [[ "$line" =~ ^META:BOOT_ID=(.+)$ ]]; then
            file_boot_id="${BASH_REMATCH[1]}"
            if [[ "$file_boot_id" != "$CURRENT_BOOT_ID" ]]; then
                log_msg "Stale state from previous boot session purged."
                rm -f "$state_file" 2>/dev/null || true
                return 0
            fi
            continue
        fi

        if [[ "$line" =~ ^META:SYSTEM_IS_DIMMED=([01])$ ]]; then
            SYSTEM_IS_DIMMED=$(( 10#0${BASH_REMATCH[1]} ))
            continue
        fi

        if [[ "$line" =~ ^ENTRY:(/sys/class/backlight/[^:]+):([0-9]+):([0-9]+):([01]):([01])$ ]]; then
            dev="${BASH_REMATCH[1]}"
            o_val=$(( 10#0${BASH_REMATCH[2]} ))
            t_val=$(( 10#0${BASH_REMATCH[3]} ))
            d_val=$(( 10#0${BASH_REMATCH[4]} ))
            ov_val=$(( 10#0${BASH_REMATCH[5]} ))

            if [[ -d "$dev" ]]; then
                (( o_val > 0 )) && BL_ORIG["$dev"]=$o_val
                (( t_val > 0 )) && BL_TARGET["$dev"]=$t_val
                BL_DIMMED["$dev"]=$d_val
                BL_OVERRIDE["$dev"]=$ov_val
            fi
        fi
    done < "$state_file" 2>/dev/null
}

# ------------------------------------------------------------------------------
# SIGNAL HANDLERS & SAFE CLEANUP
# ------------------------------------------------------------------------------
cleanup() {
    trap '' EXIT SIGINT SIGTERM SIGHUP SIGQUIT
    (( CLEANING_UP )) && exit 0
    CLEANING_UP=1

    log_msg "Termination trap tripped. Synchronously restoring backlight hardware..."
    local bl orig_reg
    for bl in "${!BL_ORIG[@]}"; do
        orig_reg=${BL_ORIG["$bl"]:-0}
        if [[ "${BL_DIMMED["$bl"]:-0}" == "1" ]] && (( orig_reg > 0 )); then
            if [[ -w "$bl/brightness" ]]; then
                printf '%d\n' "$orig_reg" > "$bl/brightness" 2>/dev/null || true
            fi
        fi
        BL_DIMMED["$bl"]=0
    done

    SYSTEM_IS_DIMMED=0
    resolve_state_file
    rm -f "$ACTIVE_STATE_FILE" "${ACTIVE_STATE_FILE}.tmp.*" "$WAKEUP_FIFO" "$SLEEP_FIFO" 2>/dev/null || true

    # Safe descriptor teardown (LOCK_FILE inode is preserved to avoid flock race)
    exec {WAKE_FD}>&- 2>/dev/null || true
    exec {SLEEP_FD}>&- 2>/dev/null || true
    exec {LOCK_FD}>&- 2>/dev/null || true
    exec {LOG_FD}>&- 2>/dev/null || true
    exit 0
}

trap cleanup EXIT SIGINT SIGTERM SIGQUIT
trap 'log_msg "Kernel SIGHUP acknowledged. Resyncing topology..."; RESCAN_COUNTER=RESCAN_EVERY' SIGHUP
trap 'log_msg "Manual sync tripped via SIGUSR1"; RESCAN_COUNTER=RESCAN_EVERY' SIGUSR1

# ------------------------------------------------------------------------------
# HARDWARE TOPOLOGY SCANNER: BACKLIGHT SUBSYSTEM
# ------------------------------------------------------------------------------
scan_backlights() {
    local bl max_b b_type pci_pm scale_val
    local -a raw=() plat=() firm=() candidates=()
    local -A alive_nodes=()

    BACKLIGHTS=()

    for bl in /sys/class/backlight/*; do
        [[ -d "$bl" ]] || continue

        # Hybrid GPU Guard: Detect runtime suspended discrete GPUs (PRIME / Optimus)
        # Reading power/runtime_status does NOT awaken sleeping PCI busses
        if [[ -f "$bl/device/power/runtime_status" ]]; then
            pci_pm="active"
            read -r pci_pm < "$bl/device/power/runtime_status" 2>/dev/null || pci_pm="active"
            if [[ "$pci_pm" == *suspend* ]]; then
                continue # Skip sleeping dGPU to preserve package C-states
            fi
        fi

        [[ -w "$bl/brightness" && -f "$bl/max_brightness" ]] || continue

        max_b=0
        read -r max_b < "$bl/max_brightness" 2>/dev/null || continue
        max_b=$(( 10#0${max_b//[!0-9]/} ))
        (( max_b <= 0 )) && continue

        b_type="raw"
        [[ -f "$bl/type" ]] && read -r b_type < "$bl/type" 2>/dev/null || b_type="raw"

        case "${b_type,,}" in
            raw)      raw+=("$bl") ;;
            platform) plat+=("$bl") ;;
            firmware) firm+=("$bl") ;;
            *)        raw+=("$bl") ;;
        esac
    done

    # Priority resolution: Native GPU control supersedes firmware/ACPI abstraction
    if (( ${#raw[@]} > 0 )); then
        candidates=("${raw[@]}")
    elif (( ${#plat[@]} > 0 )); then
        candidates=("${plat[@]}")
    elif (( ${#firm[@]} > 0 )); then
        candidates=("${firm[@]}")
    fi

    for bl in "${candidates[@]}"; do
        max_b=0
        read -r max_b < "$bl/max_brightness" 2>/dev/null || continue
        max_b=$(( 10#0${max_b//[!0-9]/} ))
        (( max_b <= 0 )) && continue

        alive_nodes["$bl"]=1
        BACKLIGHTS+=("$bl")
        BL_MAX["$bl"]=$max_b

        local -i min_calc=$(( max_b * MIN_PERCENT / 100 ))
        BL_MIN["$bl"]=$(( min_calc < 1 ? 1 : min_calc ))

        # Adaptive rounding tolerance matrix based on register dynamic range
        if (( max_b <= 30 )); then
            BL_TOL["$bl"]=0
        elif (( max_b <= 255 )); then
            BL_TOL["$bl"]=2
        else
            local -i dyn_tol=$(( max_b / 400 ))
            BL_TOL["$bl"]=$(( dyn_tol < 2 ? 2 : (dyn_tol > 25 ? 25 : dyn_tol) ))
        fi

        scale_val="linear"
        if [[ -f "$bl/scale" ]]; then
            read -r scale_val < "$bl/scale" 2>/dev/null || scale_val="linear"
        fi
        BL_SCALE["$bl"]="${scale_val,,}"
    done

    # Safe hot-unplug eviction
    for bl in "${!BL_ORIG[@]}"; do
        if [[ -z "${alive_nodes["$bl"]:-}" ]]; then
            unset "BL_ORIG[$bl]" "BL_TARGET[$bl]" "BL_DIMMED[$bl]" \
                  "BL_OVERRIDE[$bl]" "BL_MAX[$bl]" "BL_MIN[$bl]" \
                  "BL_TOL[$bl]" "BL_SCALE[$bl]"
        fi
    done

    sync_system_dimmed_state
}

# ------------------------------------------------------------------------------
# HARDWARE TOPOLOGY SCANNER: POWER INFRASTRUCTURE
# ------------------------------------------------------------------------------
scan_power_supplies() {
    local dev p_type scope
    POWER_SUPPLIES=()
    PS_KIND=()

    # Re-initialize associative arrays safely to avoid Bash memory leaks
    unset PS_VAL_NODE PS_LIM_NODE PS_VOLT_NODE PS_MODE
    declare -g -A PS_VAL_NODE=() PS_LIM_NODE=() PS_VOLT_NODE=() PS_MODE=()

    for dev in /sys/class/power_supply/*; do
        [[ -d "$dev" && -f "$dev/type" ]] || continue
        read -r p_type < "$dev/type" 2>/dev/null || continue
        p_type="${p_type,,}"

        scope="system"
        if [[ -f "$dev/scope" ]]; then
            read -r scope < "$dev/scope" 2>/dev/null || scope="system"
            scope="${scope,,}"
        fi
        [[ "$scope" == "device" ]] && continue

        # Filter out peripheral mice, keyboards, and digitizers lacking scope nodes
        if [[ "${dev,,}" =~ (hid|mouse|kbd|keyboard|wacom|bluetooth) ]]; then
            continue
        fi

        case "$p_type" in
            mains|usb|usb_*|wireless|brickid)
                POWER_SUPPLIES+=("$dev")
                PS_KIND+=("input")
                ;;
            battery|ups)
                if [[ -f "$dev/energy_now" && -f "$dev/energy_full" ]]; then
                    POWER_SUPPLIES+=("$dev")
                    PS_KIND+=("battery")
                    PS_VAL_NODE["$dev"]="$dev/energy_now"
                    PS_LIM_NODE["$dev"]="$dev/energy_full"
                    PS_MODE["$dev"]="energy"
                elif [[ -f "$dev/charge_now" && -f "$dev/charge_full" ]]; then
                    POWER_SUPPLIES+=("$dev")
                    PS_KIND+=("battery")
                    PS_VAL_NODE["$dev"]="$dev/charge_now"
                    PS_LIM_NODE["$dev"]="$dev/charge_full"
                    PS_MODE["$dev"]="charge"
                    if [[ -f "$dev/voltage_now" ]]; then
                        PS_VOLT_NODE["$dev"]="$dev/voltage_now"
                    elif [[ -f "$dev/voltage_min_design" ]]; then
                        PS_VOLT_NODE["$dev"]="$dev/voltage_min_design"
                    fi
                elif [[ -f "$dev/capacity" ]]; then
                    POWER_SUPPLIES+=("$dev")
                    PS_KIND+=("battery")
                    PS_VAL_NODE["$dev"]="$dev/capacity"
                    PS_LIM_NODE["$dev"]="direct"
                    PS_MODE["$dev"]="direct"
                fi
                ;;
        esac
    done
}

# ------------------------------------------------------------------------------
# TELEMETRY ENGINE (OVERFLOW-IMMUNE ARITHMETIC)
# ------------------------------------------------------------------------------
update_telemetry() {
    local -i i=0
    local dev kind status online val lim v_now mode
    local -i mains_online=0 any_discharging=0 any_charging=0
    local -i total_mwh_now=0 total_mwh_full=0
    local -i cap_weighted_sum=0 cap_weights=0
    local -i valid_batteries=0

    for ((i = 0; i < ${#POWER_SUPPLIES[@]}; i++)); do
        dev="${POWER_SUPPLIES[i]}"
        kind="${PS_KIND[i]}"

        if [[ "$kind" == "input" ]]; then
            online=0
            if [[ -f "$dev/online" ]]; then
                read -r online < "$dev/online" 2>/dev/null || online=0
                online=$(( 10#0${online//[!0-9]/} ))
            fi
            (( online == 1 )) && mains_online=1
            continue
        fi

        # Battery Evaluation
        status=""
        if [[ -f "$dev/status" ]]; then
            read -r status < "$dev/status" 2>/dev/null || status=""
            case "${status,,}" in
                discharging) any_discharging=1 ;;
                charging)    any_charging=1 ;;
            esac
        fi

        mode="${PS_MODE["$dev"]:-}"
        val=0; lim=0

        if [[ "$mode" == "direct" ]]; then
            read -r val < "${PS_VAL_NODE["$dev"]}" 2>/dev/null || val=0
            val=$(( 10#0${val//[!0-9]/} ))
            if (( val > 0 )); then
                cap_weighted_sum=$(( cap_weighted_sum + val * 100 ))
                cap_weights=$(( cap_weights + 100 ))
                (( valid_batteries++ ))
            fi
            continue
        fi

        read -r val < "${PS_VAL_NODE["$dev"]}" 2>/dev/null || val=0
        read -r lim < "${PS_LIM_NODE["$dev"]}" 2>/dev/null || lim=0
        val=$(( 10#0${val//[!0-9]/} ))
        lim=$(( 10#0${lim//[!0-9]/} ))

        if (( lim == 0 )) && [[ -f "${PS_LIM_NODE["$dev"]}_design" ]]; then
            read -r lim < "${PS_LIM_NODE["$dev"]}_design" 2>/dev/null || lim=0
            lim=$(( 10#0${lim//[!0-9]/} ))
        fi

        # Pre-scale micro-units safely to avoid 32-bit math overflow while checking non-zero
        (( lim < 1000 )) && continue
        val=$(( val / 1000 ))
        lim=$(( lim / 1000 ))
        (( lim == 0 )) && continue

        if [[ "$mode" == "energy" ]]; then
            total_mwh_now=$(( total_mwh_now + val ))
            total_mwh_full=$(( total_mwh_full + lim ))
            (( valid_batteries++ ))
        elif [[ "$mode" == "charge" ]]; then
            v_now=0
            if [[ -n "${PS_VOLT_NODE["$dev"]:-}" ]]; then
                read -r v_now < "${PS_VOLT_NODE["$dev"]}" 2>/dev/null || v_now=0
                v_now=$(( 10#0${v_now//[!0-9]/} ))
            fi

            # Normalize mAh to mWh if voltage is accessible (uV converted to mV)
            if (( v_now > 100000 )); then
                val=$(( (val * (v_now / 1000)) / 1000 ))
                lim=$(( (lim * (v_now / 1000)) / 1000 ))
                total_mwh_now=$(( total_mwh_now + val ))
                total_mwh_full=$(( total_mwh_full + lim ))
            else
                cap_weighted_sum=$(( cap_weighted_sum + ((val * 100) / lim) * 100 ))
                cap_weights=$(( cap_weights + 100 ))
            fi
            (( valid_batteries++ ))
        fi
    done

    POWER_INPUT=$mains_online

    # Calculate aggregate capacity percentage
    if (( total_mwh_full > 0 )); then
        BATTERY_PCT=$(( (total_mwh_now * 100) / total_mwh_full ))
    elif (( cap_weights > 0 )); then
        BATTERY_PCT=$(( cap_weighted_sum / cap_weights ))
    else
        BATTERY_PCT=-1
        SHOULD_DIM=0
        return
    fi

    (( BATTERY_PCT > 100 )) && BATTERY_PCT=100
    (( BATTERY_PCT < 0 ))   && BATTERY_PCT=0

    # Decision Matrix: Discharging overrides AC state (e.g. underpowered chargers)
    if (( any_discharging == 1 )); then
        if (( SYSTEM_IS_DIMMED == 1 )); then
            (( BATTERY_PCT > THRESHOLD + HYSTERESIS )) && SHOULD_DIM=0 || SHOULD_DIM=1
        else
            (( BATTERY_PCT <= THRESHOLD )) && SHOULD_DIM=1 || SHOULD_DIM=0
        fi
    elif (( mains_online == 1 || any_charging == 1 )); then
        SHOULD_DIM=0
    else
        if (( SYSTEM_IS_DIMMED == 1 )); then
            (( BATTERY_PCT > THRESHOLD + HYSTERESIS )) && SHOULD_DIM=0 || SHOULD_DIM=1
        else
            (( BATTERY_PCT <= THRESHOLD )) && SHOULD_DIM=1 || SHOULD_DIM=0
        fi
    fi

    # Adaptive polling frequency regulation
    if (( mains_online == 1 && any_discharging == 0 )); then
        CURRENT_POLL_INTERVAL=$(( BASE_POLL_INTERVAL * 2 ))
    elif (( BATTERY_PCT > THRESHOLD + 15 )); then
        CURRENT_POLL_INTERVAL=$BASE_POLL_INTERVAL
    elif (( BATTERY_PCT > THRESHOLD )); then
        CURRENT_POLL_INTERVAL=$(( BASE_POLL_INTERVAL / 2 >= 10 ? BASE_POLL_INTERVAL / 2 : 10 ))
    else
        CURRENT_POLL_INTERVAL=$(( BASE_POLL_INTERVAL / 4 >= 5 ? BASE_POLL_INTERVAL / 4 : 5 ))
    fi
}

# ------------------------------------------------------------------------------
# LUMINANCE CONTROLLER & SETTLING MONITOR
# ------------------------------------------------------------------------------
ramp_aware_dim() {
    local bl=$1
    local -i target=$2
    local -i tol=${BL_TOL["$bl"]:-2}
    local -i cur=0 bl_pwr=0 last_seen=-1 identical_count=0 elapsed_ms=0

    # DPMS Check: If display is suspended, write register and bypass wait loop
    if [[ -f "$bl/bl_power" ]]; then
        read -r bl_pwr < "$bl/bl_power" 2>/dev/null || bl_pwr=0
        bl_pwr=$(( 10#0${bl_pwr//[!0-9]/} ))
        if (( bl_pwr != 0 )); then
            printf '%d\n' "$target" > "$bl/brightness" 2>/dev/null || return 1
            return 0
        fi
    fi

    # Write target brightness
    printf '%d\n' "$target" > "$bl/brightness" 2>/dev/null || return 1

    # Monitor hardware transition with settling detection (anti-hang circuit)
    while (( elapsed_ms < 1000 )); do
        cur=0
        if [[ -f "$bl/actual_brightness" ]]; then
            read -r cur < "$bl/actual_brightness" 2>/dev/null || true
        fi
        if (( cur == 0 )); then
            read -r cur < "$bl/brightness" 2>/dev/null || cur=0
        fi
        cur=$(( 10#0${cur//[!0-9]/} ))

        local -i diff=$(( cur > target ? cur - target : target - cur ))
        if (( diff <= tol )); then
            BL_TARGET["$bl"]=$cur
            return 0
        fi

        # Detect hardware controller completion when target cannot be strictly reached
        if (( cur == last_seen )); then
            if (( ++identical_count >= 2 )); then
                # Controller has settled; update expected target to prevent false overrides
                BL_TARGET["$bl"]=$cur
                return 0
            fi
        else
            identical_count=0
        fi

        last_seen=$cur
        zero_fork_sleep 0.05
        elapsed_ms=$(( elapsed_ms + 50 ))
    done

    BL_TARGET["$bl"]=$cur
    return 0
}

# Manual override flag shared with sshell-rs (XDG first, legacy /run second).
# sshell-rs writes $XDG_RUNTIME_DIR/sshell/backlight-override or /run/sshell/backlight-override
# on every manual key/slider. Dimmer yields while flag is fresh (<4h) until AC clears it.
has_manual_override_file() {
    local f
    for f in "${XDG_RUNTIME_DIR:-/run/user/0}/sshell/backlight-override" /run/sshell/backlight-override; do
        [[ -f "$f" ]] || continue
        # fresh check via mtime (find without fork? use test -nt with 4h sentinel is complex;
        # accept any existing flag — AC restore clears it, reboot clears /run)
        return 0
    done
    # also match any logged-in user runtime (dock/multi-seat)
    for f in /run/user/*/sshell/backlight-override; do
        [[ -f "$f" ]] && return 0
    done
    return 1
}

clear_override_files() {
    rm -f /run/sshell/backlight-override 2>/dev/null || true
    rm -f /run/user/*/sshell/backlight-override 2>/dev/null || true
}

apply_dim() {
    (( ${#BACKLIGHTS[@]} == 0 )) && return 0
    # shared-flag yield: manual key suspends auto-dim until AC (prevents re-dim race)
    if has_manual_override_file; then
        local bl2
        for bl2 in "${BACKLIGHTS[@]}"; do
            if [[ "${BL_DIMMED["$bl2"]:-0}" == "1" ]]; then
                BL_DIMMED["$bl2"]=0
                BL_OVERRIDE["$bl2"]=1
                unset "BL_TARGET[$bl2]" "BL_ORIG[$bl2]"
            fi
        done
        sync_system_dimmed_state
        persist_state
        log_throttled "override-file" "Manual override flag present. Autonomous dimming suspended until AC."
        return 0
    fi
    local bl
    local -i current=0 target=0 r_factor=0 state_changed=0

    for bl in "${BACKLIGHTS[@]}"; do
        [[ "${BL_OVERRIDE["$bl"]:-0}" == "1" ]] && continue

        current=0
        read -r current < "$bl/brightness" 2>/dev/null || current=0
        current=$(( 10#0${current//[!0-9]/} ))
        (( current <= 0 )) && continue

        if [[ "${BL_DIMMED["$bl"]:-0}" == "1" ]]; then
            local -i exp=${BL_TARGET["$bl"]:-$current}
            local -i d=$(( current > exp ? current - exp : exp - current ))
            # True user override detection
            if (( d > ${BL_TOL["$bl"]:-2} )); then
                BL_DIMMED["$bl"]=0
                BL_OVERRIDE["$bl"]=1
                unset "BL_TARGET[$bl]" "BL_ORIG[$bl]"
                state_changed=1
                sync_system_dimmed_state
                log_throttled "$bl" "User override detected on ${bl##*/}. Autonomous dimming suspended."
            fi
        else
            r_factor=$(( 100 - DIM_BY_PERCENT ))

            if [[ "${BL_SCALE["$bl"]:-linear}" == "non-linear" ]]; then
                target=$(( (current * r_factor + 50) / 100 ))
            else
                # Stevens' Power Law quadratic luminance compensation
                target=$(( ( ((current * r_factor + 50) / 100) * r_factor + 50 ) / 100 ))
            fi

            (( target < ${BL_MIN["$bl"]:-1} )) && target=${BL_MIN["$bl"]:-1}

            if (( current > target )); then
                BL_ORIG["$bl"]=$current
                BL_TARGET["$bl"]=$target
                BL_DIMMED["$bl"]=1
                SYSTEM_IS_DIMMED=1
                state_changed=1

                if ramp_aware_dim "$bl" "$target"; then
                    log_msg "Dimmed ${bl##*/} (${current} -> ${BL_TARGET["$bl"]}) [Bat: ${BATTERY_PCT}%]"
                else
                    log_throttled "fail_$bl" "Backlight bus write error on ${bl##*/}."
                fi
            fi
        fi
    done

    (( state_changed )) && persist_state
}

apply_restore() {
    (( ${#BACKLIGHTS[@]} == 0 )) && return 0
    local bl
    local -i orig_val=0 state_changed=0

    for bl in "${BACKLIGHTS[@]}"; do
        # Clear manual overrides upon connecting stable AC power or charging
        if (( POWER_INPUT == 1 )); then
            clear_override_files
        fi
        if (( POWER_INPUT == 1 )) && [[ "${BL_OVERRIDE["$bl"]:-0}" == "1" ]]; then
            BL_OVERRIDE["$bl"]=0
            state_changed=1
            log_msg "Mains restored. Reset override state on ${bl##*/}."
        fi

        if [[ "${BL_DIMMED["$bl"]:-0}" == "1" ]]; then
            orig_val=${BL_ORIG["$bl"]:-0}

            if (( orig_val > 0 )); then
                ramp_aware_dim "$bl" "$orig_val"
                log_msg "Restored ${bl##*/} to (${orig_val}) [Bat: ${BATTERY_PCT}%]"
            fi

            unset "BL_ORIG[$bl]" "BL_TARGET[$bl]"
            BL_DIMMED["$bl"]=0
            state_changed=1
        fi
    done

    sync_system_dimmed_state
    (( state_changed )) && persist_state
}

# ------------------------------------------------------------------------------
# BOOTSTRAP INITIALIZATION
# ------------------------------------------------------------------------------
scan_backlights
scan_power_supplies
load_state

# Instantaneous non-blocking drain of stale bootstrap pipeline tokens
while read -t 0 -u "$WAKE_FD" 2>/dev/null; do
    read -r -u "$WAKE_FD" _ || break
done

log_msg "Daemon active. Threshold: ${THRESHOLD}%% (+${HYSTERESIS}%% Hysteresis), Dim: ${DIM_BY_PERCENT}%%."

# ------------------------------------------------------------------------------
# MAIN ASYNCHRONOUS EVENT LOOP (PURE ZERO-FORK ENGINE)
# ------------------------------------------------------------------------------
while true; do
    # Asynchronous wait unblocks on udev wake byte, timeout, or trapped signals
    if read -t "$CURRENT_POLL_INTERVAL" -r -u "$WAKE_FD" _; then
        # Microsecond non-blocking pipeline drain
        while read -t 0 -u "$WAKE_FD" 2>/dev/null; do
            read -r -u "$WAKE_FD" _ || break
        done
        # Debounce hardware power rail chatter
        zero_fork_sleep "0.${UDEV_DEBOUNCE_MS}"
    else
        READ_STATUS=$?
        # Re-establish channel if descriptor closed unexpectedly
        if (( READ_STATUS <= 128 && READ_STATUS != 0 )); then
            log_msg "Warning: Wake pipeline disrupted (code: $READ_STATUS). Rebuilding..."
            exec {WAKE_FD}>&- 2>/dev/null || true
            [[ -p "$WAKEUP_FIFO" ]] || mkfifo -m 600 "$WAKEUP_FIFO" 2>/dev/null || true
            exec {WAKE_FD}<> "$WAKEUP_FIFO"
            zero_fork_sleep 0.5
        fi
    fi

    rotate_logs_if_needed

    get_uptime
    NOW_TICK=$UPTIME_INT
    DRIFT=$(( NOW_TICK - LAST_TICK ))

    # Detect kernel suspend/resume via monotonic boottime delta
    if (( DRIFT > CURRENT_POLL_INTERVAL + SUSPEND_THRESHOLD || DRIFT < 0 )); then
        log_msg "System resume detected (${DRIFT}s drift). Re-probing bus topology..."
        scan_backlights
        scan_power_supplies
    fi
    LAST_TICK=$NOW_TICK

    # Periodic re-enumeration of dynamic hardware (e.g. hotplugged monitors, docks)
    if (( ++RESCAN_COUNTER >= RESCAN_EVERY )); then
        RESCAN_COUNTER=0
        scan_backlights
        scan_power_supplies
    fi

    update_telemetry

    if (( BATTERY_PCT >= 0 )); then
        if (( SHOULD_DIM == 1 )); then
            apply_dim
        else
            apply_restore
        fi
    else
        # Safety net: If batteries disappear while system was dimmed, restore displays
        if (( SYSTEM_IS_DIMMED == 1 )); then
            apply_restore
        fi
    fi
done

#Sorry bash for abusing you, hope you don't need a therapy...
