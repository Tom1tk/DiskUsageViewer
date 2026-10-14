#!/bin/bash
# Builds a release binary and wraps it in DiskUsageViewer.app, signed ad hoc (no developer account needed).
set -euo pipefail

cd "$(dirname "$0")"
swift build -c release
BIN_DIR="$(swift build -c release --show-bin-path)"

APP="DiskUsageViewer.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp Info.plist "$APP/Contents/Info.plist"
cp "$BIN_DIR/DiskUsageViewer" "$APP/Contents/MacOS/DiskUsageViewer"
codesign --force --sign - "$APP"
echo "Built $(pwd)/$APP"
