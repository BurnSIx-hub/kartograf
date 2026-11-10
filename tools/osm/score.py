"""Насколько карта похожа на снимок: цвет по сетке и доля «древесного» тёмно-зелёного.

    python -I score.py <cmp.jpg> [out.jpg] [сетка=24]

Вход — пара из sat.py (слева снимок, справа карта, высота 900). Обе половины
приводятся к одной рамке, режутся на сетку и сравниваются в CIELAB:
- ΔE — средняя разница цвета по ячейкам (меньше — ближе); отдельно — разница
  светлоты L и сдвиг оттенка (a, b): тон FA стилизован, важнее, где расходится
  сильнее среднего;
- лес — доля ячеек, где тёмная зелень (кроны) на снимке и на карте, и их согласие.
Картинка `out` — снимок, карта и карта расхождений (ярче — дальше от снимка).
"""
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFont

src = sys.argv[1]
out = sys.argv[2] if len(sys.argv) > 2 else None
n = int(sys.argv[3]) if len(sys.argv) > 3 else 24

im = Image.open(src).convert("RGB")
w, h = im.size
# Пара: снимок, полоса 12 px, карта. Ширины равны с точностью до пикселя.
half = (w - 12) // 2
sat = im.crop((0, 0, half, h))
mp = im.crop((w - half, 0, w, h))
# Подписи «снимок» и «карта» в левом верхнем углу — мимо них.
top = 46


def lab(img):
    a = np.asarray(img.crop((0, top, img.width, img.height)).resize((n, n), Image.BOX), dtype=float) / 255.0
    a = np.where(a > 0.04045, ((a + 0.055) / 1.055) ** 2.4, a / 12.92)
    xyz = a @ np.array([[0.4124, 0.2126, 0.0193], [0.3576, 0.7152, 0.1192], [0.1805, 0.0722, 0.9505]])
    xyz /= np.array([0.9505, 1.0, 1.089])
    f = np.where(xyz > 0.008856, np.cbrt(xyz), 7.787 * xyz + 16 / 116)
    return np.dstack([116 * f[..., 1] - 16, 500 * (f[..., 0] - f[..., 1]), 200 * (f[..., 1] - f[..., 2])])


A, B = lab(sat), lab(mp)
d = np.sqrt(((A - B) ** 2).sum(-1))
dl = B[..., 0] - A[..., 0]
dab = np.sqrt(((A[..., 1:] - B[..., 1:]) ** 2).sum(-1))


def trees(L):
    # Кроны: тёмные и зеленоватые (a < 0), светлота ниже средней по участку.
    return (L[..., 0] < np.percentile(L[..., 0], 40)) & (L[..., 1] < -2)


ta, tb = trees(A), trees(B)
agree = (ta == tb).mean()
# Только ASCII: консоль Windows в cp1251 не печатает Δ и «−».
print(f"dE {d.mean():5.1f}  dL(map-sat) {dl.mean():+5.1f}  dab {dab.mean():4.1f}  "
      f"trees sat {ta.mean():.0%} map {tb.mean():.0%} agree {agree:.0%}")

if out:
    s = 300
    heat = np.clip(d / max(d.max(), 1e-6), 0, 1)
    hm = Image.fromarray((np.dstack([heat, heat * 0.35, 1 - heat]) * 255).astype(np.uint8)).resize((s, s), Image.NEAREST)
    sheet = Image.new("RGB", (s * 3 + 24, s + 28), (255, 255, 255))
    for k, (pic, label) in enumerate([(sat, "снимок"), (mp, "карта"), (hm, f"ΔE {d.mean():.1f}")]):
        pic = pic.crop((0, top, pic.width, pic.height)).resize((s, s)) if pic is not hm else pic
        sheet.paste(pic, (k * (s + 12), 28))
        ImageDraw.Draw(sheet).text((k * (s + 12) + 4, 4), label, fill=(0, 0, 0), font=ImageFont.truetype("arial.ttf", 18))
    sheet.save(out, quality=90)
