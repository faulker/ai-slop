#!/bin/bash
set -e

CONFIG="${1:-Debug}"
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
