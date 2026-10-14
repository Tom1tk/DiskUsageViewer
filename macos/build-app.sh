#!/bin/bash
# Builds a release binary and wraps it in DiskUsageViewer.app, signed ad hoc (no developer account needed).
set -euo pipefail

cd "$(dirname "$0")"
# rustup installs cargo under ~/.cargo/bin and does not change the shell's PATH.
if ! command -v cargo >/dev/null && [ -f "$HOME/.cargo/env" ]; then
    . "$HOME/.cargo/env"
fi
cargo build --release

APP="DiskUsageViewer.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp Info.plist "$APP/Contents/Info.plist"
cp target/release/DiskUsageViewer "$APP/Contents/MacOS/DiskUsageViewer"
codesign --force --sign - "$APP"
echo "Built $(pwd)/$APP"
