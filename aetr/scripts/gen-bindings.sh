#!/usr/bin/env bash
# Generates the UniFFI bindings for both platform apps:
#   Swift  -> macos/Generated/
#   Kotlin -> android/app/src/main/java/ (package uniffi.aetr_core)
# Builds the aetr-core cdylib first and runs uniffi-bindgen in library mode
# against it, so the bindings always match the compiled surface.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

SWIFT_OUT="$ROOT/macos/Generated"
KOTLIN_OUT="$ROOT/android/app/src/main/java"

case "$(uname -s)" in
    Darwin) LIB="$ROOT/target/release/libaetr_core.dylib" ;;
    *)      LIB="$ROOT/target/release/libaetr_core.so" ;;
esac

echo "==> building aetr-core cdylib (release)"
cargo build -p aetr-core --release

if [[ ! -f "$LIB" ]]; then
    echo "error: expected cdylib at $LIB after build" >&2
    exit 1
fi

mkdir -p "$SWIFT_OUT" "$KOTLIN_OUT"

echo "==> generating Swift bindings into $SWIFT_OUT"
cargo run -p aetr-core --features cli --bin uniffi-bindgen -- \
    generate --library "$LIB" --language swift --out-dir "$SWIFT_OUT"

echo "==> generating Kotlin bindings into $KOTLIN_OUT"
cargo run -p aetr-core --features cli --bin uniffi-bindgen -- \
    generate --library "$LIB" --language kotlin --out-dir "$KOTLIN_OUT"

# Fail loudly if either generator silently produced nothing.
for f in "$SWIFT_OUT/aetr_core.swift" \
         "$SWIFT_OUT/aetr_coreFFI.h" \
         "$KOTLIN_OUT/uniffi/aetr_core/aetr_core.kt"; do
    if [[ ! -f "$f" ]]; then
        echo "error: expected generated file missing: $f" >&2
        exit 1
    fi
done

echo "==> bindings generated:"
ls -1 "$SWIFT_OUT"
ls -1 "$KOTLIN_OUT/uniffi/aetr_core"
