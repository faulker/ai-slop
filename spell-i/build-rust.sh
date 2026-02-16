#!/bin/bash
# Xcode Run Script build phase — builds the Rust FFI library.
# Must run BEFORE "Compile Sources".

set -euo pipefail

# When invoked from Xcode, SRCROOT is set. For manual builds, default to script dir.
SRCROOT="${SRCROOT:-$(cd "$(dirname "$0")" && pwd)}"
CRATE_DIR="$SRCROOT/spell-i-engine"

# Ensure cargo is on PATH
export PATH="$HOME/.cargo/bin:$PATH"

# Determine build profile
if [ "${CONFIGURATION:-Debug}" = "Release" ]; then
    PROFILE="release"
    CARGO_FLAGS="--release"
else
    PROFILE="debug"
    CARGO_FLAGS=""
fi

# Detect architecture from Xcode or system
# If ARCHS is set by Xcode, it might contain multiple values (e.g., "arm64 x86_64").
# We pick the first one that matches our capability or just the first one.
if [[ "${ARCHS:-}" == *"arm64"* ]]; then
    TARGET="aarch64-apple-darwin"
elif [[ "${ARCHS:-}" == *"x86_64"* ]]; then
    TARGET="x86_64-apple-darwin"
else
    # Fallback to system architecture
    SYS_ARCH=$(uname -m)
    if [ "$SYS_ARCH" = "arm64" ]; then
        TARGET="aarch64-apple-darwin"
    else
        TARGET="x86_64-apple-darwin"
    fi
fi

echo "Building spell-i-engine ($PROFILE) for $TARGET..."
echo "SRCROOT is: $SRCROOT"
mkdir -p "$SRCROOT/Generated"
cd "$CRATE_DIR"
echo "Current directory in Rust build: $(pwd)"
cargo build $CARGO_FLAGS --target "$TARGET"

# Copy static library to SRCROOT so Xcode can find it
cp "target/$TARGET/$PROFILE/libspell_i_engine.a" "$SRCROOT/libspell_i_engine.a"
echo "Copied libspell_i_engine.a to $SRCROOT"
ls -la "$SRCROOT/Generated"
