#!/usr/bin/env bash
# Builds build/DisableIMusic.app. Pass --install to also copy it into /Applications.
set -euo pipefail
cd "$(dirname "$0")"

swift build -c release
BIN="$(swift build -c release --show-bin-path)/DisableIMusic"
APP=build/DisableIMusic.app

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp "$BIN" "$APP/Contents/MacOS/DisableIMusic"
cp Resources/Info.plist "$APP/Contents/Info.plist"
codesign --force --sign - "$APP"
echo "Built $APP"

if [[ "${1:-}" == "--install" ]]; then
    pkill -x DisableIMusic || true
    rm -rf /Applications/DisableIMusic.app
    cp -R "$APP" /Applications/
    echo "Installed to /Applications/DisableIMusic.app"
fi
