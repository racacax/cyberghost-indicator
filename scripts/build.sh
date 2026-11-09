#!/usr/bin/env bash
# Build the release binary.
#   scripts/build.sh            static binary (x86_64-unknown-linux-musl), runs on any distro
#   TARGET=x86_64-unknown-linux-gnu scripts/build.sh   glibc build (this machine's glibc or newer)
# Toolchain comes from rust-toolchain.toml (installed by rustup on first use).
set -euo pipefail
cd "$(dirname "$0")/.."

TARGET="${TARGET:-x86_64-unknown-linux-musl}"

if ! command -v cargo >/dev/null 2>&1; then
    if [ -x "$HOME/.cargo/bin/cargo" ]; then
        export PATH="$HOME/.cargo/bin:$PATH"
    else
        echo "error: cargo not found. Install rustup (https://rustup.rs), then run 'make setup'." >&2
        exit 1
    fi
fi

# rust-toolchain.toml lists the targets; make sure it is there for older rustup.
if command -v rustup >/dev/null 2>&1; then
    rustup target list --installed | grep -qx "$TARGET" || rustup target add "$TARGET"
fi

cargo build --release --locked --target "$TARGET"

BIN="target/$TARGET/release/cyberghost-indicator"
echo "built: $BIN ($(du -h "$BIN" | cut -f1))"
