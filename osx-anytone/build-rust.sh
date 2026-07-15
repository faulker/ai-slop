#!/bin/bash
# Builds the anytone-core Rust static library (release) and copies it next to
# the Xcode project so the AnyToneMac target can link it. Runs standalone or
# as an Xcode pre-build phase (where Xcode sets SRCROOT/ARCHS).

set -euo pipefail

SRCROOT="${SRCROOT:-$(cd "$(dirname "$0")" && pwd)}"
export PATH="$HOME/.cargo/bin:$PATH"

# Pick the Rust target from Xcode's ARCHS when present, else the host arch.
if [[ "${ARCHS:-}" == *"arm64"* ]]; then
    TARGET="aarch64-apple-darwin"
elif [[ "${ARCHS:-}" == *"x86_64"* ]]; then
    TARGET="x86_64-apple-darwin"
elif [ "$(uname -m)" = "arm64" ]; then
    TARGET="aarch64-apple-darwin"
else
    TARGET="x86_64-apple-darwin"
fi

echo "Building anytone-core (release) for $TARGET..."
cd "$SRCROOT"
cargo build --release -p anytone-core --target "$TARGET"

cp "target/$TARGET/release/libanytone_core.a" "$SRCROOT/libanytone_core.a"
echo "Copied libanytone_core.a to $SRCROOT"
