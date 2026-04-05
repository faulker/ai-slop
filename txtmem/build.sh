#!/bin/bash
set -e

INPUT="${1:-Debug}"
CONFIG="$(tr '[:lower:]' '[:upper:]' <<< "${INPUT:0:1}")$(tr '[:upper:]' '[:lower:]' <<< "${INPUT:1}")"
BUILD_DIR="build"

echo "==> Generating Xcode project..."
xcodegen generate

echo "==> Building ThoughtQueue ($CONFIG)..."
xcodebuild -project txtmem.xcodeproj \
  -scheme txtmem \
  -configuration "$CONFIG" \
  -derivedDataPath "$BUILD_DIR" \
  build | tail -5

APP_PATH="$BUILD_DIR/Build/Products/$CONFIG/ThoughtQueue.app"

if [ -f "$APP_PATH/Contents/MacOS/ThoughtQueue" ]; then
  echo "==> Build succeeded: $APP_PATH"
  echo ""
  echo "Run with:  open $APP_PATH"
else
  echo "==> Build failed"
  exit 1
fi
