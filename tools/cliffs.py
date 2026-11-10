"""Полосы рельефа для встроенного набора: стенка обрыва, гребень уступа, осыпь под обрывом.

Как у полос FA (`Cliff_Stone_*`, `Grass_Ridge_*`, `Under_Cliff_*`): верх полосы —
верхняя кромка (ложится на край высокого яруса), низ — подножие. Бесшовны по длине.
Цвета нейтральные: полоса ложится и на траву, и на снег, и на песок.

    python tools/cliffs.py assets/builtin/walls
"""
import os
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFilter

rng = np.random.default_rng(7)


def noise(w, h, cells_x, cells_y, octaves=4, gen=None):
    """Шум 0..1, бесшовный по ширине: сетка узлов зациклена по x."""
    gen = gen or rng
    out = np.zeros((h, w))
    amp, total = 1.0, 0.0
    cx, cy = cells_x, cells_y
    for _ in range(octaves):
        grid = gen.random((cy + 2, cx))
        xs = np.arange(w) / w * cx
        ys = np.arange(h) / h * cy
        x0 = np.floor(xs).astype(int)
        tx = xs - x0
        y0 = np.floor(ys).astype(int)
        ty = ys - y0
        tx = tx * tx * (3 - 2 * tx)
        ty = ty * ty * (3 - 2 * ty)
        a = grid[y0][:, x0 % cx]
        b = grid[y0][:, (x0 + 1) % cx]
        c = grid[y0 + 1][:, x0 % cx]
        d = grid[y0 + 1][:, (x0 + 1) % cx]
        top = a * (1 - tx) + b * tx
        low = c * (1 - tx) + d * tx
        out += (top * (1 - ty[:, None]) + low * ty[:, None]) * amp
        total += amp
        amp *= 0.5
        cx *= 2
        cy *= 2
    return out / total


def edge(w, cells, lo, hi):
    """Рваный край по длине полосы: строка, где он проходит, для каждого x."""
    n = noise(w, 1, cells, 1, 3)[0]
    return lo + (hi - lo) * n


def save(img, path):
    img.save(path, "WEBP", quality=90, method=6)
    print(path, img.size)


def cliff(out):
    W, H = 1536, 256
    y = np.arange(H)[:, None].astype(float) * np.ones((1, W))
    top = edge(W, 9, 4, 22)[None, :]
    bottom = edge(W, 7, 206, 248)[None, :]
    rim = top + 24 + 12 * noise(W, 1, 12, 1, 2)[0][None, :]
    grain = noise(W, H, 64, 10, 3)
    # Пласты породы: волнистые уступы. Фаза — высота, сбитая шумом сильнее по длине.
    warp = noise(W, H, 10, 3, 4) * 70 + noise(W, H, 40, 6, 2) * 12
    phase = (y + warp) / 30.0
    f = phase - np.floor(phase)
    # Внутри пласта: светлый верх, тёмный низ — уступ нависает над следующим.
    ledge = 1.0 - f ** 1.5
    # Пласты рвутся: где шум мал, уступа нет — ровный камень.
    broken = noise(W, H, 24, 8, 2)
    ledge = np.where(broken > 0.42, ledge, 0.55)
    t = np.clip((y - top) / (bottom - top), 0, 1)
    base = np.array([160, 148, 130])
    dark = np.array([52, 45, 38])
    k = (0.1 + 0.9 * t ** 0.8)[..., None]
    col = base * (1 - k) + dark * k
    col = col * ((0.72 + 0.45 * ledge) * (0.86 + 0.28 * grain))[..., None]
    # Чернильный низ пласта — тонкий штрих там, где уступ обрывается.
    ink = (f > 0.85) & (broken > 0.42) & (y > rim)
    col[ink] = col[ink] * 0.32
    # Кромка поверху: светлый голый камень с обводкой.
    lip = np.clip(1 - (y - top) / np.maximum(rim - top, 1), 0, 1)
    col = col * (1 - 0.6 * lip[..., None]) + np.array([202, 192, 172]) * 0.6 * lip[..., None]
    line_top = np.abs(y - rim) < 1.4
    col[line_top] = col[line_top] * 0.45
    foot = (bottom - y < 6) & (bottom - y >= 0)
    col[foot] = col[foot] * 0.3
    alpha = np.clip(np.minimum(y - top, bottom - y) / 3.0, 0, 1)
    img = np.dstack([np.clip(col, 0, 255), alpha * 255]).astype(np.uint8)
    save(Image.fromarray(img, "RGBA"), os.path.join(out, "cliff.webp"))


# Кромка гребня по сезонам: травяная бахрома (лето, весна, волшебный лес), сухая
# трава (осень), снежный козырёк (зима). Цвета — от тёмного низа к светлым кончикам.
FRINGES = {
    "ridge": [(34, 52, 16), (56, 84, 22), (86, 118, 30), (150, 168, 40), (200, 204, 66)],
    "ridge_dry": [(70, 56, 24), (104, 86, 38), (140, 120, 56), (184, 160, 80), (216, 196, 118)],
    "ridge_snow": None,
}


def blades(draw, W, S, base_y, count, palette, spread, length, width, down, g):
    """Травинки клиньями от кромки: вниз — нависают над откосом, вверх — лохматят
    верхний край. Бесшовно: у краёв полосы дублируются со сдвигом на W."""
    for col in palette:
        for _ in range(count // len(palette)):
            x = g.random() * W
            y0 = base_y[int(x) % W] + spread[0] + g.random() * (spread[1] - spread[0])
            turn = (g.random() - 0.5) * (1.2 if down else 1.6)
            ang = (np.pi / 2 if down else -np.pi / 2) + turn
            ln = length[0] + g.random() * (length[1] - length[0])
            wd = width[0] + g.random() * (width[1] - width[0])
            tip = (x + np.cos(ang) * ln, y0 + np.sin(ang) * ln)
            nx, ny = -np.sin(ang) * wd, np.cos(ang) * wd
            for dx in (-W, 0, W):
                if -40 < x + dx < W + 40:
                    pts = [(x + dx + nx, y0 + ny), (x + dx - nx, y0 - ny), (tip[0] + dx, tip[1])]
                    draw.polygon([(px * S, py * S) for px, py in pts], fill=col + (255,))


def ridge(out, name, palette):
    """Гребень уступа по образцу FA Grass_Ridge: поверху бахрома травы (или снега),
    под ней полоса голой земли — мелкие трещины, пятна, тень от нависающего края, —
    к низу земля темнеет и переходит в мягкую тень на нижнем ярусе. Вся полоса
    плавно волнится. Кромка — около ряда 44: полоса кладётся чуть выше края яруса."""
    W, H, S = 2048, 256, 2
    g = np.random.default_rng({"ridge": 11, "ridge_dry": 12, "ridge_snow": 13}[name])
    y = np.arange(H)[:, None].astype(float) * np.ones((1, W))
    sway = edge(W, 3, -20, 20)
    lip = 46 + sway + edge(W, 25, -5, 5)                      # кромка
    clump = noise(W, 1, 60, 1, 2)[0]                          # пучки травы
    bottom = (lip + edge(W, 11, 92, 136))[None, :]            # низ откоса
    top = lip[None, :] + 4
    # Земля: бугры, светлые сверху и тёмные снизу (свет сверху), мелкие трещины
    # двух размеров и лёгкие пласты поперёк.
    warp = noise(W, H, 18, 4, 3) * 26
    grain = noise(W, H, 110, 14, 4)
    blot = noise(W, H, 44, 7, 3)
    phase = (y - sway[None, :] + warp) / 13.0
    plate = 1.0 - (phase - np.floor(phase)) ** 1.6
    crack = (np.abs(noise(W, H, 46, 10, 4) - 0.5) < 0.016) | (np.abs(noise(W, H, 96, 18, 3) - 0.5) < 0.011)
    if palette:
        dark, mid, light = np.array([28, 20, 10]), np.array([88, 68, 40]), np.array([146, 120, 80])
    else:
        dark, mid, light = np.array([30, 28, 30]), np.array([72, 66, 62]), np.array([122, 116, 112])
    v = 0.45 * grain + 0.35 * blot + 0.2 * plate
    col = np.where((v > 0.55)[..., None], mid + (light - mid) * ((v - 0.55) / 0.3).clip(0, 1)[..., None], dark + (mid - dark) * (v / 0.55).clip(0, 1)[..., None])
    lit = np.roll(blot, 2, axis=0) - np.roll(blot, -2, axis=0)
    col = col * (1.0 + np.clip(lit * 9.0, -0.45, 0.45))[..., None]
    col[crack] = col[crack] * 0.4
    # Под краем — тень от бахромы; к низу откос темнеет и уходит в тень.
    u = y - top
    rest = bottom - y
    shade = (0.45 + 0.55 * np.clip(u / 30.0, 0, 1) ** 0.8) * (0.6 + 0.4 * np.clip(rest / 34.0, 0, 1))
    col = col * shade[..., None]
    shadow = np.array([14, 12, 8])
    a = np.clip((u + 3) / 3.0, 0, 1)
    # Ниже подножия — тень: от 0.6 до нуля за 44 ряда, край чуть рваный.
    fall = np.clip(-rest / (40 + 10 * noise(W, 1, 17, 1, 2)[0][None, :]), 0, 1)
    low = rest < 0
    col[low] = shadow
    a = np.where(low, 0.6 * (1 - fall) ** 1.7, a)
    # Сам переход земли в тень — мягкий.
    blend = np.clip(1 - rest / 10.0, 0, 1) * (rest >= 0)
    col = col * (1 - blend[..., None]) + shadow * blend[..., None]
    a = a * (1 - blend * 0.4)
    base = Image.fromarray(np.dstack([np.clip(col, 0, 255), np.clip(a, 0, 1) * 255]).astype(np.uint8), "RGBA")
    base = base.filter(ImageFilter.GaussianBlur(0.5)).resize((W * S, H * S), Image.BICUBIC)

    over = Image.new("RGBA", (W * S, H * S), (0, 0, 0, 0))
    draw = ImageDraw.Draw(over)
    if palette:
        # Тёмная подложка бахромы с рваным низом, потом травинки: тёмные вниз,
        # пёстрые вверх, светлые кончики поверх.
        # Пучки: где трава гуще, подложка толще и травинки свисают ниже.
        under = lip + 3 + 14 * clump
        for x in range(W):
            draw.rectangle([x * S, (lip[x] - 3) * S, x * S + S, under[x] * S], fill=palette[0] + (255,))
        hang = lip + 10 * clump
        blades(draw, W, S, hang, 3600, palette[:3], (-4, 8), (10, 26), (0.9, 1.8), True, g)
        blades(draw, W, S, lip, 3000, palette[:4], (-2, 6), (6, 16), (0.8, 1.5), False, g)
        blades(draw, W, S, hang, 2200, palette[2:], (-8, 8), (7, 18), (0.7, 1.4), True, g)
        blades(draw, W, S, lip, 1200, palette[3:], (-4, 2), (5, 12), (0.6, 1.2), False, g)
    else:
        # Снежный козырёк: наплыв с округлыми буграми, голубоватая тень снизу.
        drift = lip + 10 + 14 * noise(W, 1, 37, 1, 3)[0]
        for x in range(W):
            draw.rectangle([x * S, (lip[x] - 4) * S, x * S + S, drift[x] * S], fill=(168, 184, 202, 255))
            draw.rectangle([x * S, (lip[x] - 4) * S, x * S + S, (drift[x] - 3) * S], fill=(234, 241, 247, 255))
        for _ in range(900):
            x = g.random() * W
            r = 2 + g.random() * 5
            yy = drift[int(x) % W] - r * 0.7
            for dx in (-W, 0, W):
                draw.ellipse([(x + dx - r) * S, (yy - r) * S, (x + dx + r) * S, (yy + r) * S], fill=(226, 235, 243, 255))
    over = over.filter(ImageFilter.GaussianBlur(0.6))
    img = Image.alpha_composite(base, over).resize((W, H), Image.LANCZOS)
    save(img, os.path.join(out, name + ".webp"))


def under(out):
    W, H = 1536, 128
    y = np.arange(H)[:, None].astype(float)
    grain = noise(W, H, 96, 8, 3)
    noise(W, H, 40, 4, 2)  # не убирать: тот же ход общего шума — гребни остаются прежними
    reach = edge(W, 13, 60, 118)[None, :]
    t = np.clip(y / reach, 0, 1)
    a = (1 - t) ** 1.2 * 0.75 * (0.75 + 0.5 * grain)
    rgb = np.zeros((H, W, 3)) + np.array([40, 34, 28])
    shade = Image.fromarray(np.dstack([rgb, np.clip(a, 0, 1) * 255]).astype(np.uint8), "RGBA")
    # Камни осыпи — отдельные обломки, как у FA: тёмное тело, светлый верх,
    # обводка и тень вниз. Крупные и густо у стенки, мельче и реже дальше.
    S = 3
    g = np.random.default_rng(21)
    rocks = []
    for _ in range(240):
        depth = g.random() ** 1.7
        r = (16 - 10 * depth) * g.uniform(0.6, 1.25)
        k = int(g.integers(5, 9))
        ang = np.sort(g.uniform(0, 2 * np.pi, k))
        rad = r * g.uniform(0.72, 1.12, k)
        rocks.append((4 + depth * 78, g.uniform(0, W), r, ang, rad, g.uniform(0.55, 0.85), g.uniform(0.82, 1.12)))
    rocks.sort(key=lambda q: q[0])
    layer = Image.new("RGBA", (W * S, H * S), (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)

    def shape(x, yy, ang, rad, sq, kx=1.0, ky=1.0, dy=0.0, dx=0.0):
        return [((x + dx + np.cos(t_) * rr * kx) * S, (yy + dy + np.sin(t_) * rr * sq * ky) * S) for t_, rr in zip(ang, rad)]

    for pass_ in range(2):
        for yy, x, r, ang, rad, sq, tone in rocks:
            for ox in (-W, 0, W):
                if x + ox + 2 * r < 0 or x + ox - 2 * r > W:
                    continue
                if pass_ == 0:
                    d.polygon(shape(x + ox, yy, ang, rad, sq, 1.05, 1.1, 0.3 * r, 0.15 * r), fill=(14, 11, 8, 150))
                    continue
                body = tuple(int(c * tone) for c in (74, 69, 61))
                top = tuple(int(c * tone) for c in (120, 113, 100))
                d.polygon(shape(x + ox, yy, ang, rad, sq), fill=body + (255,), outline=(30, 26, 22, 255), width=S)
                d.polygon(shape(x + ox, yy, ang, rad, sq, 0.72, 0.62, -0.2 * r * sq), fill=top + (255,))
    stones_img = layer.resize((W, H), Image.LANCZOS)
    shade.alpha_composite(stones_img)
    save(shade, os.path.join(out, "under_cliff.webp"))


if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else "assets/builtin/walls"
    cliff(out)
    under(out)
    for name, palette in FRINGES.items():
        ridge(out, name, palette)
