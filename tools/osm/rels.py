"""Мультиполигоны участка: полные или неполные (части линий нет в выгрузке —
импорт докачивает их из API OSM).

    python -I rels.py <участок.osm> [...]
"""
import sys, xml.etree.ElementTree as ET
KEYS = ("natural", "landuse", "leisure", "water", "waterway", "amenity", "building")
for f in sys.argv[1:]:
    root = ET.parse(f).getroot()
    ways = {w.get("id") for w in root.findall("way")}
    print("==", f.split("\\")[-1])
    for r in root.findall("relation"):
        tags = {t.get("k"): t.get("v") for t in r.findall("tag")}
        if tags.get("type") != "multipolygon":
            continue
        mem = [m for m in r.findall("member") if m.get("type") == "way"]
        have = sum(1 for m in mem if m.get("ref") in ways)
        t = {k: tags[k] for k in KEYS if k in tags}
        if t:
            print(f"  {'ПОЛНЫЙ ' if have == len(mem) else 'НЕПОЛН.'} {have:3}/{len(mem):<4} {t} {tags.get('name', '')}")
