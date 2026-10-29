"""Объект из картинки Gemini: вырезка фона-хромакея, тень, размер в клетках.

Gemini прозрачный фон не рисует, поэтому просим ровный ярко-зелёный (для зелёных
предметов — пурпурный). Фон узнаётся по рамке картинки; прозрачность — по тому,
насколько точка «фоновее» предмета, с мягким переходом. С краёв снимается отсвет
фона. Предмет обрезается по себе и вписывается в свой настоящий размер в клетках
(256 px на клетку, как в паках Dungeondraft). Холст — целое число клеток с запасом
под тень: по нему Картограф узнаёт размер объекта. Тень мягкая, вправо-вниз;
коврам и прочему плоскому её не нужно.

python cutout.py вход.jpg выход.png ширина высота [тень 1/0] [px на клетку]
(ширина и высота — настоящий размер предмета в клетках, можно дробные;
встроенные объекты режутся по 512 px на клетку — исходники Gemini это позволяют)
"""
import sys
import numpy as np
from PIL import Image, ImageFilter

src, dst = sys.argv[1], sys.argv[2]
cw, ch = float(sys.argv[3]), float(sys.argv[4])
shadow = sys.argv[5] != "0" if len(sys.argv) > 5 else True
CELL = int(sys.argv[6]) if len(sys.argv) > 6 else 256
SHADOW = (0.03, 0.04, 0.45)  # сдвиг по x, y (доля стороны клетки) и плотность

img = np.asarray(Image.open(src).convert("RGB"), dtype=np.float32)
border = np.concatenate([img[:8].reshape(-1, 3), img[-8:].reshape(-1, 3), img[:, :8].reshape(-1, 3), img[:, -8:].reshape(-1, 3)])
bg = np.median(border, 0)
# Канал, которым фон выделяется: у зелёного — G над R и B, у пурпурного — R и B над G.
if bg[1] > max(bg[0], bg[2]):
    key = img[..., 1] - np.maximum(img[..., 0], img[..., 2])
    kb = bg[1] - max(bg[0], bg[2])
else:
    key = np.minimum(img[..., 0], img[..., 2]) - img[..., 1]
    kb = min(bg[0], bg[2]) - bg[1]
# На пурпуре — только зелёные предметы, в них розового нет: порог строже, и бледные
# розовые пятна, которые Gemini оставляет в просветах листвы, тоже уходят в фон.
green = bg[1] > max(bg[0], bg[2])
lo, hi = (0.25 * kb, 0.75 * kb) if green else (0.1 * kb, 0.5 * kb)
alpha = 1 - np.clip((key - lo) / (hi - lo), 0, 1)
# Отсвет: на краях предмет подкрашен фоном — срезаем «лишний» канал.
rgb = img.copy()
if bg[1] > max(bg[0], bg[2]):
    rgb[..., 1] = np.minimum(rgb[..., 1], np.maximum(rgb[..., 0], rgb[..., 2]))
else:
    # Пурпур поднимает R и B над G: на сколько меньший из них выше G — столько
    # снимается с обоих (раньше срезалось только до среднего, и край оставался розовым).
    spill = np.clip(np.minimum(rgb[..., 0], rgb[..., 2]) - rgb[..., 1], 0, None)
    rgb[..., 0] -= spill
    rgb[..., 2] -= spill
    # Розоватые пятна внутри предмета (просветы, которые Gemini залил бледным) —
    # цветом окрестности: медиана по окну, где самих пятен меньшинство.
    blot = (spill > 12) & (alpha > 0.5)
    if blot.any():
        # С кромкой: по краю пятна — смесь с белым, порог её не ловит.
        grown = Image.fromarray((blot * 255).astype(np.uint8)).filter(ImageFilter.MaxFilter(7))
        blot = (np.asarray(grown) > 0) & (alpha > 0.5)
        smooth = np.asarray(Image.fromarray(rgb.clip(0, 255).astype(np.uint8)).filter(ImageFilter.MedianFilter(21)), dtype=np.float32)
        rgb[blot] = smooth[blot]
# Край поджимается на точку внутрь — в полупрозрачной кромке цвет фона, его не спасти.
# Мелкий сор (одиночные точки) убирается: прозрачность сглаживается и подрезается.
a8 = Image.fromarray((alpha * 255).astype(np.uint8)).filter(ImageFilter.MinFilter(3)).filter(ImageFilter.MedianFilter(5))
alpha = np.asarray(a8, dtype=np.float32) / 255
ys, xs = np.nonzero(alpha > 0.1)
y0, y1, x0, x1 = ys.min(), ys.max() + 1, xs.min(), xs.max() + 1
obj = np.dstack([rgb, alpha * 255])[y0:y1, x0:x1].clip(0, 255).astype(np.uint8)
obj = Image.fromarray(obj, "RGBA")

scale = min(cw * CELL / obj.width, ch * CELL / obj.height)
obj = obj.resize((max(1, round(obj.width * scale)), max(1, round(obj.height * scale))), Image.LANCZOS)
W, H = (int(np.ceil(s / CELL + 0.08)) * CELL for s in obj.size)
ox, oy = (W - obj.width) // 2, (H - obj.height) // 2
# Тень — размытая прозрачность, сдвинутая вправо-вниз.
sh = Image.new("L", (W, H), 0)
sh.paste(obj.getchannel("A"), (ox + round(SHADOW[0] * CELL), oy + round(SHADOW[1] * CELL)))
sh = sh.filter(ImageFilter.GaussianBlur(CELL * 0.025))
out = Image.new("RGBA", (W, H), (0, 0, 0, 0))
out.putalpha(sh.point(lambda v: round(v * SHADOW[2] * shadow)))
layer = Image.new("RGBA", (W, H), (0, 0, 0, 0))
layer.paste(obj, (ox, oy))
out = Image.alpha_composite(out, layer)
out.save(dst)
print(f"{W}×{H}, предмет {obj.width}×{obj.height}, фон {bg.round()}")
