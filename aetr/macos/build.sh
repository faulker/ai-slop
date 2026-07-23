#!/bin/bash
# Builds the Aetr macOS app: Rust static lib + bindings first, then
# XcodeGen, then xcodebuild. Usage: ./build.sh [Debug|Release]

set -euo pipefail

INPUT="${1:-Debug}"
CONFIG="$(tr '[:lower:]' '[:upper:]' <<< "${INPUT:0:1}")$(tr '[:upper:]' '[:lower:]' <<< "${INPUT:1}")"
BUILD_DIR="build"

# Remember where the user invoked us so the final path is relative to that.
INVOKE_DIR="$PWD"
cd "$(dirname "$0")"
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"

echo "==> Building Rust core + bindings..."
./build-rust.sh

echo "==> Generating Xcode project..."
xcodegen generate

echo "==> Building Aetr ($CONFIG)..."
set +e
xcodebuild -project Aetr.xcodeproj \
    -scheme Aetr \
    -configuration "$CONFIG" \
    -derivedDataPath "$BUILD_DIR" \
    CODE_SIGN_IDENTITY="-" \
    build > build_output.log 2>&1
XCODE_EXIT=$?
set -e

# SYMROOT in project.yml puts products directly under build/<config>/.
APP_PATH="$BUILD_DIR/$CONFIG/Aetr.app"

if [ $XCODE_EXIT -eq 0 ] && [ -f "$APP_PATH/Contents/MacOS/Aetr" ]; then
    # Path relative to where the script was invoked, so it's copy-pasteable.
    REL_APP="$(python3 -c 'import os,sys; print(os.path.relpath(sys.argv[1], sys.argv[2]))' \
        "$PWD/$APP_PATH" "$INVOKE_DIR")"
    echo "==> Build succeeded: $REL_APP"
    echo ""
    echo "Run with:  open \"$REL_APP\""
else
    echo "==> Build failed. Last 30 lines of build_output.log:"
    tail -n 30 build_output.log
    exit 1
fi
