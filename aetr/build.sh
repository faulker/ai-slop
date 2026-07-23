#!/bin/bash
# Top-level build for Aetr: compiles the macOS app (Aetr.app) and the
# Android APK. Each platform builds the shared aetr-core Rust crate and its
# UniFFI bindings as part of its own flow (macos/build.sh, gradle cargoNdk),
# so this script just orchestrates the two.
#
# Usage:
#   ./build.sh                 # build both, Debug
#   ./build.sh Release         # build both, Release
#   ./build.sh --macos         # macOS only
#   ./build.sh --android       # Android only
#   ./build.sh --android Release
#
# macOS builds require Xcode + xcodegen; Android builds require the Android
# SDK/NDK and cargo-ndk. Missing a toolchain for a requested platform is a
# hard error.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT"
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"

BUILD_MACOS=true
BUILD_ANDROID=true
CONFIG_INPUT="Debug"

for arg in "$@"; do
    case "$arg" in
        --macos|--macos-only)     BUILD_ANDROID=false ;;
        --android|--android-only) BUILD_MACOS=false ;;
        Debug|debug|Release|release) CONFIG_INPUT="$arg" ;;
        *) echo "unknown argument: $arg" >&2; exit 2 ;;
    esac
done

# Normalize to capitalized config (Debug/Release) for both toolchains.
CONFIG="$(tr '[:lower:]' '[:upper:]' <<< "${CONFIG_INPUT:0:1}")$(tr '[:upper:]' '[:lower:]' <<< "${CONFIG_INPUT:1}")"

build_macos() {
    echo "==> Building macOS app ($CONFIG)..."
    ./macos/build.sh "$CONFIG"
}

build_android() {
    echo "==> Building Android APK ($CONFIG)..."

    # Gradle 8.14 / the Android toolchain need a JDK it can parse. Newer JDKs
    # (e.g. 26) break the bundled Kotlin compiler with "IllegalArgumentException:
    # <version>", so pin JAVA_HOME to a JDK 17 when the default java isn't 17.
    if [ -z "${JAVA_HOME:-}" ] || ! "$JAVA_HOME/bin/java" -version 2>&1 | grep -q 'version "17'; then
        local jdk17=""
        for cand in \
            /opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home \
            /usr/local/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home \
            /Library/Java/JavaVirtualMachines/*17*/Contents/Home; do
            [ -x "$cand/bin/java" ] && { jdk17="$cand"; break; }
        done
        if [ -z "$jdk17" ]; then
            echo "==> No JDK 17 found. Install one: brew install openjdk@17" >&2
            exit 1
        fi
        export JAVA_HOME="$jdk17"
        echo "==> Using JDK 17 at $JAVA_HOME"
    fi

    # assembleRelease needs a signing config the project doesn't define, so
    # only Debug is wired up here; anything else falls back to a plain
    # debug assemble.
    local task="assembleDebug"
    [ "$CONFIG" = "Release" ] && task="assembleRelease"
    ./android/gradlew -p android "$task"

    local apk
    apk="$(ls -t android/app/build/outputs/apk/*/*.apk 2>/dev/null | head -n1 || true)"
    if [ -n "$apk" ]; then
        echo "==> APK: $ROOT/$apk"
    else
        echo "==> Android build reported success but no APK was found" >&2
        exit 1
    fi
}

$BUILD_MACOS && build_macos
$BUILD_ANDROID && build_android

echo ""
echo "==> Done."
