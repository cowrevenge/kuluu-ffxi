#!/usr/bin/env bash
set -euo pipefail
source_dir=$(cd "$(dirname "$0")" && pwd)
data_dir=${XDG_DATA_HOME:-"$HOME/.local/share"}
bin_dir="$HOME/.local/bin"
app_id=io.github.jondwillis.kuluu
mkdir -p "$bin_dir" "$data_dir/applications" "$data_dir/metainfo"
install -m 755 "$source_dir/kuluu" "$bin_dir/kuluu"
install -m 644 "$source_dir/share/applications/$app_id.desktop" "$data_dir/applications/"
install -m 644 "$source_dir/share/metainfo/$app_id.metainfo.xml" "$data_dir/metainfo/"
python3 - "$data_dir/applications/$app_id.desktop" "$bin_dir/kuluu" <<'PY'
from pathlib import Path
import sys
desktop = Path(sys.argv[1])
executable = sys.argv[2]
for char in ('\\', '"', '`', '$'):
    executable = executable.replace(char, '\\' + char)
executable = executable.replace('\\', '\\\\').replace('%', '%%')
desktop.write_text(desktop.read_text().replace('Exec=kuluu play', f'Exec="{executable}" play'))
PY
for size in 16 24 32 48 64 128 256 512; do
    destination="$data_dir/icons/hicolor/${size}x${size}/apps"
    mkdir -p "$destination"
    install -m 644 "$source_dir/share/icons/hicolor/${size}x${size}/apps/$app_id.png" "$destination/"
done
if command -v update-desktop-database >/dev/null; then
    update-desktop-database "$data_dir/applications"
fi
echo "Installed Kuluu in $bin_dir and added its desktop launcher."
