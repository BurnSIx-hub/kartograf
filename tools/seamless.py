"""Бесшовная плитка из обычной картинки.

Картинка смешивается со своей копией, сдвинутой на половину по обеим осям: у краёв
берётся копия (её края — соседние точки середины оригинала, поэтому стык сходится),
в середине — оригинал. Смешивание с сохранением разброса (Heitz — Neyret), чтобы
в зоне перехода текстура не мутнела.

python seamless.py вход.jpg выход.png [сторона]
"""
import sys
import numpy as np
from PIL import Image

src, dst = sys.argv[1], sys.argv[2]
side = int(sys.argv[3]) if len(sys.argv) > 3 else 1024
img = Image.open(src).convert("RGB")
s = min(img.size)
img = img.crop(((img.width - s) // 2, (img.height - s) // 2, (img.width + s) // 2, (img.height + s) // 2))
img = img.resize((side, side), Image.LANCZOS)
a = np.asarray(img, dtype=np.float32)
h, w, _ = a.shape
shifted = np.roll(a, (h // 2, w // 2), axis=(0, 1))
wx = np.sin(np.pi * (np.arange(w) + 0.5) / w)
wy = np.sin(np.pi * (np.arange(h) + 0.5) / h)
m = np.outer(wy, wx)[..., None] ** 0.5
mean = a.reshape(-1, 3).mean(0)
mix = mean + (m * (a - mean) + (1 - m) * (shifted - mean)) / np.sqrt(m * m + (1 - m) ** 2)
# Крупные пятна светлее-темнее выдают повтор плитки издалека: размытие по кругу
# (плитка замкнута, поэтому через FFT) и вычитание большей части отклонения.
fy = np.fft.fftfreq(h)[:, None]
fx = np.fft.fftfreq(w)[None, :]
sigma = w / 10
kernel = np.exp(-2 * (np.pi * sigma) ** 2 * (fx * fx + fy * fy))
for c in range(3):
    low = np.real(np.fft.ifft2(np.fft.fft2(mix[..., c]) * kernel))
    mix[..., c] -= 0.85 * (low - low.mean())
out = np.clip(mix, 0, 255).astype(np.uint8)
Image.fromarray(out).save(dst)

g = out.astype(np.float32).mean(2)
ix = np.abs(np.diff(g, axis=1)).mean()
iy = np.abs(np.diff(g, axis=0)).mean()
print(f"{dst}: шов л-п {np.abs(g[:, 0] - g[:, -1]).mean() / ix:.2f}, в-н {np.abs(g[0] - g[-1]).mean() / iy:.2f}")
