#!/usr/bin/env bash
# Build and assemble the release ZIP:
#   dist/cyberghost-indicator-<version>-x86_64-linux.zip
#     cyberghost-indicator-<version>-x86_64-linux/
#       bin/cyberghost-indicator
#       share/cyberghost-indicator/icons/*.png
#       share/cyberghost-indicator/uninstall.sh
#       install.sh  uninstall.sh  README.md  LICENSE-ICONS.md  SHA256SUMS
set -euo pipefail
cd "$(dirname "$0")/.."

TARGET="${TARGET:-x86_64-unknown-linux-musl}"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
NAME="cyberghost-indicator-${VERSION}-x86_64-linux"
STAGE="dist/$NAME"

TARGET="$TARGET" scripts/build.sh

command -v zip >/dev/null || { echo "error: 'zip' is required (apt install zip)" >&2; exit 1; }

rm -rf "$STAGE" "dist/$NAME.zip"
mkdir -p "$STAGE/bin" "$STAGE/share/cyberghost-indicator"
install -m 755 "target/$TARGET/release/cyberghost-indicator" "$STAGE/bin/"
cp -r assets/icons "$STAGE/share/cyberghost-indicator/icons"
install -m 755 packaging/install.sh packaging/uninstall.sh "$STAGE/"
install -m 755 packaging/uninstall.sh "$STAGE/share/cyberghost-indicator/"
install -m 644 README.md "$STAGE/"
install -m 644 assets/LICENSE-ICONS.md "$STAGE/"

# quick sanity check of the staged binary
"$STAGE/bin/cyberghost-indicator" --version >/dev/null

(cd "$STAGE" && find . -type f ! -name SHA256SUMS -print0 | sort -z | xargs -0 sha256sum > SHA256SUMS)
(cd dist && zip -qr9 "$NAME.zip" "$NAME")
(cd dist && sha256sum "$NAME.zip" > "$NAME.zip.sha256")

echo "package: dist/$NAME.zip ($(du -h "dist/$NAME.zip" | cut -f1))"
