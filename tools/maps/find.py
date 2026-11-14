import json, os, sys
sys.stdout.reconfigure(encoding="utf-8")
exec(open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "mcp.py"), encoding="utf-8").read())
kind = sys.argv[1]
for q in sys.argv[2:]:
    a = {"query": q, "limit": 16}
    if kind != "any": a["kind"] = kind
    _, t = call("find_assets", **a)
    try: d = json.loads(t)
    except Exception: print(q, "ERR", t[:200]); continue
    def sz(x):
        s = x.get("size_cells")
        return f"{s[0]:g}x{s[1]:g}" if s else "?"
    print(f"{q} ({d.get('total', len(d['assets']))}):", ", ".join(f"{x['name']}[{sz(x)}]" for x in d["assets"]))
p.stdin.close(); p.wait()
