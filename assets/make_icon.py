"""Draws the NumberLock icon (padlock on a green squircle) and writes PNG + ICO files. No dependencies."""
import math, struct, sys, zlib, os

def png_bytes(w, h, rgba):
    raw = b''.join(b'\x00' + bytes(rgba[y * w * 4:(y + 1) * w * 4]) for y in range(h))
    chunk = lambda t, d: struct.pack('>I', len(d)) + t + d + struct.pack('>I', zlib.crc32(t + d) & 0xffffffff)
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 6, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(raw, 9)) + chunk(b'IEND', b''))

def rrect(x, y, cx, cy, hw, hh, r):
    dx, dy = abs(x - cx) - hw + r, abs(y - cy) - hh + r
    return math.hypot(max(dx, 0), max(dy, 0)) + min(max(dx, dy), 0) - r

def shade(u, v):
    """Colour at a point in unit space (0..1), or None for transparent."""
    if rrect(u, v, .5, .5, .44, .44, .2) > 0:
        return None
    t = v
    bg = (int(0x3a + (0x1c - 0x3a) * t), int(0x74 + (0x3d - 0x74) * t), int(0x63 + (0x34 - 0x63) * t))
    cream = (0xf4, 0xf1, 0xea)
    # shackle: thick arc on top, legs down into the body
    ring = abs(math.hypot(u - .5, v - .40) - .135)
    if (v <= .40 and ring < .042) or (.40 < v < .52 and abs(abs(u - .5) - .135) < .042):
        return cream
    if rrect(u, v, .5, .64, .225, .17, .05) <= 0:
        if math.hypot(u - .5, v - .60) < .045 or (abs(u - .5) < .018 and .60 < v < .72):
            return (0x1f, 0x3f, 0x36)
        return cream
    return bg

def render(size, ss=3):
    out = bytearray(size * size * 4)
    for y in range(size):
        for x in range(size):
            r = g = b = a = 0
            for sy in range(ss):
                for sx in range(ss):
                    c = shade((x + (sx + .5) / ss) / size, (y + (sy + .5) / ss) / size)
                    if c:
                        r += c[0]; g += c[1]; b += c[2]; a += 1
            i = (y * size + x) * 4
            if a:
                out[i:i + 4] = bytes((r // a, g // a, b // a, a * 255 // (ss * ss)))
    return out

def main(outdir):
    pngs = {}
    for s in (16, 32, 48, 64, 128, 256, 512, 1024):
        pngs[s] = png_bytes(s, s, render(s, 4 if s <= 64 else 2))
        open(os.path.join(outdir, f'icon-{s}.png'), 'wb').write(pngs[s])
    # ICO with embedded PNGs (Windows Vista+)
    sizes = (16, 32, 48, 256)
    head = struct.pack('<HHH', 0, 1, len(sizes))
    offset, dirs, blobs = 6 + 16 * len(sizes), b'', b''
    for s in sizes:
        d = pngs[s]
        dirs += struct.pack('<BBBBHHII', s % 256, s % 256, 0, 0, 1, 32, len(d), offset)
        blobs += d
        offset += len(d)
    open(os.path.join(outdir, 'icon.ico'), 'wb').write(head + dirs + blobs)

main(sys.argv[1] if len(sys.argv) > 1 else os.path.dirname(os.path.abspath(__file__)))
