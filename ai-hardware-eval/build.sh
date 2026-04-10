#!/usr/bin/env bash
set -euo pipefail

VERSION="0.1.0"
NAME="ai-hardware-eval"
OUT_DIR="dist"

rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"

echo "=== Building $NAME v$VERSION ==="
echo ""

# ── macOS (Apple Silicon) ────────────────────────────────────────────────────

MACOS_ARM_TARGET="aarch64-apple-darwin"
echo "Building macOS (Apple Silicon)..."
rustup target add "$MACOS_ARM_TARGET" 2>/dev/null || true
cargo build --release --target "$MACOS_ARM_TARGET"
cp "target/$MACOS_ARM_TARGET/release/$NAME" "$OUT_DIR/${NAME}-macos-arm64"
echo "  -> $OUT_DIR/${NAME}-macos-arm64"

# ── macOS (Intel) ────────────────────────────────────────────────────────────

MACOS_X86_TARGET="x86_64-apple-darwin"
echo "Building macOS (Intel)..."
rustup target add "$MACOS_X86_TARGET" 2>/dev/null || true
cargo build --release --target "$MACOS_X86_TARGET"
cp "target/$MACOS_X86_TARGET/release/$NAME" "$OUT_DIR/${NAME}-macos-x86_64"
echo "  -> $OUT_DIR/${NAME}-macos-x86_64"

# ── Linux x86_64 (static musl binary) ───────────────────────────────────────

LINUX_TARGET="x86_64-unknown-linux-musl"
echo "Building Linux x86_64 (static, musl)..."
rustup target add "$LINUX_TARGET" 2>/dev/null || true

# Find the musl cross-linker
MUSL_LINKER=""
for candidate in x86_64-linux-musl-gcc x86_64-unknown-linux-musl-gcc musl-gcc; do
    if command -v "$candidate" &>/dev/null; then
        MUSL_LINKER="$candidate"
        break
    fi
done

if [ -z "$MUSL_LINKER" ]; then
    echo ""
    echo "  ERROR: No musl cross-linker found. Install with:"
    echo ""
    echo "    brew install filosottile/musl-cross/musl-cross"
    echo ""
    exit 1
fi

echo "  Using linker: $MUSL_LINKER"
CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER="$MUSL_LINKER" \
    CC_x86_64_unknown_linux_musl="$MUSL_LINKER" \
    cargo build --release --target "$LINUX_TARGET"
cp "target/$LINUX_TARGET/release/$NAME" "$OUT_DIR/${NAME}-linux-x86_64"
echo "  -> $OUT_DIR/${NAME}-linux-x86_64"

# ── Summary ──────────────────────────────────────────────────────────────────

echo ""
echo "=== Build complete ==="
echo ""
ls -lh "$OUT_DIR"/
