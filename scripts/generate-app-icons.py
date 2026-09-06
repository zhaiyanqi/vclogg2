#!/usr/bin/env python3
"""Derive native/package icons from the four approved RGBA PNG masters (requires Pillow)."""

from pathlib import Path
import shutil

from PIL import Image


def main():
    root = Path(__file__).resolve().parent.parent
    resources = root / "crates/vclogg-app/resources"
    icons = resources / "icons"
    sizes = [(size, size) for size in (16, 20, 24, 32, 40, 48, 64, 128, 256)]
    for name in ("soft", "compact", "illustration", "sticker"):
        with Image.open(icons / f"{name}.png") as image:
            if image.mode != "RGBA" or image.size != (1024, 1024):
                raise ValueError(f"{name}: expected a 1024px square RGBA master")
            if image.getchannel("A").getextrema() != (0, 255):
                raise ValueError(f"{name}: expected both transparent and opaque pixels")
            target = resources / "windows/vclogg2.ico" if name == "soft" else icons / f"{name}.ico"
            image.save(target, format="ICO", sizes=sizes)
    shutil.copyfile(icons / "soft.png", resources / "windows/vclogg2.png")
    shutil.copyfile(icons / "soft.png", root / "docs/assets/vclogg2.png")


if __name__ == "__main__":
    main()
