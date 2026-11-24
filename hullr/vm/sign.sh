#!/bin/sh
# Ad-hoc sign the hullr binary with the virtualization entitlement, which
# Virtualization.framework requires. Run after every `cargo build` on macOS:
#   vm/sign.sh target/release/hullr
set -eu
here=$(cd "$(dirname "$0")" && pwd)
bin=${1:-target/release/hullr}
codesign --entitlements "$here/hullr.entitlements" --force -s - "$bin"
codesign -d --entitlements - "$bin" 2>/dev/null | grep -q com.apple.security.virtualization \
    && echo "signed $bin with com.apple.security.virtualization"
