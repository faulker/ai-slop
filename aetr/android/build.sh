#!/bin/bash
# Builds the Aetr Android app and prints the relative path to the APK.
# The Gradle build's cargoNdk task cross-compiles aetr-core into jniLibs
# first, so this single command produces a complete APK.
# Usage: ./build.sh [assembleDebug|assembleRelease]  (default: assembleDebug)

set -euo pipefail

TASK="${1:-assembleDebug}"

# Remember where the user invoked us so the final path is relative to that.
INVOKE_DIR="$PWD"
cd "$(dirname "$0")"

echo "==> ./gradlew $TASK"
./gradlew "$TASK"

# Grab the most recently produced APK for the requested build type.
APK="$(find app/build/outputs/apk -name '*.apk' -type f -print0 2>/dev/null \
    | xargs -0 ls -t 2>/dev/null | head -n1 || true)"

if [ -n "$APK" ] && [ -f "$APK" ]; then
    # Path relative to where the script was invoked, so it's copy-pasteable.
    REL_APK="$(python3 -c 'import os,sys; print(os.path.relpath(sys.argv[1], sys.argv[2]))' \
        "$PWD/$APK" "$INVOKE_DIR")"
    echo "==> Build succeeded: $REL_APK"
    echo ""
    echo "Install with:  adb install -r \"$REL_APK\""
else
    echo "==> Build finished but no APK was found under app/build/outputs/apk."
    exit 1
fi
