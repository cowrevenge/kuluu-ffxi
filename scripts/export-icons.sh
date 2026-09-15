#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

brand=kuluu/assets/branding
master="$brand/kuluu-master.png"
mkdir -p "$brand/png" kuluu-viewer-wasm/branding
for size in 16 24 32 48 64 128 180 192 256 512 1024; do
    sips -z "$size" "$size" "$master" --out "$brand/png/kuluu-$size.png" >/dev/null
done
python3 - <<'PY'
from pathlib import Path
import struct

brand = Path('kuluu/assets/branding')
sizes = (16, 24, 32, 48, 64, 128, 256)
payloads = [(brand / 'png' / f'kuluu-{size}.png').read_bytes() for size in sizes]
offset = 6 + 16 * len(sizes)
entries = []
for size, payload in zip(sizes, payloads):
    entries.append(struct.pack('<BBBBHHII', size % 256, size % 256, 0, 0, 1, 32, len(payload), offset))
    offset += len(payload)
(brand / 'kuluu.ico').write_bytes(struct.pack('<HHH', 0, 1, len(sizes)) + b''.join(entries) + b''.join(payloads))
chunks = []
for tag, size in ((b'icp4', 16), (b'icp5', 32), (b'icp6', 64), (b'ic07', 128),
                  (b'ic08', 256), (b'ic09', 512), (b'ic10', 1024)):
    payload = (brand / 'png' / f'kuluu-{size}.png').read_bytes()
    chunks.append(tag + struct.pack('>I', len(payload) + 8) + payload)
body = b''.join(chunks)
(brand / 'kuluu.icns').write_bytes(b'icns' + struct.pack('>I', len(body) + 8) + body)
PY
for size in 32 180 192 512; do
    cp "$brand/png/kuluu-$size.png" "kuluu-viewer-wasm/branding/kuluu-$size.png"
done
