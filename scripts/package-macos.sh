#!/usr/bin/env bash
set -euo pipefail
binary=$1
archive=$2
version=${3#v}
version=${version%%[-+]*}
repo=$(cd "$(dirname "$0")/.." && pwd)
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
contents="$stage/Kuluu.app/Contents"
mkdir -p "$contents/MacOS" "$contents/Resources"
install -m 755 "$binary" "$stage/kuluu"
install -m 755 "$binary" "$contents/MacOS/kuluu"
cp "$repo/kuluu/assets/branding/kuluu.icns" "$contents/Resources/kuluu.icns"
python3 - "$contents/Info.plist" "$version" <<'PY'
import plistlib
import sys
from pathlib import Path

Path(sys.argv[1]).write_bytes(plistlib.dumps({
    'CFBundleName': 'Kuluu',
    'CFBundleDisplayName': 'Kuluu',
    'CFBundleIdentifier': 'io.github.jondwillis.kuluu',
    'CFBundleExecutable': 'kuluu',
    'CFBundleIconFile': 'kuluu.icns',
    'CFBundlePackageType': 'APPL',
    'CFBundleShortVersionString': sys.argv[2],
    'CFBundleVersion': sys.argv[2],
    'LSMinimumSystemVersion': '14.0',
    'NSHighResolutionCapable': True,
}))
PY
codesign --force --sign - "$stage/Kuluu.app"
tar -C "$stage" -czf "$archive" kuluu Kuluu.app
