#!/usr/bin/env bash
# Tests for packaging/install.sh and packaging/uninstall.sh, run against an
# unzipped release directory:   tests/install_test.sh dist/cyberghost-indicator-<v>-x86_64-linux
#
# Every test runs in a sandbox (temporary HOME and prefix) with fake sudo / visudo /
# pkill / busctl / setsid / id in PATH: nothing outside the sandbox is touched,
# /etc/sudoers.d is never written and no running indicator is killed.
set -uo pipefail

RELEASE="$(cd "${1:?usage: $0 <unzipped release dir>}" && pwd)"
[ -x "$RELEASE/install.sh" ] || { echo "not a release dir: $RELEASE" >&2; exit 2; }
APP=cyberghost-indicator
PASS=0 FAIL=0 FAILED=()

# ---------------------------------------------------------------- sandbox
setup() {
    T="$(mktemp -d)"
    mkdir -p "$T/home" "$T/fake" "$T/cg" "$T/work"
    export HOME="$T/home" XDG_CONFIG_HOME="$T/home/.config" XDG_CACHE_HOME="$T/home/.cache"
    export CYBERGHOST_BIN="$T/cg/cyberghostvpn"
    printf '#!/bin/sh\nexit 0\n' > "$CYBERGHOST_BIN"; chmod +x "$CYBERGHOST_BIN"
    unset DISPLAY WAYLAND_DISPLAY SUDO_USER CYBERGHOST_INDICATOR_ZENITY
    fake sudo '
echo "sudo $*" >> "$T/sudo.log"
if [ "$1" = -n ] && [ "$2" = -l ]; then [ -f "$T/sudo_configured" ]; exit $?; fi
case "$1" in
    visudo) cp "${@: -1}" "$T/sudoers.checked"; [ ! -f "$T/visudo_fail" ]; exit $? ;;
    install|rm) exit 0 ;;   # would write /etc/sudoers.d: only logged
esac
exit 0'
    fake pkill 'echo "pkill $*" >> "$T/pkill.log"; exit 1'
    fake busctl '[ ! -f "$T/no_tray" ]'
    fake setsid 'echo "setsid $*" >> "$T/setsid.log"'
    fake zenity 'exit 0'   # required dependency: don't depend on the host having it
    fake id 'if [ "$1" = -u ] && [ $# = 1 ] && [ -f "$T/as_root" ]; then echo 0; else exec /usr/bin/id "$@"; fi'
    export PATH="$T/fake:/usr/bin:/bin" T
}
teardown() { rm -rf "$T"; }

fake() { printf '#!/bin/bash\n%s\n' "$2" > "$T/fake/$1"; chmod +x "$T/fake/$1"; }

# run the installer (stdin is not a terminal: no prompts)
inst() { "$RELEASE/install.sh" "$@" > "$T/out" 2>&1 < /dev/null; }
uninst() { "$1" "${@:2}" > "$T/out" 2>&1 < /dev/null; }

# ---------------------------------------------------------------- assertions
fail() { echo "    $*"; return 1; }
assert_file() { [ -f "$1" ] || fail "missing file $1"; }
assert_no() { [ ! -e "$1" ] || fail "should not exist: $1"; }
assert_exec() { [ -x "$1" ] || fail "not executable: $1"; }
assert_grep() { grep -qF -- "$2" "$1" 2>/dev/null || fail "'$2' not in $1: $(cat "$1" 2>/dev/null | head -20)"; }
assert_no_grep() { ! grep -qF -- "$2" "$1" 2>/dev/null || fail "'$2' unexpectedly in $1"; }
assert_eq() { [ "$1" = "$2" ] || fail "expected '$2', got '$1'"; }

PREFIX_DEFAULT() { echo "$HOME/.local"; }

# ---------------------------------------------------------------- tests
test_default_install() {
    inst --yes || fail "install failed: $(cat "$T/out")" || return 1
    local p; p="$(PREFIX_DEFAULT)"
    assert_exec "$p/bin/$APP" &&
    assert_eq "$("$p/bin/$APP" --version)" "$("$RELEASE/bin/$APP" --version)" &&
    assert_eq "$(ls "$p/share/$APP/icons" | wc -l)" "$(ls "$RELEASE/share/$APP/icons" | wc -l)" &&
    assert_file "$p/share/$APP/icons/ghost-off.png" &&
    assert_exec "$p/share/$APP/uninstall.sh" &&
    assert_eq "$(stat -c %a "$p/bin/$APP")" 755 &&
    assert_eq "$(stat -c %a "$p/share/$APP/icons/flag-FR.png")" 644 &&
    # autostart entry
    local d="$XDG_CONFIG_HOME/autostart/$APP.desktop"
    assert_file "$d" &&
    assert_grep "$d" "Exec=env CYBERGHOST_BIN=$CYBERGHOST_BIN $p/bin/$APP" &&
    assert_grep "$d" "Icon=$p/share/$APP/icons/ghost-on.png" &&
    assert_grep "$d" "Type=Application" &&
    # sudoers rule validated with visudo then installed (via sudo)
    assert_grep "$T/sudoers.checked" "$(id -un) ALL=(root) NOPASSWD: $CYBERGHOST_BIN" &&
    assert_grep "$T/sudo.log" "sudo visudo -cqf" &&
    assert_grep "$T/sudo.log" "sudo install -m 440 -o root -g root" &&
    assert_grep "$T/sudo.log" "/etc/sudoers.d/$APP" &&
    # manifest lists everything to remove
    local m="$p/share/$APP/install-manifest"
    assert_grep "$m" "$p/bin/$APP" && assert_grep "$m" "$p/share/$APP" &&
    assert_grep "$m" "$d" && assert_grep "$m" "/etc/sudoers.d/$APP" &&
    # running instances are stopped by process name, never by command line
    assert_grep "$T/pkill.log" "-x cyberghost-indi" &&
    assert_no_grep "$T/pkill.log" " -f " &&
    # no display: not started
    assert_no "$T/setsid.log" &&
    assert_grep "$T/out" "done — $APP"
}

test_options_disable_autostart_and_sudoers() {
    inst --no-autostart --no-sudoers || fail "$(cat "$T/out")" || return 1
    assert_exec "$(PREFIX_DEFAULT)/bin/$APP" &&
    assert_no "$XDG_CONFIG_HOME/autostart/$APP.desktop" &&
    assert_no "$T/sudo.log" &&
    assert_no_grep "$(PREFIX_DEFAULT)/share/$APP/install-manifest" "sudoers"
}

test_custom_prefix_relative_path() {
    cd "$T/work" && inst --prefix ./myprefix --no-sudoers || fail "$(cat "$T/out")" || return 1
    assert_exec "$T/work/myprefix/bin/$APP" &&
    assert_grep "$XDG_CONFIG_HOME/autostart/$APP.desktop" "$T/work/myprefix/bin/$APP" &&
    assert_no "$(PREFIX_DEFAULT)/bin/$APP"
}

test_prefix_equals_syntax() {
    inst --prefix="$T/p2" --no-sudoers || fail "$(cat "$T/out")" || return 1
    assert_exec "$T/p2/bin/$APP"
}

test_runs_from_any_directory() {
    cd / && inst --no-sudoers || fail "$(cat "$T/out")" || return 1
    assert_exec "$(PREFIX_DEFAULT)/bin/$APP"
}

test_reinstall_is_idempotent() {
    inst --yes || fail "$(cat "$T/out")" || return 1
    local p; p="$(PREFIX_DEFAULT)"
    touch "$p/share/$APP/icons/flag-OLD.png"
    : > "$T/sudo.log"; touch "$T/sudo_configured"
    inst --yes || fail "$(cat "$T/out")" || return 1
    assert_no "$p/share/$APP/icons/flag-OLD.png" &&
    assert_eq "$(sort "$p/share/$APP/install-manifest" | uniq -d)" "" &&
    assert_grep "$T/out" "already configured" &&
    assert_no_grep "$T/sudo.log" "visudo"
}

test_uninstall_removes_installed_files_only() {
    inst --yes || fail "$(cat "$T/out")" || return 1
    local p; p="$(PREFIX_DEFAULT)"
    echo keep > "$p/bin/other-tool"
    mkdir -p "$XDG_CACHE_HOME/$APP" && echo '{}' > "$XDG_CACHE_HOME/$APP/state.json"
    : > "$T/sudo.log"
    uninst "$p/share/$APP/uninstall.sh" || fail "uninstall failed: $(cat "$T/out")" || return 1
    assert_no "$p/bin/$APP" &&
    assert_no "$p/share/$APP" &&
    assert_no "$XDG_CONFIG_HOME/autostart/$APP.desktop" &&
    assert_grep "$T/sudo.log" "sudo rm -f /etc/sudoers.d/$APP" &&
    assert_file "$p/bin/other-tool" &&
    assert_file "$XDG_CACHE_HOME/$APP/state.json" &&
    assert_grep "$T/out" "uninstalled"
}

test_uninstall_purge_removes_cache() {
    inst --no-sudoers || fail "$(cat "$T/out")" || return 1
    mkdir -p "$XDG_CACHE_HOME/$APP" && echo '{}' > "$XDG_CACHE_HOME/$APP/lists.json"
    uninst "$(PREFIX_DEFAULT)/share/$APP/uninstall.sh" --purge || fail "$(cat "$T/out")" || return 1
    assert_no "$XDG_CACHE_HOME/$APP"
}

test_uninstall_from_release_dir_finds_installation() {
    inst --no-sudoers || fail "$(cat "$T/out")" || return 1
    uninst "$RELEASE/uninstall.sh" || fail "$(cat "$T/out")" || return 1
    assert_no "$(PREFIX_DEFAULT)/bin/$APP"
}

test_uninstall_without_installation_fails() {
    uninst "$RELEASE/uninstall.sh" && fail "should fail"
    assert_grep "$T/out" "no installation found"
}

test_missing_cli_warns_and_skips_sudoers() {
    export CYBERGHOST_BIN="$T/nowhere/cyberghostvpn"
    inst --yes || fail "$(cat "$T/out")" || return 1
    assert_grep "$T/out" "cyberghostvpn not found" &&
    assert_no "$T/sudo.log"
}

test_missing_zenity_aborts_before_installing() {
    export CYBERGHOST_INDICATOR_ZENITY="$T/nowhere/zenity"
    inst --yes && fail "should refuse without zenity"
    assert_grep "$T/out" "zenity is required" &&
    assert_grep "$T/out" "sudo apt install zenity" &&
    assert_no "$(PREFIX_DEFAULT)/bin/$APP" &&
    assert_no "$XDG_CONFIG_HOME/autostart/$APP.desktop" &&
    assert_no "$T/sudo.log"
}

test_sudo_rule_already_configured() {
    touch "$T/sudo_configured"
    inst --yes || fail "$(cat "$T/out")" || return 1
    assert_grep "$T/out" "passwordless sudo for cyberghostvpn already configured" &&
    assert_no_grep "$T/sudo.log" "visudo"
}

test_non_interactive_without_yes_skips_sudoers() {
    inst || fail "$(cat "$T/out")" || return 1
    assert_grep "$T/out" "no sudoers rule" &&
    assert_no_grep "$T/sudo.log" "visudo"
}

test_invalid_sudoers_rule_is_not_installed() {
    touch "$T/visudo_fail"
    inst --yes || fail "$(cat "$T/out")" || return 1
    assert_grep "$T/out" "could not install the sudoers rule" &&
    assert_no_grep "$T/sudo.log" "sudo install" &&
    assert_no_grep "$(PREFIX_DEFAULT)/share/$APP/install-manifest" "sudoers"
}

test_symlinked_cli_allows_both_paths() {
    mkdir -p "$T/real" && printf '#!/bin/sh\n' > "$T/real/cg" && chmod +x "$T/real/cg"
    ln -sf "$T/real/cg" "$T/cg/link"
    export CYBERGHOST_BIN="$T/cg/link"
    inst --yes || fail "$(cat "$T/out")" || return 1
    assert_grep "$T/sudoers.checked" "NOPASSWD: $T/cg/link, $T/real/cg"
}

test_refuses_per_user_install_as_root() {
    touch "$T/as_root"
    inst && fail "should refuse"
    assert_grep "$T/out" "don't run a per-user install as root" &&
    assert_no "$(PREFIX_DEFAULT)/bin/$APP"
}

test_system_install_requires_root() {
    inst --system && fail "should refuse"
    assert_grep "$T/out" "--system needs root"
}

test_warns_without_tray_host() {
    touch "$T/no_tray"
    inst --no-sudoers || fail "$(cat "$T/out")" || return 1
    assert_grep "$T/out" "no system tray host"
}

test_starts_the_indicator_in_a_graphical_session() {
    DISPLAY=:99 inst --no-sudoers || fail "$(cat "$T/out")" || return 1
    assert_grep "$T/setsid.log" "setsid -f $(PREFIX_DEFAULT)/bin/$APP"
}

test_no_start_option() {
    DISPLAY=:99 inst --no-sudoers --no-start || fail "$(cat "$T/out")" || return 1
    assert_no "$T/setsid.log"
}

test_help_and_bad_options() {
    "$RELEASE/install.sh" --help > "$T/out" 2>&1 || fail "--help failed" || return 1
    assert_grep "$T/out" "--no-autostart" && assert_grep "$T/out" "CYBERGHOST_BIN" &&
    { "$RELEASE/install.sh" --bogus > "$T/out" 2>&1; assert_eq "$?" 2; } &&
    assert_grep "$T/out" "unknown option: --bogus" &&
    { "$RELEASE/install.sh" --prefix > "$T/out" 2>&1; [ $? -ne 0 ] || fail "--prefix without value accepted"; }
}

test_refuses_to_run_outside_a_release() {
    cp "$RELEASE/install.sh" "$T/work/install.sh"
    "$T/work/install.sh" > "$T/out" 2>&1 < /dev/null && fail "should refuse"
    assert_grep "$T/out" "run this script from the unzipped release directory"
}

# ---------------------------------------------------------------- runner
for t in $(declare -F | awk '{print $3}' | grep '^test_'); do
    setup
    if ( set -e; cd "$T/work"; "$t" ); then
        PASS=$((PASS + 1)); echo "ok   $t"
    else
        FAIL=$((FAIL + 1)); FAILED+=("$t"); echo "FAIL $t"
    fi
    teardown
done
echo
echo "install tests: $PASS passed, $FAIL failed"
[ "$FAIL" = 0 ] || { printf '  %s\n' "${FAILED[@]}"; exit 1; }
