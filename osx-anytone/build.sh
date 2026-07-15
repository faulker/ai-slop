#!/bin/bash
# Unified build for AnyToneMac: builds the anytone-core Rust staticlib
# (release), regenerates the Xcode project with XcodeGen, and builds the app
# with xcodebuild. Signing is ad-hoc/local; no notarization.

set -euo pipefail

PROJECT_NAME="AnyToneMac"
CONFIGURATION="${CONFIGURATION:-Debug}"

cd "$(dirname "$0")"
export SRCROOT="$(pwd)"
BUILD_DIR="$SRCROOT/build"
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"

# 1. Rust static library (release).
./build-rust.sh

# 2. Xcode project from project.yml.
if ! command -v xcodegen >/dev/null 2>&1; then
    echo "error: xcodegen not found (brew install xcodegen)" >&2
    exit 1
fi
xcodegen generate --spec project.yml

# 3. Swift app.
echo "Building $PROJECT_NAME ($CONFIGURATION) with xcodebuild..."
if ! xcodebuild \
    -project "$PROJECT_NAME.xcodeproj" \
    -scheme "$PROJECT_NAME" \
    -configuration "$CONFIGURATION" \
    -derivedDataPath "$BUILD_DIR" \
    -arch "$(uname -m)" \
    ONLY_ACTIVE_ARCH=YES \
    CODE_SIGN_IDENTITY="-" \
    build > build_output.log 2>&1; then
    echo "xcodebuild failed; last 30 lines of build_output.log:" >&2
    tail -n 30 build_output.log >&2
    exit 1
fi

echo "Build succeeded."
APP_PATH=$(find "$BUILD_DIR" -name "$PROJECT_NAME.app" -type d | head -n 1)
echo "App: $APP_PATH"
