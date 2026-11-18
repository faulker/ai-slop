#!/bin/bash
# Builds claude-usage in release mode, copies it to ~/.local/bin, and signs it
# with a stable code-signing identity so macOS Keychain "Always Allow" survives
# rebuilds. Signing is skipped (with a warning) if the identity doesn't exist.
#
# Env:
#   CLAUDE_USAGE_SIGN_IDENTITY  substring of a code-signing identity name, or its SHA-1
#                               hash (default: "Apple Development")
#   CLAUDE_USAGE_INSTALL_DIR    install directory (default: ~/.local/bin)
set -euo pipefail

cd "$(dirname "$0")"

IDENTITY="${CLAUDE_USAGE_SIGN_IDENTITY:-Apple Development}"
INSTALL_DIR="${CLAUDE_USAGE_INSTALL_DIR:-$HOME/.local/bin}"
BUNDLE_ID="me.faulk.claude-usage"
DEST="$INSTALL_DIR/claude-usage"

cargo build --release

mkdir -p "$INSTALL_DIR"
# Copy to a temp name and rename, so a status bar running mid-install never sees a partial file
cp target/release/claude-usage "$DEST.tmp"

# Resolve to a SHA-1 hash: a bare name is ambiguous to codesign when several certs share it
# (e.g. a renewed Apple Development cert). Any of them works: an Apple-issued cert's designated
# requirement is identifier + Apple anchor + cert common name, which is what the Keychain ACL keys on.
SIGN_LINE=$(security find-identity -v -p codesigning | grep -F "$IDENTITY" | head -1 || true)
if [ -n "$SIGN_LINE" ]; then
  SIGN_HASH=$(echo "$SIGN_LINE" | awk '{print $2}')
  codesign --force --sign "$SIGN_HASH" --identifier "$BUNDLE_ID" "$DEST.tmp"
  echo "Signed with $(echo "$SIGN_LINE" | cut -d'"' -f2) ($SIGN_HASH)"
else
  echo "warning: code-signing identity \"$IDENTITY\" not found; installing unsigned." >&2
  echo "  Keychain will prompt again after every install. See README: Avoiding the Keychain password prompt." >&2
fi

mv "$DEST.tmp" "$DEST"
echo "Installed $DEST"
