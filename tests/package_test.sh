#!/usr/bin/env bash
# Checks a release ZIP: layout, checksums, static binary, version, icons, scripts.
#   tests/package_test.sh dist/cyberghost-indicator-<version>-x86_64-linux.zip
set -uo pipefail
ZIP="$(realpath "${1:?usage: $0 <release.zip>}")"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP=cyberghost-indicator
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
NAME="$APP-$VERSION-x86_64-linux"
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
PASS=0 FAIL=0

check() {
    local desc="$1"; shift
    if "$@" > "$T/check.out" 2>&1; then PASS=$((PASS + 1)); echo "ok   $desc"
    else FAIL=$((FAIL + 1)); echo "FAIL $desc"; sed 's/^/     /' "$T/check.out" | head -20; fi
}

unzip -q "$ZIP" -d "$T"
R="$T/$NAME"

check "zip name matches the version"           test "$(basename "$ZIP")" = "$NAME.zip"
check "single top-level directory"             test "$(ls "$T" | grep -v check.out)" = "$NAME"
check "zip checksum file matches"              bash -c "cd '$(dirname "$ZIP")' && sha256sum -c --quiet '$NAME.zip.sha256'"
check "SHA256SUMS covers every file"           bash -c "cd '$R' && sha256sum -c --quiet SHA256SUMS && test \$(find . -type f ! -name SHA256SUMS | wc -l) = \$(wc -l < SHA256SUMS)"
for f in bin/$APP install.sh uninstall.sh share/$APP/uninstall.sh; do
    check "$f is executable"                   test -x "$R/$f"
done
for f in README.md LICENSE-ICONS.md; do
    check "$f is included"                     test -s "$R/$f"
done
check "binary is statically linked"            bash -c "file '$R/bin/$APP' | grep -Eq 'static(-pie)? linked' || ldd '$R/bin/$APP' 2>&1 | grep -Eq 'not a dynamic|statically linked'"
check "binary is x86_64"                       bash -c "file '$R/bin/$APP' | grep -q x86-64"
check "binary is stripped"                     bash -c "file '$R/bin/$APP' | grep -q ', stripped'"
check "binary reports version $VERSION"        test "$("$R/bin/$APP" --version)" = "$APP $VERSION"
check "all bundled icons are shipped"          diff <(cd "$ROOT/assets/icons" && ls) <(cd "$R/share/$APP/icons" && ls)
check "icons are identical to assets"          diff -r "$ROOT/assets/icons" "$R/share/$APP/icons"
check "scripts are the packaging sources"      bash -c "cmp '$ROOT/packaging/install.sh' '$R/install.sh' && cmp '$ROOT/packaging/uninstall.sh' '$R/uninstall.sh'"
check "scripts are valid bash"                 bash -c "bash -n '$R/install.sh' && bash -n '$R/uninstall.sh'"
check "no stray files (tmp, backups, dotfiles)" bash -c "! find '$R' \( -name '*.tmp' -o -name '*~' -o -name '.*' \) | grep -q ."
check "no files outside the expected layout"   bash -c "cd '$R' && ! find . -type f | grep -Ev '^\./(bin/$APP|install\.sh|uninstall\.sh|README\.md|LICENSE-ICONS\.md|SHA256SUMS|share/$APP/uninstall\.sh|share/$APP/icons/[a-zA-Z0-9-]+\.png)$'"
check "release size is reasonable (< 10 MB)"   test "$(stat -c %s "$ZIP")" -lt 10000000

echo
echo "package tests: $PASS passed, $FAIL failed"
[ "$FAIL" = 0 ]
