"""Линии берега, пляжа, причала, обрыва и голого камня поверх рамки участка.
Так нашлось, что пляж лежит по обе стороны линии берега.

    python -I plot.py <участок.osm> <out.png>
"""
import sys, xml.etree.ElementTree as ET
from PIL import Image, ImageDraw
f, out = sys.argv[1], sys.argv[2]
root = ET.parse(f).getroot()
b = root.find("bounds").attrib
minlat, minlon, maxlat, maxlon = (float(b[k]) for k in ("minlat", "minlon", "maxlat", "maxlon"))
nodes = {n.get("id"): (float(n.get("lat")), float(n.get("lon"))) for n in root.findall("node")}
S = 800
def xy(i):
    lat, lon = nodes[i]
    return ((lon - minlon) / (maxlon - minlon) * S, (maxlat - lat) / (maxlat - minlat) * S)
im = Image.new("RGB", (S, S), (255, 255, 255)); d = ImageDraw.Draw(im)
col = {"coastline": (0, 0, 255), "beach": (200, 150, 0), "pier": (255, 0, 0), "cliff": (0, 0, 0), "bare_rock": (120, 120, 120)}
for w in root.findall("way"):
    tags = {t.get("k"): t.get("v") for t in w.findall("tag")}
    k = tags.get("natural") or tags.get("man_made")
    if k in col:
        pts = [xy(n.get("ref")) for n in w.findall("nd") if n.get("ref") in nodes]
        d.line(pts, fill=col[k], width=3 if k != "bare_rock" else 1)
im.save(out)
