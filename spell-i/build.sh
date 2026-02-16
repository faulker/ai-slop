#!/bin/bash

# ==============================================================================
# Spell-i Unified Build Script
# Compiles Rust engine, generates FFI bridge, and builds the macOS app.
# ==============================================================================

set -euo pipefail

# --- Configuration ---
PROJECT_NAME="Spell-i"
SCHEME="Spell-i"
CONFIGURATION="Debug"
BUILD_DIR="$(pwd)/build"
export SRCROOT="$(pwd)"

echo "🚀 Starting Unified Build for $PROJECT_NAME ($CONFIGURATION)..."

# 1. Setup Environment & Path
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"

# 2. Pre-create Generated directory structure
echo "📂 Preparing Generated directory structure..."
mkdir -p "$SRCROOT/Generated/spell-i-engine"

# 3. Build Rust Engine & Generate Bridge
# We run this BEFORE xcodegen/xcodebuild so the files definitely exist.
echo "🦀 Building Rust engine (spell-i-engine)..."

# If Generated is empty, we force a rebuild to ensure files are created
if [ ! -f "$SRCROOT/Generated/SwiftBridgeCore.swift" ]; then
    echo "⚠️  Generated files missing, forcing Rust bridge regeneration..."
    (cd spell-i-engine && touch src/lib.rs)
fi

./build-rust.sh

# Verify files exist
if [ ! -f "$SRCROOT/Generated/SwiftBridgeCore.swift" ]; then
    echo "❌ Error: Rust build completed but Generated/SwiftBridgeCore.swift is still missing!"
    exit 1
fi

# 4. Re-generate Xcode project
if command -v xcodegen >/dev/null 2>&1; then
    echo "🔨 Generating Xcode project with xcodegen..."
    xcodegen generate --spec project.yml
else
    echo "⚠️  xcodegen not found, skipping project generation."
fi

# 5. Build Swift Application
echo "🍎 Building macOS application with xcodebuild..."
ARCH=$(uname -m)

set +e
xcodebuild \
    -project "$PROJECT_NAME.xcodeproj" \
    -scheme "$SCHEME" \
    -configuration "$CONFIGURATION" \
    -derivedDataPath "$BUILD_DIR" \
    -arch "$ARCH" \
    ONLY_ACTIVE_ARCH=YES \
    CODE_SIGN_IDENTITY="-" \
    CODE_SIGNING_REQUIRED=YES \
    build > build_output.log 2>&1
XCODE_EXIT_CODE=$?
set -e

# 6. Reporting
if [ $XCODE_EXIT_CODE -eq 0 ]; then
    echo "✅ Build Successful!"
    APP_PATH=$(find "$BUILD_DIR" -name "$PROJECT_NAME.app" -type d | head -n 1)
    echo "📍 App location: $APP_PATH"
else
    echo "❌ Build Failed."
    echo "--------------------------------------------------------------------------------"
    echo "Check build_output.log for errors."
    echo "Last 20 lines:"
    tail -n 20 build_output.log
    echo "--------------------------------------------------------------------------------"
    exit 1
fi
