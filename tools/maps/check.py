"""Самопроверка карты в открытом окне: смысл предметов, трубы, наложения, двери, стены, крыши, проходы.

python -I check.py — печатает нарушения и итог (0 — чисто). Сами проверки с 0.2.48 живут в
Картографе (`src/check.rs`, перенесены туда из этого скрипта 2026-11-18 один в один, сверено на
24 картах): их видно в окне — «Проверка» в меню, — а скрипт берёт тот же список через MCP
`check_map`. Ложное срабатывание чинить в `src/check.rs`, с тестом.
"""
import json, os, sys
sys.stdout.reconfigure(encoding="utf-8")
exec(open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "mcp.py"), encoding="utf-8").read())

r, t = call("check_map")
if r.get("isError"):
    print(f"check_map не ответил (Картограф старше 0.2.48?): {t[:300]}")
    sys.exit(2)
d = json.loads(t)
for i in d["issues"]:
    print(i["line"])
print(f"ИТОГО нарушений: {d['count']} (объектов {d['objects']}, крыш {d['roofs']}, дверей {d['doors']})")
p.stdin.close(); p.wait()
