"""Renders the app icon (assets/icon.png, icon.ico, icon.icns) with Pillow.

Run from the repo root: python packaging/make_icon.py
"""
import math
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFilter

S = 2048  # supersampled canvas, downscaled at the end
OUT = Path(__file__).resolve().parent.parent / "assets"


def mask():
    return Image.new("L", (S, S), 0)


def disc(m, cx, cy, r, fill=255):
    ImageDraw.Draw(m).ellipse([cx - r, cy - r, cx + r, cy + r], fill=fill)


def biohazard(cx, cy, u):
    """Biohazard symbol as an L mask; u = size unit (symbol is ~60u wide)."""
    arms = mask()
    cut = mask()
    inner = mask()  # the crescents' hollows, shrunk to leave a gap around the ring arcs
    for k in range(3):
        a = math.radians(-90 + 120 * k)
        dx, dy = math.cos(a), math.sin(a)
        disc(arms, cx + dx * 11 * u, cy + dy * 11 * u, 15 * u)
        disc(cut, cx + dx * 16.5 * u, cy + dy * 16.5 * u, 11.2 * u)
        disc(inner, cx + dx * 16.5 * u, cy + dy * 16.5 * u, 9.9 * u)
    shape = ImageChops.subtract(arms, cut)
    # ring around the centre, visible only as arcs inside each crescent's hollow
    ring = mask()
    disc(ring, cx, cy, 13.6 * u)
    disc(ring, cx, cy, 11.2 * u, 0)
    ring = ImageChops.multiply(ring, inner)
    shape = ImageChops.lighter(shape, ring)
    # centre hole and the thin gaps between arms and centre
    hole = mask()
    disc(hole, cx, cy, 3.6 * u)
    d = ImageDraw.Draw(hole)
    for k in range(3):
        a = math.radians(-90 + 120 * k)
        dx, dy = math.cos(a), math.sin(a)
        d.line([cx + dx * 3 * u, cy + dy * 3 * u, cx + dx * 10.6 * u, cy + dy * 10.6 * u], fill=255, width=int(1.2 * u))
    return ImageChops.subtract(shape, hole)


def gradient(top, bottom):
    g = Image.new("RGB", (1, S))
    for y in range(S):
        t = y / (S - 1)
        g.putpixel((0, y), tuple(round(a + (b - a) * t) for a, b in zip(top, bottom)))
    return g.resize((S, S))


def render():
    pad = int(S * 0.06)
    tile = mask()
    ImageDraw.Draw(tile).rounded_rectangle([pad, pad, S - pad, S - pad], radius=int(S * 0.2), fill=255)

    icon = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    # soft drop shadow under the tile
    shadow = tile.filter(ImageFilter.GaussianBlur(S * 0.02))
    icon.paste((0, 0, 0, 110), (0, int(S * 0.012)), shadow)
    # Steam navy tile
    icon.paste(gradient((0x2a, 0x47, 0x5e), (0x0e, 0x14, 0x1b)), (0, 0), tile)
    # thin light rim along the top edge
    rim = mask()
    ImageDraw.Draw(rim).rounded_rectangle([pad, pad, S - pad, S - pad], radius=int(S * 0.2), outline=255, width=int(S * 0.006))
    icon.paste((0x66, 0xc0, 0xf4, 90), (0, 0), ImageChops.multiply(rim, gradient((255,) * 3, (0,) * 3).convert("L")))

    sym = biohazard(S / 2, S * 0.52, S / 92)
    glow = sym.filter(ImageFilter.GaussianBlur(S * 0.035))
    icon.paste((0x1a, 0x9f, 0xff, 150), (0, 0), ImageChops.multiply(glow, tile))
    icon.paste(gradient((0x9b, 0xd8, 0xff), (0x47, 0xa8, 0xe8)), (0, 0), sym)

    # green "installed" badge, bottom-right
    b = mask()
    bx, by, br = S * 0.735, S * 0.735, S * 0.125
    disc(b, bx, by, br + S * 0.022)
    icon.paste((0x0e, 0x14, 0x1b, 255), (0, 0), b)
    b = mask()
    disc(b, bx, by, br)
    icon.paste(gradient((0x8e, 0xc8, 0x2a), (0x58, 0x8a, 0x1b)), (0, 0), b)
    chk = mask()
    w = int(S * 0.034)
    ImageDraw.Draw(chk).line(
        [(bx - br * 0.48, by + br * 0.02), (bx - br * 0.12, by + br * 0.38), (bx + br * 0.5, by - br * 0.36)],
        fill=255, width=w, joint="curve",
    )
    for x, y in [(bx - br * 0.48, by + br * 0.02), (bx + br * 0.5, by - br * 0.36)]:
        disc(chk, x, y, w / 2)
    icon.paste((0xf4, 0xff, 0xe4, 255), (0, 0), chk)
    return icon


def main():
    OUT.mkdir(exist_ok=True)
    big = render().resize((1024, 1024), Image.LANCZOS)
    big.save(OUT / "icon.png")
    big.resize((256, 256), Image.LANCZOS).save(OUT / "icon-256.png")
    big.save(OUT / "icon.ico", sizes=[(s, s) for s in (16, 24, 32, 48, 64, 128, 256)])
    big.save(OUT / "icon.icns")
    print("wrote", *sorted(p.name for p in OUT.iterdir()))


if __name__ == "__main__":
    main()
