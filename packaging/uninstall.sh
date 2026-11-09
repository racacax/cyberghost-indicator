#!/usr/bin/env bash
# Uninstall cyberghost-indicator (files listed in the install manifest).
#   ~/.local/share/cyberghost-indicator/uninstall.sh            per-user install
#   sudo /usr/local/share/cyberghost-indicator/uninstall.sh     --system install
#   --purge   also remove the cache (~/.cache/cyberghost-indicator)
set -euo pipefail
APP=cyberghost-indicator
PURGE=0
[ "${1:-}" = "--purge" ] && PURGE=1

# Locate the manifest: next to this script once installed, else the default prefixes.
HERE="$(cd "$(dirname "$0")" && pwd)"
MANIFEST=""
for m in "$HERE/install-manifest" "$HOME/.local/share/$APP/install-manifest" "/usr/local/share/$APP/install-manifest"; do
    [ -f "$m" ] && { MANIFEST="$m"; break; }
done
[ -n "$MANIFEST" ] || { echo "error: no installation found (install-manifest missing)" >&2; exit 1; }

pkill -u "$(id -u "${SUDO_USER:-$(id -un)}")" -x "${APP:0:15}" 2>/dev/null || true

mapfile -t FILES < "$MANIFEST"
for f in "${FILES[@]}"; do
    [ -n "$f" ] || continue
    case "$f" in
        /etc/sudoers.d/*)
            if [ "$(id -u)" = 0 ]; then rm -f "$f"; else sudo rm -f "$f"; fi ;;
        */share/$APP)
            rm -rf -- "$f" ;;   # icons, manifest, this script
        *)
            rm -f -- "$f" ;;
    esac
    echo "removed $f"
done

if [ "$PURGE" = 1 ]; then
    rm -rf -- "${XDG_CACHE_HOME:-$HOME/.cache}/$APP"
    echo "removed cache"
fi
echo "cyberghost-indicator uninstalled"
