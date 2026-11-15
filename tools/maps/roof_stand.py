"""Стенд крыш: рядом прямоугольная вальмовая и многоугольная (Г-образная, уступ стены 0,5,
срез угла 45°) с одним покрытием, к обеим — пристройки, и отдельная вальма для сравнения.

python -I roof_stand.py <папка снимков> [px]   (KDATA — папка данных с паками FA)

Что смотреть: резкость и шаг рядов у крыши по контуру против прямоугольной, кромку свеса,
коньки и ендовы, врезку пристроек в скаты по диагональным ендовам. Снимки: all (вся карта),
a, b, pair, join, ref — участки по 200 точек на клетку.
"""
import os, sys
sys.stdout.reconfigure(encoding="utf-8")
exec(open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "mcp.py"), encoding="utf-8").read())
out = os.path.abspath(sys.argv[1])
px = int(sys.argv[2]) if len(sys.argv) > 2 else 140
os.makedirs(out, exist_ok=True)

call("new_map", width=34, height=20)
call("paint", material=key("Grass_A_01", "floor"), area=[0, 0, 34, 20], target="terrain")
cover = key("Roof_Texture_Wood_Dark_Shingle_A1", "roof")
floor = key("Wood_Dark_Flooring_A", "floor")
call("set_wall_style", asset=key("Wall_Wood_Dark_A", "wall"), width=0.15)

# А — прямоугольная вальмовая 10×7 и двускатная пристройка с юга (врезается в южный скат)
call("paint", material=floor, area=[3, 4, 13, 11], target="room")
call("paint", material=floor, area=[6, 11, 10, 15], target="room")
call("add_roof", area=[3, 4, 13, 11], paint=cover, shape="hip")
call("add_roof", area=[6, 11, 10, 15], paint=cover, shape="gable", turn=1)

# Б — Г-образный корпус по контуру стен и вальмовая пристройка к восточной стене крыла
walls = [[17, 3], [22, 3], [22, 3.5], [27, 3.5], [29, 5.5], [29, 9], [23, 9], [23, 17], [17, 17]]
call("paint", material=floor, shape="poly", points=walls, target="room")
call("add_roof", points=walls, paint=cover, shape="tent")
call("paint", material=floor, area=[23, 11.5, 28, 15.5], target="room")
call("add_roof", area=[23, 11.5, 28, 15.5], paint=cover, shape="hip")

# Г — отдельная прямоугольная вальмовая: образец вида, ни с чем не врезается
call("paint", material=floor, area=[11, 13, 15.5, 18], target="room")
call("add_roof", area=[11, 13, 15.5, 18], paint=cover, shape="hip")

call("save_map", path=os.path.join(out, "roof_stand.karta"))
for name, a in (("all", {"px": px}), ("a", {"px": 200, "area": [2, 3, 14, 16]}), ("b", {"px": 200, "area": [16, 2, 30, 18]}),
                ("pair", {"px": 200, "area": [10, 3, 22, 9]}), ("join", {"px": 200, "area": [21, 10, 29, 17]}), ("ref", {"px": 200, "area": [10, 12, 23, 19]})):
    _, t = call("render_picture", path=os.path.join(out, name + ".png"), **a)
    print(name, t[:100].replace("\n", " "))
print("ошибки:", errors or "нет")
p.stdin.close(); p.wait()
