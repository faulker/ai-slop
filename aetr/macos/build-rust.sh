#!/bin/bash
# Builds the aetr-core Rust static library for the macOS app and refreshes
# the UniFFI Swift bindings. Runs standalone or as an Xcode pre-build script
# (Xcode sets SRCROOT; standalone falls back to the script's directory).
#
# Produces macos/libaetr_core.a: universal (arm64 + x86_64) when both Apple
# targets are installed in rustup, otherwise arm64-only.

set -euo pipefail

SRCROOT="${SRCROOT:-$(cd "$(dirname "$0")" && pwd)}"
ROOT="$(cd "$SRCROOT/.." && pwd)"

export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"

# Regenerate the UniFFI bindings first (this also builds the host cdylib
# that uniffi-bindgen's library mode reads), so Generated/ always matches
# the compiled Rust surface.
"$ROOT/scripts/gen-bindings.sh"

# Per-architecture static libs.
TARGETS=(aarch64-apple-darwin)
if rustup target list --installed | grep -q '^x86_64-apple-darwin$'; then
    TARGETS+=(x86_64-apple-darwin)
fi

LIBS=()
for target in "${TARGETS[@]}"; do
    echo "==> cargo build --release --target $target"
    cargo build --manifest-path "$ROOT/Cargo.toml" -p aetr-core --release --target "$target"
    LIBS+=("$ROOT/target/$target/release/libaetr_core.a")
done

# Universal lib when we have both slices, plain copy otherwise.
if [ "${#LIBS[@]}" -gt 1 ]; then
    echo "==> lipo -create -> $SRCROOT/libaetr_core.a (universal)"
    lipo -create "${LIBS[@]}" -output "$SRCROOT/libaetr_core.a"
else
    echo "==> copying arm64-only libaetr_core.a"
    cp "${LIBS[0]}" "$SRCROOT/libaetr_core.a"
fi

lipo -info "$SRCROOT/libaetr_core.a"
