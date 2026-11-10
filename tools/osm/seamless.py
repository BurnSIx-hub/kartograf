"""Сгенерированная текстура земли → бесшовная плитка для пола Картографа.

    python -I seamless.py <картинка> <выход.png> [--crop x0,y0,x1,y1] [--size N]

Генератор обещает бесшовность, но не держит её: шов меряется (разница крайних
столбцов и строк против соседних внутри), и если он заметен, середина плитки
смешивается со сдвинутой на полплитки копией — края берутся из сдвинутой, где
они непрерывны, середина — из исходной. `--crop` — кусок в долях (лист, где
модель нарисовала несколько текстур). `--size` — сторона плитки, точек: 256 на
клетку, как у FA (2048 — плитка в 8 клеток).
"""
import sys

import numpy as np
from PIL import Image


def seam(a):
    """Шов: насколько край отличается от противоположного против соседних столбцов внутри."""
    inner = np.abs(np.diff(a, axis=1)).mean() + np.abs(np.diff(a, axis=0)).mean()
    edge = np.abs(a[:, 0] - a[:, -1]).mean() + np.abs(a[0] - a[-1]).mean()
    return edge / max(inner, 1e-6)


def tile(a):
    h, w = a.shape[:2]
    rolled = np.roll(a, (h // 2, w // 2), axis=(0, 1))
    y = np.sin(np.linspace(0, np.pi, h))[:, None]
    x = np.sin(np.linspace(0, np.pi, w))[None, :]
    wgt = np.clip(np.minimum(y, x) * 1.6, 0, 1)[..., None]
    return a * wgt + rolled * (1 - wgt)


if __name__ == "__main__":
    args = sys.argv[1:]
    im = Image.open(args[0]).convert("RGB")
    if "--crop" in args:
        f = [float(v) for v in args[args.index("--crop") + 1].split(",")]
        im = im.crop((int(f[0] * im.width), int(f[1] * im.height), int(f[2] * im.width), int(f[3] * im.height)))
    side = min(im.size)
    im = im.crop((0, 0, side, side))
    a = np.asarray(im, float)
    before = seam(a)
    if before > 1.3:
        a = tile(a)
    out = Image.fromarray(np.clip(a, 0, 255).astype(np.uint8))
    if "--size" in args:
        n = int(args[args.index("--size") + 1])
        out = out.resize((n, n), Image.LANCZOS)
    out.save(args[1])
    print(args[1].split("/")[-1], out.size, f"шов {before:.2f} -> {seam(np.asarray(out, float)):.2f}")
