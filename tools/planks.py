"""Бесшовные доски из картинки досок.

Ряды досок ищутся по тёмным горизонтальным швам. В каждом ряду находится
вертикальный стык — он берётся образцом. Новый ряд — доски случайной длины,
вырезанные из дерева исходных рядов, между ними — образец стыка. Ряд замкнут
стыком на краю, ряды режутся по середине шва, поэтому плитка бесшовна.

python planks.py вход.jpg выход.png [зерно] [сжатие поперёк]

Сжатие поперёк (по умолчанию 0.5) делает доски уже, не укорачивая их: исходник
сжимается только по высоте. В плитке 1024 — 4 клетки; ряд 64 px — 4 доски на клетку.
"""
import sys
import numpy as np
from PIL import Image

src, dst = sys.argv[1], sys.argv[2]
rng = np.random.default_rng(int(sys.argv[3]) if len(sys.argv) > 3 else 1)
squeeze = float(sys.argv[4]) if len(sys.argv) > 4 else 0.5
picture = Image.open(src).convert("RGB")
picture = picture.resize((picture.width, round(picture.height * squeeze)), Image.LANCZOS)
img = np.asarray(picture, dtype=np.float32)
H, W, _ = img.shape
lum = img.mean(2)
JOINT = 5  # полуширина стыка, пикселей


def minima(profile, depth, gap):
    out = []
    for i in range(1, len(profile) - 1):
        lo, hi = max(0, i - gap), min(len(profile), i + gap)
        if profile[i] == profile[lo:hi].min() and profile[i] < np.median(profile[lo:hi]) - depth:
            if not out or i - out[-1] >= gap:
                out.append(i)
    return out


profile = lum.mean(1)
seams = minima(profile, 15, 40)
step = int(round(np.median(np.diff(seams))))
# Рез — ровно посередине шва: шов в пару строк режется между ними, и ряды
# ложатся друг на друга без сдвига. Дальше — строго через шаг.
c = seams[0]
cut = c if profile[c - 1] < profile[c + 1] else c + 1
starts = list(range(cut % step, H - step + 1, step))
bands, joints, wood = [], [], []
for a in starts:
    band = img[a:a + step]
    if band.shape[0] < step:
        continue
    edge = max(2, step // 16)
    core = lum[a + edge:a + step - edge].mean(0)
    js = [j for j in minima(core, 25, 30) if JOINT <= j < W - JOINT]
    for j in js:
        joints.append(band[:, j - JOINT:j + JOINT])
    # Дерево без стыков: куски между ними.
    cuts = [0] + [x for j in js for x in (j - JOINT, j + JOINT)] + [W]
    for x0, x1 in zip(cuts[::2], cuts[1::2]):
        if x1 - x0 > 120:
            wood.append(band[:, x0:x1])
print(f"рядов {len(starts)}, шаг {step}, стыков {len(joints)}, кусков дерева {len(wood)}")
if not joints or not wood:
    sys.exit("стыки не нашлись")

rows = []
OUT = 1024
for r in range(OUT // step):
    row, total = [], 0
    while W - total > 0:
        left = W - total
        want = int(rng.uniform(260, 700))
        joint = joints[rng.integers(len(joints))]
        want = min(want, left - joint.shape[1])
        if left - joint.shape[1] - want < 200:
            want = left - joint.shape[1]  # остаток — в эту доску
        piece = [p for p in wood if p.shape[1] >= want]
        if not piece:
            piece = [max(wood, key=lambda p: p.shape[1])]
        p = piece[rng.integers(len(piece))]
        x0 = int(rng.integers(0, p.shape[1] - min(want, p.shape[1]) + 1))
        plank = p[:, x0:x0 + want]
        if plank.shape[1] < want:
            plank = np.asarray(Image.fromarray(plank.astype(np.uint8)).resize((want, step), Image.LANCZOS), dtype=np.float32)
        if rng.random() < 0.5:
            plank = plank[:, ::-1]
        row += [joint, plank]
        total += joint.shape[1] + plank.shape[1]
    strip = np.concatenate(row, axis=1)[:, :W]
    rows.append(np.roll(strip, int(rng.integers(W)), axis=1))
tile = np.concatenate(rows, axis=0)
Image.fromarray(np.clip(tile, 0, 255).astype(np.uint8)).resize((OUT, OUT), Image.LANCZOS).save(dst)
