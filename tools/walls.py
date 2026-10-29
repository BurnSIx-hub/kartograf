"""Полоса стены в духе паков FA: вид сверху на гребень, бесшовно по длине.

Из рисованной текстуры материала (вид сверху) вырезается горизонтальная полоса —
по швам рядов, если они есть, иначе по середине. Полоса ужимается до высоты
«толщина × 256» и замыкается по длине: копия со сдвигом на половину вклеивается
по самому тёмному пути поперёк (по швам раствора), как в `stones.py`. По краям —
тёмная обводка; рваный край (`--ragged`) — неровная кромка с выщербинами для
разрушенных стен и скалы, кромка тоже замкнута по длине.

python walls.py вход выход толщина [--rows N] [--ragged 0..1] [--length клеток]
  толщина — в клетках (FA: 0,15–0,5); --rows — сколько рядов кладки взять в полосу
  (по тёмным швам); без него — середина картинки во всю высоту, нужную по пропорции.
"""
import argparse
import numpy as np
from PIL import Image, ImageFilter

CELL = 256
ap = argparse.ArgumentParser()
ap.add_argument("src")
ap.add_argument("dst")
ap.add_argument("thick", type=float)
ap.add_argument("--rows", type=int, default=0)
ap.add_argument("--ragged", type=float, default=0.0)
ap.add_argument("--length", type=float, default=4.0, help="длина повтора в клетках")
ap.add_argument("--band", type=float, default=0.0, help="без --rows: высота полосы, доля картинки")
ap.add_argument("--y", type=int, nargs=2, help="полоса вручную: строки от и до")
ap.add_argument("--edge", choices=["broken", "rough"], default="broken", help="рваный край: сколы кладки или бугры (изгородь, скала)")
a = ap.parse_args()

img = np.asarray(Image.open(a.src).convert("RGB"), dtype=np.float32)
H, W, _ = img.shape
lum = img.mean(2)

# Полоса: по швам рядов или по середине.
if a.y:
    y0, y1 = a.y
elif a.rows:
    prof = lum.mean(1)
    prof = prof - np.convolve(prof, np.ones(41) / 41, "same")
    seams = [i for i in range(4, H - 4) if prof[i] == prof[i - 4:i + 5].min() and prof[i] < -4]
    # Соседние минимумы ближе трети ряда — один шов.
    merged = []
    for s in seams:
        if merged and s - merged[-1] < 20:
            continue
        merged.append(s)
    mid = min(range(len(merged) - a.rows), key=lambda k: abs((merged[k] + merged[k + a.rows]) / 2 - H / 2))
    y0, y1 = merged[mid], merged[mid + a.rows]
else:
    h = round(H * (a.band or 0.25))
    y0 = (H - h) // 2
    y1 = y0 + h
band = img[y0:y1]

# Высота полосы — толщина × 256; длина — по пропорции, потом подгоняется под повтор.
out_h = max(8, round(a.thick * CELL))
scale = out_h / band.shape[0]
band = np.asarray(Image.fromarray(band.clip(0, 255).astype(np.uint8)).resize((round(band.shape[1] * scale), out_h), Image.LANCZOS), dtype=np.float32)
# Повтор не длиннее `--length` клеток (при 256 px на клетку); короче — если картинки мало.
band = band[:, :round(a.length * CELL)]
L = band.shape[1]


def seam_path(cost):
    """Путь сверху вниз по `cost` (строки — высота), шаг вбок до 2."""
    rows, width = cost.shape
    acc = cost[0].copy()
    back = np.zeros((rows, width), dtype=np.int64)
    for y in range(1, rows):
        best = np.full(width, np.inf)
        arg = np.zeros(width, dtype=np.int64)
        for d in (-2, -1, 0, 1, 2):
            sh = np.full(width, np.inf)
            if d >= 0:
                sh[d:] = acc[:width - d] if d else acc
            else:
                sh[:d] = acc[-d:]
            better = sh < best
            best[better] = sh[better]
            arg[better] = (np.arange(width) - d)[better]
        acc = best + cost[y]
        back[y] = arg
    xs = [int(np.argmin(acc))]
    for y in range(rows - 1, 0, -1):
        xs.append(back[y][xs[-1]])
    return np.array(xs[::-1])


# Замыкание по длине: края берутся из копии со сдвигом на половину, середина —
# из исходной полосы, граница — по тёмному пути (по шву между камнями).
shifted = np.roll(band, L // 2, axis=1)
cost = band.mean(2) + shifted.mean(2)
q = L // 8
left = seam_path(cost[:, q:L // 2 - 8]) + q
right = seam_path(cost[:, L // 2 + 8:L - q]) + L // 2 + 8
xs = np.arange(L)[None, :]
m = (np.clip(xs - left[:, None] + 1, 0, 1) * np.clip(right[:, None] - xs + 1, 0, 1))[..., None]
strip = m * band + (1 - m) * shifted

# Кромка: верх и низ, у рваной — замкнутый по длине шум (сумма синусов с целым
# числом периодов), съедающий до `ragged` толщины с каждой стороны.
rng = np.random.default_rng(len(a.dst))
t = np.arange(L) / L * 2 * np.pi


def rough_noise():
    """Бугристая кромка: сумма синусов с целым числом периодов и мелкая дрожь."""
    n = np.zeros(L)
    for k, amp in ((3, 1.0), (7, 0.7), (15, 0.5), (31, 0.35), (63, 0.2)):
        n += amp * np.sin(k * t + rng.uniform(0, 2 * np.pi))
    return (n - n.min()) / (np.ptp(n) + 1e-6)


def edge_noise():
    """Кромка: мелкая дрожь по всей длине и несколько выбоин с плоским дном —
    выпавшие камни. Всё по кругу, чтобы конец сошёлся с началом."""
    x = np.arange(L)
    k = 64
    knots = rng.uniform(0, 1, k) * 0.12
    xs = x * k / L
    i = xs.astype(int)
    f = xs - i
    n = knots[i % k] * (1 - f) + knots[(i + 1) % k] * f
    for _ in range(rng.integers(2, 5)):
        c, w = rng.uniform(0, L), rng.uniform(L / 40, L / 14)
        depth = rng.uniform(0.5, 1.1)
        d = np.minimum(np.abs(x - c), L - np.abs(x - c))
        s = np.clip((w - d) / (w * 0.3), 0, 1)
        wall = s * s * (3 - 2 * s)
        # Неровное дно: ступенька посередине выбоины.
        step = np.where(((x - c) % L) < L / 2, 1.0, 0.8 + 0.2 * rng.uniform())
        n = np.maximum(n, depth * wall * step)
    return n


top = np.zeros(L)
bot = np.zeros(L)
if a.ragged:
    make = rough_noise if a.edge == "rough" else edge_noise
    top = make() * a.ragged * out_h * 0.5
    bot = make() * a.ragged * out_h * 0.5
yy = np.arange(out_h)[:, None]
inside_top = yy - top[None, :]
inside_bot = (out_h - 1 - yy) - bot[None, :]
dist = np.minimum(inside_top, inside_bot)  # до ближайшей кромки, в точках
alpha = np.clip(dist + 0.5, 0, 1)
# Обводка: тёмная полоса у кромки, внутри — лёгкое затемнение к краю.
line = max(2.0, out_h * 0.05)
dark = np.clip(1 - (dist - line) / 1.5, 0, 1)
shade = np.clip(1 - dist / (out_h * 0.25), 0, 1) * 0.25
rgb = strip * (1 - shade[..., None])
rgb = rgb * (1 - dark[..., None]) + np.array([24, 20, 18]) * dark[..., None]

out = np.dstack([rgb, alpha * 255]).clip(0, 255).astype(np.uint8)
Image.fromarray(out, "RGBA").save(a.dst)
print(f"{L}×{out_h} (повтор {L / CELL:.1f} клетки при 256 px), ряды {y0}–{y1}")
