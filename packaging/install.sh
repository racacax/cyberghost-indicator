#!/usr/bin/env bash
# Install cyberghost-indicator from an unzipped release.
#
# Usage: ./install.sh [options]
#   --prefix DIR     install prefix (default: ~/.local, or /usr/local with --system)
#   --system         install for all users (/usr/local, /etc/xdg/autostart); run with sudo
#   --no-autostart   don't start the indicator at login
#   --no-sudoers     don't install the passwordless sudo rule for cyberghostvpn
#   --no-start       don't launch the indicator after installing
#   -y, --yes        don't ask questions (accept the sudoers rule)
# Environment: CYBERGHOST_BIN=/path/to/cyberghostvpn for a non-standard CLI location.
set -euo pipefail

APP=cyberghost-indicator
SRC="$(cd "$(dirname "$0")" && pwd)"

SYSTEM=0 PREFIX="" AUTOSTART=1 SUDOERS=1 START=1 YES=0
while [ $# -gt 0 ]; do
    case "$1" in
        --prefix) PREFIX="${2:?--prefix needs a directory}"; shift ;;
        --prefix=*) PREFIX="${1#*=}" ;;
        --system) SYSTEM=1 ;;
        --no-autostart) AUTOSTART=0 ;;
        --no-sudoers) SUDOERS=0 ;;
        --no-start) START=0 ;;
        -y|--yes) YES=1 ;;
        -h|--help) sed -n '2,12s/^# \{0,1\}//p' "$0"; exit 0 ;;
        *) echo "unknown option: $1 (see --help)" >&2; exit 2 ;;
    esac
    shift
done

info() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33mwarning:\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }
ask()  { [ "$YES" = 1 ] && return 0; [ -t 0 ] || return 1; read -r -p "$1 [Y/n] " a; [[ -z "$a" || "$a" =~ ^[YyOo] ]]; }

# ---------------------------------------------------------------- checks
[ "$(uname -s)" = Linux ] || die "Linux only"
[ "$(uname -m)" = x86_64 ] || die "this build is for x86_64 (got $(uname -m))"
[ -x "$SRC/bin/$APP" ] || die "run this script from the unzipped release directory"

if [ "$SYSTEM" = 1 ]; then
    [ "$(id -u)" = 0 ] || die "--system needs root: sudo ./install.sh --system"
    PREFIX="${PREFIX:-/usr/local}"
    TARGET_USER="${SUDO_USER:-}"
    AUTOSTART_DIR=/etc/xdg/autostart
else
    [ "$(id -u)" != 0 ] || die "don't run a per-user install as root (use --system for a system-wide install)"
    PREFIX="${PREFIX:-$HOME/.local}"
    TARGET_USER="$(id -un)"
    AUTOSTART_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/autostart"
fi
PREFIX="$(mkdir -p "$PREFIX" && cd "$PREFIX" && pwd)"
BIN="$PREFIX/bin/$APP"
SHARE="$PREFIX/share/$APP"
MANIFEST="$SHARE/install-manifest"

# CyberGhost CLI: $CYBERGHOST_BIN if set (then passed on to the app), else the default paths.
CG=""
if [ -n "${CYBERGHOST_BIN:-}" ]; then
    [ -x "$CYBERGHOST_BIN" ] && CG="$CYBERGHOST_BIN"
else
    for p in /usr/bin/cyberghostvpn /usr/local/bin/cyberghostvpn; do
        [ -x "$p" ] && { CG="$p"; break; }
    done
fi
[ -n "$CG" ] || warn "cyberghostvpn not found: install the CyberGhost CLI (and run 'cyberghostvpn --setup') before using the indicator"
# zenity shows the favorites window (installed by default on Ubuntu/Debian/Fedora GNOME, Xfce…)
ZENITY="${CYBERGHOST_INDICATOR_ZENITY:-zenity}"
command -v "$ZENITY" >/dev/null || die "zenity is required (favorites window). Install it first:
    sudo apt install zenity      # Debian, Ubuntu
    sudo dnf install zenity      # Fedora
    sudo pacman -S zenity        # Arch"
command -v notify-send >/dev/null || warn "notify-send not found: no notifications (apt install libnotify-bin)"
command -v sudo >/dev/null || [ "$SUDOERS" = 0 ] || { warn "sudo not found: skipping sudoers rule"; SUDOERS=0; }

# ---------------------------------------------------------------- files
info "installing to $PREFIX"
# stop a running instance so the binary can be replaced
# (process names are truncated to 15 chars: "cyberghost-indi")
pkill -u "$(id -u "${TARGET_USER:-root}")" -x "${APP:0:15}" 2>/dev/null && sleep 0.5 || true

install -d "$PREFIX/bin" "$SHARE/icons"
install -m 755 "$SRC/bin/$APP" "$BIN"
rm -f "$SHARE"/icons/*.png
install -m 644 "$SRC"/share/$APP/icons/*.png "$SHARE/icons/"
install -m 755 "$SRC/uninstall.sh" "$SHARE/uninstall.sh"
{
    echo "$BIN"
    echo "$SHARE"
} > "$MANIFEST"

if [ "$AUTOSTART" = 1 ]; then
    install -d "$AUTOSTART_DIR"
    cat > "$AUTOSTART_DIR/$APP.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=CyberGhost VPN indicator
Comment=Connect / disconnect CyberGhost VPN from the system tray
Exec=${CYBERGHOST_BIN:+env CYBERGHOST_BIN=$CYBERGHOST_BIN }$BIN
Icon=$SHARE/icons/ghost-on.png
Terminal=false
Categories=Network;
X-GNOME-Autostart-enabled=true
EOF
    echo "$AUTOSTART_DIR/$APP.desktop" >> "$MANIFEST"
    info "autostart entry: $AUTOSTART_DIR/$APP.desktop"
fi

# ---------------------------------------------------------------- sudoers
# Connecting needs root. A NOPASSWD rule limited to cyberghostvpn avoids a
# password prompt on every connection (otherwise pkexec asks each time).
SUDOERS_FILE=/etc/sudoers.d/cyberghost-indicator
if [ "$SUDOERS" = 1 ] && [ -n "$CG" ] && [ -n "$TARGET_USER" ]; then
    CG_REAL="$(readlink -f "$CG")"
    RULE="$TARGET_USER ALL=(root) NOPASSWD: $CG"
    [ "$CG_REAL" != "$CG" ] && RULE="$RULE, $CG_REAL"
    if sudo -n -l "$CG" >/dev/null 2>&1 && [ "$(id -u)" != 0 ]; then
        info "passwordless sudo for cyberghostvpn already configured"
    elif ask "Allow '$TARGET_USER' to run cyberghostvpn as root without a password (needed to connect from the tray)?"; then
        tmp="$(mktemp)"
        printf '# installed by %s\n%s\n' "$APP" "$RULE" > "$tmp"
        SUDO=""; [ "$(id -u)" = 0 ] || SUDO=sudo
        if $SUDO visudo -cqf "$tmp" && $SUDO install -m 440 -o root -g root "$tmp" "$SUDOERS_FILE"; then
            echo "$SUDOERS_FILE" >> "$MANIFEST"
            info "sudoers rule: $SUDOERS_FILE"
        else
            warn "could not install the sudoers rule; you'll be asked for your password on each connection"
        fi
        rm -f "$tmp"
    else
        warn "no sudoers rule: you'll be asked for your password (pkexec) on each connection"
    fi
fi

# ---------------------------------------------------------------- tray host
if command -v busctl >/dev/null && [ "$SYSTEM" = 0 ] \
   && ! busctl --user status org.kde.StatusNotifierWatcher >/dev/null 2>&1; then
    warn "no system tray host (StatusNotifierWatcher) found on this session."
    warn "XFCE: add the 'Indicator Plugin' or 'Status Tray' to the panel;"
    warn "GNOME: install the 'AppIndicator and KStatusNotifierItem Support' extension."
fi

case ":$PATH:" in *":$PREFIX/bin:"*) ;; *) warn "$PREFIX/bin is not in your PATH (the autostart entry uses the full path, so this is fine)";; esac

if [ "$START" = 1 ] && [ "$SYSTEM" = 0 ] && [ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ]; then
    setsid -f "$BIN" >/dev/null 2>&1 < /dev/null
    info "started"
fi

info "done — $("$BIN" --version)"
echo "    uninstall: $SHARE/uninstall.sh"
