"""Встроенные двери, косяки и подоконники — из полос встроенных стен.

    portals.py папка_assets_builtin

Устроено как у FA: створка — тонкая доска поперёк холста 1×1 клетка (петля справа),
косяк — пара столбов по краям проёма на холсте шире проёма на клетку
(`frame_*_1` — для проёма в клетку, холст 2×1), подоконник — плита вдоль холста
2×1 со стеклом посередине. 512 точек на клетку.
"""
import sys
import numpy as np
from PIL import Image, ImageDraw, ImageFilter

B = sys.argv[1]
C = 512
OUT = f"{B}/portals"
INK = (24, 17, 12, 255)


def strip(name, length_px, thick_px, src_cells=1.0):
    """Кусок полосы стены длиной `src_cells` клеток, вписанный в length×thick."""
    im = Image.open(f"{B}/walls/{name}.webp").convert("RGBA")
    w = min(im.width, int(src_cells * 256))
    return im.crop((0, 0, w, im.height)).resize((length_px, thick_px), Image.LANCZOS)


def outline(im, width=4):
    """Тёмная обводка по краю непрозрачного — как у FA."""
    a = np.asarray(im.getchannel("A"))
    grown = Image.fromarray(a).filter(ImageFilter.MaxFilter(width * 2 + 1))
    out = Image.new("RGBA", im.size, (0, 0, 0, 0))
    ink = Image.new("RGBA", im.size, INK)
    out.paste(ink, mask=grown)
    out.alpha_composite(im)
    return out


def canvas(wc, hc=1.0):
    return Image.new("RGBA", (int(wc * C), int(hc * C)), (0, 0, 0, 0))


def leaf(name, src, bands=False, rivets=False):
    """Створка: доска на всю длину холста, толщина 0,14 клетки, петля справа."""
    im = canvas(1)
    t = int(0.14 * C)
    y0 = (C - t) // 2
    board = strip(src, C - 24, t)
    im.alpha_composite(board, (12, y0))
    d = ImageDraw.Draw(im)
    if bands:
        for x in (int(0.18 * C), int(0.5 * C), int(0.82 * C)):
            d.rectangle((x - 9, y0 - 3, x + 9, y0 + t + 3), fill=(52, 52, 56, 255))
            for yy in (y0 + 10, y0 + t - 10):
                d.ellipse((x - 4, yy - 4, x + 4, yy + 4), fill=(120, 120, 126, 255))
    if rivets:
        for x in range(40, C - 30, 64):
            for yy in (y0 + 9, y0 + t - 9):
                d.ellipse((x - 4, yy - 4, x + 4, yy + 4), fill=(140, 140, 146, 255))
    # Петля: тёмная скоба у правого края, чуть шире доски.
    d.rectangle((C - 46, y0 - 8, C - 18, y0 + t + 8), fill=(46, 46, 50, 255))
    d.ellipse((C - 40, y0 + t // 2 - 7, C - 26, y0 + t // 2 + 7), fill=(130, 130, 136, 255))
    outline(im).save(f"{OUT}/{name}.webp", lossless=True)


def frame(name, src, cells):
    """Косяк для проёма в `cells` клеток: столбы у его краёв, холст на клетку шире."""
    im = canvas(cells + 1)
    pw, ph = int(0.2 * C), int(0.62 * C)
    # Кусок полосы поперёк: волокно дерева и швы камня идут поперёк стены.
    post = strip(src, ph, pw, src_cells=0.8).rotate(90, expand=True)
    y0 = (C - ph) // 2
    im.alpha_composite(post, (C // 2 - pw, y0))
    im.alpha_composite(post.transpose(Image.FLIP_LEFT_RIGHT), (C // 2 + cells * C, y0))
    outline(im).save(f"{OUT}/{name}.webp", lossless=True)


def sill(name, src):
    """Подоконник с окном: плита вдоль, стекло по середине, переплёты."""
    im = canvas(2)
    t = int(0.24 * C)
    y0 = (C - t) // 2
    im.alpha_composite(strip(src, 2 * C - 16, t, src_cells=2.0), (8, y0))
    d = ImageDraw.Draw(im)
    g0, g1 = y0 + int(t * 0.36), y0 + int(t * 0.64)
    d.rectangle((18, g0, 2 * C - 18, g1), fill=(150, 205, 225, 255))
    d.line((24, g0 + 4, 2 * C - 24, g0 + 4), fill=(225, 245, 252, 255), width=3)
    for x in (18, C, 2 * C - 18):
        d.rectangle((x - 7, g0 - 4, x + 7, g1 + 4), fill=(60, 44, 32, 255))
    outline(im).save(f"{OUT}/{name}.webp", lossless=True)


import os
os.makedirs(OUT, exist_ok=True)
leaf("door_wood", "wood")
leaf("door_wood_iron", "wood", bands=True)
leaf("door_metal", "metal", rivets=True)
for n in (1, 2, 3):
    frame(f"frame_wood_{n}", "wood", n)
    frame(f"frame_stone_{n}", "stone_blocks", n)
sill("window_stone", "stone_blocks")
sill("window_wood", "wood")
print("готово")
