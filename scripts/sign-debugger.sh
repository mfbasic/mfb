#!/bin/sh
# Ad-hoc sign an mfb binary with the macOS debugger entitlement, so `mfb dbg` may
# take the task port of the program it launches or attaches to (task_for_pid).
# Without it macOS refuses unless mfb runs as root. Every `cargo build` relinks
# mfb and drops the signature, so run this again after each build.
#
# Usage: sh scripts/sign-debugger.sh [path/to/mfb ...]
#        (default: target/debug/mfb and target/release/mfb, whichever exist)
set -eu

if [ "$(uname -s)" != "Darwin" ]; then
    echo "sign-debugger.sh: mfb dbg needs no signing outside macOS" >&2
    exit 1
fi

if [ "$#" -eq 0 ]; then
    root=$(cd "$(dirname "$0")/.." && pwd)
    for candidate in "$root/target/debug/mfb" "$root/target/release/mfb"; do
        if [ -f "$candidate" ]; then
            set -- "$@" "$candidate"
        fi
    done
    if [ "$#" -eq 0 ]; then
        echo "sign-debugger.sh: no target/debug/mfb or target/release/mfb; build first" >&2
        exit 1
    fi
fi

entitlements=$(mktemp -t mfb-debugger-entitlements)
trap 'rm -f "$entitlements"' EXIT
cat > "$entitlements" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.security.cs.debugger</key>
    <true/>
</dict>
</plist>
PLIST

for mfb in "$@"; do
    codesign --sign - --force --entitlements "$entitlements" "$mfb"
    echo "signed $mfb for mfb dbg"
done
