#!/usr/bin/env python3
"""Regenerate assets/icons/*.png from the Noto Color Emoji font.

Dev-only tool: the generated PNGs are committed, end users never run this.
Requires: python3, Pillow, fonts-noto-color-emoji.

Outputs (64x64 RGBA, transparent, centred):
  flag-XX.png  every ISO 3166-1 region flag the font provides
  flag-unknown.png  fallback ("?" flag)
  ghost-on.png / ghost-off.png (grey)   busy-1.png / busy-2.png (hourglass)
  error.png (warning sign)
"""
import itertools
import os
import sys

from PIL import Image, ImageDraw, ImageFont, ImageOps

FONT = os.environ.get("EMOJI_FONT", "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf")
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "assets", "icons")
SIZE = 64

font = ImageFont.truetype(FONT, 109)  # the only bitmap strike of Noto Color Emoji


def render(text):
    img = Image.new("RGBA", (300, 160), (0, 0, 0, 0))
    ImageDraw.Draw(img).text((0, 0), text, font=font, embedded_color=True)
    bbox = img.getbbox()
    return img.crop(bbox) if bbox else None


def square(img, grey=False):
    w, h = img.size
    side = max(w, h)
    sq = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    sq.paste(img, ((side - w) // 2, (side - h) // 2))
    if grey:
        alpha = sq.getchannel("A")
        g = ImageOps.grayscale(sq).point(lambda v: 90 + v * 0.45)
        sq = Image.merge("RGBA", (g, g, g, alpha.point(lambda a: a * 0.75)))
    return sq.resize((SIZE, SIZE), Image.LANCZOS)


def save(name, img):
    img.save(os.path.join(OUT, name + ".png"), "PNG", optimize=True)


def flag(cc):
    return "".join(chr(0x1F1E6 + ord(c) - ord("A")) for c in cc)


def main():
    os.makedirs(OUT, exist_ok=True)
    for f in os.listdir(OUT):
        if f.endswith(".png"):
            os.remove(os.path.join(OUT, f))
    # Noto draws any pair it doesn't know as a white "?" flag: use QQ (never
    # assigned) as reference, skip identical renders, keep it as the fallback.
    unknown = render(flag("QQ"))
    save("flag-unknown", square(unknown))
    count = 0
    for a, b in itertools.product("ABCDEFGHIJKLMNOPQRSTUVWXYZ", repeat=2):
        img = render(flag(a + b))
        if img and img.tobytes() != unknown.tobytes():
            save(f"flag-{a}{b}", square(img))
            count += 1
    ghost = render("\U0001F47B")
    save("ghost-on", square(ghost))
    save("ghost-off", square(ghost, grey=True))
    save("busy-1", square(render("⏳")))
    save("busy-2", square(render("⌛")))
    save("error", square(render("⚠️")))
    print(f"{count} flags + 6 other icons written to {os.path.normpath(OUT)}", file=sys.stderr)


if __name__ == "__main__":
    main()
