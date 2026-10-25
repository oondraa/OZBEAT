"""Renders the MSIX tile and taskbar images from the app icon.

Usage: python make_assets.py <out dir>
"""

import sys
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
ICON = ROOT / "app" / "assets" / "ozbeat.ico"

# name: (width, height) at scale 100, and how much of the height the icon fills.
TILES = {
    "Square44x44Logo": ((44, 44), 1.0),
    "Square150x150Logo": ((150, 150), 0.66),
    "Wide310x150Logo": ((310, 150), 0.66),
    "Square310x310Logo": ((310, 310), 0.66),
    "SplashScreen": ((620, 300), 0.5),
    "StoreLogo": ((50, 50), 1.0),
}
SCALES = (100, 125, 150, 200)
# Unplated taskbar / Start list icons at exact pixel sizes.
TARGET_SIZES = (16, 24, 32, 48, 256)


def icon() -> Image.Image:
    image = Image.open(ICON)
    image.size = max(image.ico.sizes())
    image.load()
    return image.convert("RGBA")


def render(source: Image.Image, size: tuple[int, int], fill: float) -> Image.Image:
    canvas = Image.new("RGBA", size, (0, 0, 0, 0))
    side = round(size[1] * fill)
    mark = source.resize((side, side), Image.LANCZOS)
    canvas.alpha_composite(mark, ((size[0] - side) // 2, (size[1] - side) // 2))
    return canvas


def main() -> None:
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    source = icon()
    for name, ((w, h), fill) in TILES.items():
        for scale in SCALES:
            size = (round(w * scale / 100), round(h * scale / 100))
            render(source, size, fill).save(out / f"{name}.scale-{scale}.png")
    for px in TARGET_SIZES:
        tile = render(source, (px, px), 1.0)
        tile.save(out / f"Square44x44Logo.targetsize-{px}.png")
        tile.save(out / f"Square44x44Logo.targetsize-{px}_altform-unplated.png")


if __name__ == "__main__":
    main()
