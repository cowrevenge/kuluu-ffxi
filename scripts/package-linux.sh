#!/usr/bin/env bash
set -euo pipefail
binary=$1
archive=$2
repo=$(cd "$(dirname "$0")/.." && pwd)
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
app_id=io.github.jondwillis.kuluu
install -m 755 "$binary" "$stage/kuluu"
install -m 755 "$repo/packaging/linux/install-local.sh" "$stage/install-local.sh"
mkdir -p "$stage/share/applications" "$stage/share/metainfo"
cp "$repo/packaging/linux/$app_id.desktop" "$stage/share/applications/"
cp "$repo/packaging/linux/$app_id.metainfo.xml" "$stage/share/metainfo/"
for size in 16 24 32 48 64 128 256 512; do
    destination="$stage/share/icons/hicolor/${size}x${size}/apps"
    mkdir -p "$destination"
    cp "$repo/kuluu/assets/branding/png/kuluu-$size.png" "$destination/$app_id.png"
done
tar -C "$stage" -czf "$archive" kuluu install-local.sh share
