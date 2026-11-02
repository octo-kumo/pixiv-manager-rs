#!/usr/bin/env python3
"""Generate the shipped application icon assets (Windows .ico, macOS .icns,
Linux .png) from one vector description.

The outputs are **version-controlled build assets** committed under
`assets/icons/`: packaging must never synthesise a placeholder at install time
(a generated icon is both worse-looking and an antivirus-reputation smell).

Run after changing the artwork:

    python3 scripts/make-icons.py

Requires Pillow (`python3 -m pip install --user pillow`).
"""

from __future__ import annotations

import struct
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

# ── Palette: the app's accent blue, as used by the Linux .desktop entry ──
TOP = (0x2A, 0xAE, 0xFF)
BOTTOM = (0x00, 0x6D, 0xD6)
GLYPH = (0xFF, 0xFF, 0xFF)

MASTER = 1024
CORNER_RADIUS_RATIO = 0.22

ROOT = Path(__file__).resolve().parent.parent
ICON_DIR = ROOT / "assets" / "icons"

# Fonts that ship with almost every Linux distro / macOS / Windows.
FONT_CANDIDATES = [
    "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/liberation-sans-fonts/LiberationSans-Bold.ttf",
    "/System/Library/Fonts/Helvetica.ttc",
    "C:/Windows/Fonts/segoeuib.ttf",
    "C:/Windows/Fonts/arialbd.ttf",
]


def load_font(size: int) -> ImageFont.FreeTypeFont:
    for candidate in FONT_CANDIDATES:
        try:
            return ImageFont.truetype(candidate, size)
        except OSError:
            continue
    raise SystemExit("no usable bold font found; install DejaVu Sans")


def render_master() -> Image.Image:
    """Draw the icon at 1024x1024 with a transparent surround."""
    canvas = Image.new("RGBA", (MASTER, MASTER), (0, 0, 0, 0))

    # Vertical gradient on a rounded plate.
    plate = Image.new("RGBA", (MASTER, MASTER), (0, 0, 0, 0))
    gradient = Image.new("RGBA", (1, MASTER))
    for y in range(MASTER):
        t = y / (MASTER - 1)
        gradient.putpixel(
            (0, y),
            (
                round(TOP[0] + (BOTTOM[0] - TOP[0]) * t),
                round(TOP[1] + (BOTTOM[1] - TOP[1]) * t),
                round(TOP[2] + (BOTTOM[2] - TOP[2]) * t),
                255,
            ),
        )
    gradient = gradient.resize((MASTER, MASTER))

    mask = Image.new("L", (MASTER, MASTER), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        (0, 0, MASTER - 1, MASTER - 1),
        radius=int(MASTER * CORNER_RADIUS_RATIO),
        fill=255,
    )
    plate.paste(gradient, (0, 0), mask)
    canvas.alpha_composite(plate)

    # The "P" mark, optically centred (cap height sits high in its box).
    draw = ImageDraw.Draw(canvas)
    font = load_font(int(MASTER * 0.62))
    box = draw.textbbox((0, 0), "P", font=font)
    x = (MASTER - (box[2] - box[0])) / 2 - box[0]
    y = (MASTER - (box[3] - box[1])) / 2 - box[1]
    draw.text((x, y), "P", font=font, fill=GLYPH)
    return canvas


def write_png(master: Image.Image, size: int, path: Path) -> None:
    master.resize((size, size), Image.LANCZOS).save(path, format="PNG", optimize=True)


def write_ico(master: Image.Image, path: Path) -> None:
    """Multi-resolution .ico (16-256 px), the sizes Explorer/shell use."""
    sizes = [16, 24, 32, 48, 64, 128, 256]
    master.resize((256, 256), Image.LANCZOS).save(
        path, format="ICO", sizes=[(s, s) for s in sizes]
    )


def write_icns(master: Image.Image, path: Path) -> None:
    """Multi-resolution .icns for the macOS .app bundle.

    ICNS is a flat container: `icns` magic, total length, then chunks of
    `type` + length (including the 8-byte header) + PNG payload. The modern
    `icp4`..`ic14` chunk types accept PNG directly.
    """
    chunks = [
        (b"icp4", 16),
        (b"icp5", 32),
        (b"icp6", 64),
        (b"ic07", 128),
        (b"ic08", 256),
        (b"ic09", 512),
        (b"ic10", 1024),
        (b"ic11", 32),
        (b"ic12", 64),
        (b"ic13", 256),
        (b"ic14", 512),
    ]
    body = b""
    for kind, size in chunks:
        import io

        buffer = io.BytesIO()
        master.resize((size, size), Image.LANCZOS).save(buffer, format="PNG")
        payload = buffer.getvalue()
        body += kind + struct.pack(">I", len(payload) + 8) + payload
    path.write_bytes(b"icns" + struct.pack(">I", len(body) + 8) + body)


def main() -> None:
    ICON_DIR.mkdir(parents=True, exist_ok=True)
    master = render_master()
    write_png(master, 512, ICON_DIR / "pixiv-manager.png")
    write_ico(master, ICON_DIR / "pixiv-manager.ico")
    write_icns(master, ICON_DIR / "pixiv-manager.icns")
    for name in ("pixiv-manager.png", "pixiv-manager.ico", "pixiv-manager.icns"):
        path = ICON_DIR / name
        print(f"{path.relative_to(ROOT)}: {path.stat().st_size} bytes")


if __name__ == "__main__":
    main()
