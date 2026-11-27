#!/usr/bin/env bash
set -euo pipefail

DEST="${1:-/Applications/Peedle.app}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/target/release/peedle"

if [ ! -x "$BIN" ]; then
    cargo build --release --bin peedle --manifest-path "$ROOT/Cargo.toml"
fi

TMP_ICONSET="$(mktemp -d)/AppIcon.iconset"
mkdir -p "$TMP_ICONSET"

python3 - "$TMP_ICONSET" <<'PY'
import math, os, struct, sys, zlib

out_dir = sys.argv[1]

def write_png(path, w, h, rgba):
    raw = bytearray()
    stride = w * 4
    for y in range(h):
        raw.append(0)
        raw.extend(rgba[y * stride:(y + 1) * stride])
    def chunk(tag, data):
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xffffffff)
    ihdr = struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)
    data = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(bytes(raw), 9)) + chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(data)

def clamp(v, lo=0.0, hi=1.0):
    return lo if v < lo else (hi if v > hi else v)

def mix(a, b, t):
    return a + (b - a) * t

def sdf_squircle(x, y, half, r):
    ax = abs(x) - (half - r)
    ay = abs(y) - (half - r)
    n = 2.6
    ox = max(ax, 0.0)
    oy = max(ay, 0.0)
    outside = (ox ** n + oy ** n) ** (1.0 / n) if (ox > 0.0 or oy > 0.0) else 0.0
    inside = min(max(ax, ay), 0.0)
    return outside + inside - r

def sdf_rrect(x, y, cx, cy, hw, hh, r):
    dx = abs(x - cx) - (hw - r)
    dy = abs(y - cy) - (hh - r)
    return math.hypot(max(dx, 0.0), max(dy, 0.0)) + min(max(dx, dy), 0.0) - r

def render_icon(size):
    buf = bytearray(size * size * 4)
    inv = 1.0 / size
    aa = 1.4 * inv
    for py in range(size):
        ny = (py + 0.5) * inv * 2.0 - 1.0
        for px in range(size):
            nx = (px + 0.5) * inv * 2.0 - 1.0
            d_bg = sdf_squircle(nx, ny, 0.84, 0.36)
            cov = clamp(0.5 - d_bg / aa)
            if cov <= 0.0:
                continue
            grad_t = clamp((ny + 0.84) / 1.68)
            r = mix(15.0, 28.0, grad_t)
            g = mix(23.0, 40.0, grad_t)
            b = mix(42.0, 62.0, grad_t)

            glow = math.exp(-((nx + 0.15) ** 2 + (ny + 0.35) ** 2) * 3.2)
            r = mix(r, 56.0, glow * 0.22)
            g = mix(g, 189.0, glow * 0.26)
            b = mix(b, 248.0, glow * 0.32)

            rim = clamp(1.0 - abs(d_bg + 0.012) / (aa * 1.3))
            r = mix(r, 148.0, rim * 0.28)
            g = mix(g, 185.0, rim * 0.34)
            b = mix(b, 225.0, rim * 0.42)

            ring_d = abs(math.hypot(nx, ny) - 0.54) - 0.032
            ring_cov = clamp(0.5 - ring_d / aa)
            if ring_cov > 0.0:
                r = mix(r, 56.0, ring_cov * 0.32)
                g = mix(g, 189.0, ring_cov * 0.32)
                b = mix(b, 248.0, ring_cov * 0.32)

            stem = sdf_rrect(nx, ny, -0.18, 0.02, 0.095, 0.38, 0.045)
            bowl_outer = sdf_rrect(nx, ny, 0.06, -0.11, 0.27, 0.25, 0.22)
            bowl_inner = sdf_rrect(nx, ny, 0.07, -0.11, 0.12, 0.11, 0.095)
            bowl = max(bowl_outer, -bowl_inner)
            p_d = min(stem, bowl)
            p_cov = clamp(0.5 - p_d / aa)
            if p_cov > 0.0:
                pt = clamp((nx + ny + 0.8) / 1.6)
                pr = mix(240.0, 56.0, pt)
                pg = mix(249.0, 189.0, pt)
                pb = mix(255.0, 248.0, pt)
                r = mix(r, pr, p_cov)
                g = mix(g, pg, p_cov)
                b = mix(b, pb, p_cov)

            dot_d = math.hypot(nx - 0.26, ny - 0.27) - 0.085
            dot_cov = clamp(0.5 - dot_d / aa)
            if dot_cov > 0.0:
                r = mix(r, 74.0, dot_cov)
                g = mix(g, 222.0, dot_cov)
                b = mix(b, 128.0, dot_cov)

            idx = (py * size + px) * 4
            buf[idx] = int(clamp(r, 0.0, 255.0))
            buf[idx + 1] = int(clamp(g, 0.0, 255.0))
            buf[idx + 2] = int(clamp(b, 0.0, 255.0))
            buf[idx + 3] = int(clamp(cov * 255.0, 0.0, 255.0))
    return buf

specs = [
    ("icon_16x16.png", 16),
    ("icon_16x16@2x.png", 32),
    ("icon_32x32.png", 32),
    ("icon_32x32@2x.png", 64),
    ("icon_128x128.png", 128),
    ("icon_128x128@2x.png", 256),
    ("icon_256x256.png", 256),
    ("icon_256x256@2x.png", 512),
    ("icon_512x512.png", 512),
    ("icon_512x512@2x.png", 1024),
]

cache = {}
for name, sz in specs:
    if sz not in cache:
        cache[sz] = render_icon(sz)
    write_png(os.path.join(out_dir, name), sz, sz, cache[sz])
PY

STAGE="$(mktemp -d)/Peedle.app"
mkdir -p "$STAGE/Contents/MacOS" "$STAGE/Contents/Resources"
cp "$BIN" "$STAGE/Contents/MacOS/peedle"
chmod +x "$STAGE/Contents/MacOS/peedle"
iconutil -c icns "$TMP_ICONSET" -o "$STAGE/Contents/Resources/AppIcon.icns"

cat > "$STAGE/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleDisplayName</key>
    <string>Peedle</string>
    <key>CFBundleExecutable</key>
    <string>peedle</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>CFBundleIdentifier</key>
    <string>com.coah80.peedle</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>Peedle</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.utilities</string>
    <key>LSMinimumSystemVersion</key>
    <string>13.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSSupportsAutomaticGraphicsSwitching</key>
    <true/>
    <key>CFBundleURLTypes</key>
    <array>
        <dict>
            <key>CFBundleURLName</key>
            <string>Web site URL</string>
            <key>CFBundleURLSchemes</key>
            <array>
                <string>http</string>
                <string>https</string>
                <string>peedle</string>
            </array>
            <key>CFBundleTypeRole</key>
            <string>Viewer</string>
        </dict>
    </array>
    <key>CFBundleDocumentTypes</key>
    <array>
        <dict>
            <key>CFBundleTypeExtensions</key>
            <array>
                <string>html</string>
                <string>htm</string>
            </array>
            <key>CFBundleTypeName</key>
            <string>HTML Document</string>
            <key>CFBundleTypeRole</key>
            <string>Viewer</string>
            <key>LSItemContentTypes</key>
            <array>
                <string>public.html</string>
            </array>
        </dict>
    </array>
</dict>
</plist>
PLIST

rm -rf "$DEST"
ditto "$STAGE" "$DEST"
codesign --force --deep --sign - "$DEST"
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$DEST"
rm -rf "$(dirname "$TMP_ICONSET")" "$(dirname "$STAGE")"
echo "Installed $DEST"
