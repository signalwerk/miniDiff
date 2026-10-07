#!/usr/bin/env python3
"""Render the MiniDiff app icon (stdlib only) into assets/.

    python3 scripts/make_icon.py

Writes icon-1024.png, icon-256.png and favicon-64.png. The macOS .icns is
built from icon-1024.png by scripts/bundle-macos.sh.
"""
import math
import os
import struct
import zlib

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "assets")


def rounded_rect_coverage(x, y, rx0, ry0, rx1, ry1, r):
    """Anti-aliased coverage of pixel centre (x, y) for a rounded rectangle."""
    cx = min(max(x, rx0 + r), rx1 - r)
    cy = min(max(y, ry0 + r), ry1 - r)
    d = math.hypot(x - cx, y - cy) - r
    return min(max(0.5 - d, 0.0), 1.0)


def blend(dst, src, a):
    return tuple(d * (1 - a) + s * a for d, s in zip(dst, src))


def render(size):
    s = size / 1024.0
    px = bytearray()
    # Shapes in 1024-space.
    bg = (100, 100, 924, 924, 190)
    doc_a = (250, 230, 600, 720, 56)
    doc_b = (424, 330, 774, 820, 56)
    ring = (404, 310, 794, 840, 72)  # dark outline around B
    lines = [(0, 1.0), (1, 0.7), (2, 0.85), (3, 0.5)]
    for j in range(size):
        y = (j + 0.5) / s
        row = bytearray()
        for i in range(size):
            x = (i + 0.5) / s
            a_bg = rounded_rect_coverage(x, y, *bg)
            if a_bg <= 0:
                row += b"\0\0\0\0"
                continue
            t = (y - 100) / 824
            col = (31 - 12 * t, 38 - 14 * t, 52 - 18 * t)
            col = blend(col, (248, 81, 73), rounded_rect_coverage(x, y, *doc_a))
            col = blend(col, (24, 28, 38), rounded_rect_coverage(x, y, *ring))
            cov_b = rounded_rect_coverage(x, y, *doc_b)
            col = blend(col, (63, 185, 80), cov_b)
            if cov_b > 0:
                for k, frac in lines:
                    ly = 330 + 490 * (0.26 + k * 0.16)
                    lx0 = 424 + 350 * 0.17
                    lx1 = lx0 + 350 * 0.66 * frac
                    c = rounded_rect_coverage(x, y, lx0, ly - 14, lx1, ly + 14, 14)
                    col = blend(col, (255, 255, 255), c * 0.9 * cov_b)
            row += bytes(int(max(0, min(255, round(v)))) for v in col) + bytes([int(a_bg * 255)])
        px += b"\0" + row
    return px


def png(size, data):
    def chunk(tag, body):
        return struct.pack(">I", len(body)) + tag + body + struct.pack(">I", zlib.crc32(tag + body) & 0xFFFFFFFF)

    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(bytes(data), 9))
        + chunk(b"IEND", b"")
    )


if __name__ == "__main__":
    os.makedirs(ROOT, exist_ok=True)
    for size, name in [(1024, "icon-1024.png"), (256, "icon-256.png"), (64, "favicon-64.png")]:
        with open(os.path.join(ROOT, name), "wb") as f:
            f.write(png(size, render(size)))
        print("wrote", name)
