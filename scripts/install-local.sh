#!/bin/bash
# Keep the development signing identity on this Mac across updates.
set -euo pipefail
cd "$(dirname "$0")/.."
built=${1:-src-tauri/target/release/bundle/macos/Beside.app}
target="$HOME/Applications/Beside.app"
installed="$target"
legacy="$HOME/Applications/Sumi.app"
if [[ ! -e "$target" && -d "$legacy" && ! -L "$legacy" ]]; then
  installed="$legacy"
fi
test -x "$built/Contents/MacOS/sumi"
identity=${BESIDE_LOCAL_SIGN_IDENTITY:-${SUMI_LOCAL_SIGN_IDENTITY:-}}
if [[ -z "$identity" ]]; then
  entries=$(security find-identity -v -p codesigning | awk '/"Apple Development:/ {print}')
  names=$(echo "$entries" | cut -d'"' -f2 | sort -u)
  if [[ -z "$names" || "$names" = *$'\n'* ]]; then
    echo 'Choose your Apple Development identity with BESIDE_LOCAL_SIGN_IDENTITY.' >&2
    exit 1
  fi
  identity=$(echo "$entries" | awk 'NR==1{print $2}')
fi
[[ "$identity" != "-" ]] || exit 1
mkdir -p "$HOME/Applications"
cache="$HOME/Library/Caches/studio.sumi.notebook"
mkdir -p "$cache"
stage=$(mktemp -d "$cache/signed-update-XXXXXXXX")
new="$stage/Beside.app"
ditto --norsrc --noextattr "$built" "$new"
for item in "$new/Contents/Resources/runtime/"*.dylib "$new/Contents/Resources/runtime/llama-server" "$new/Contents/Resources/runtime/beside-confidential"; do
  codesign --force --sign "$identity" --timestamp=none --options runtime "$item"
done
codesign --force --sign "$identity" --timestamp=none --options runtime --entitlements src-tauri/entitlements.plist "$new"
codesign --verify --deep --strict "$new"
if [[ -d "$installed" ]]; then
  oldmeta=$(codesign -dv --verbose=3 "$installed" 2>&1)
  if ! grep -q '^Signature=adhoc$' <<<"$oldmeta"; then
    requirement=$(codesign -d -r- "$installed" 2>&1 | sed -n 's/^designated => //p')
    [[ -n "$requirement" ]]
    codesign --verify -R="$requirement" "$new"
  fi
fi
swift -e 'import AppKit
let path = CommandLine.arguments[1]
let apps = NSWorkspace.shared.runningApplications.filter { $0.bundleURL?.path == path }
for app in apps { guard app.terminate() else { exit(1) } }
for _ in 0..<100 {
    if apps.allSatisfy({$0.isTerminated}) { exit(0) }
    RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.1))
}
exit(1)' "$installed"
python3 - "$new" "$installed" "$target" <<'PY'
from pathlib import Path
import ctypes, os, sys
new, target, branded = map(Path, sys.argv[1:])
if target.exists():
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.renamex_np(os.fsencode(new), os.fsencode(target), 2):
        raise OSError(ctypes.get_errno(), "Could not swap Beside bundles")
    new.rename(new.parent / "previous.bundle")
else:
    new.rename(target)
if target != branded:
    target.rename(branded)
    try:
        # Existing agent configurations may refer to the former executable path.
        target.symlink_to(branded.name)
    except OSError:
        branded.rename(target)
        raise
PY
open "$target"
echo "Installed $target"
if [[ -d "$stage/previous.bundle" ]]; then echo "Previous build: $stage/previous.bundle"; else rmdir "$stage"; fi
