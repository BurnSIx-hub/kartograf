"""Снимок карты: python -I shot.py out.png [px] [roofs] [lighting] [x0,y0,x1,y1] [grid]"""
import os, sys
sys.stdout.reconfigure(encoding="utf-8")
exec(open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "mcp.py"), encoding="utf-8").read())
out = sys.argv[1]
a = {"path": out, "px": int(sys.argv[2]) if len(sys.argv) > 2 else 40}
if len(sys.argv) > 3 and sys.argv[3] != "-": a["roofs"] = sys.argv[3]
if len(sys.argv) > 4 and sys.argv[4] != "-": a["lighting"] = sys.argv[4]
if len(sys.argv) > 5 and sys.argv[5] != "-": a["area"] = [float(v) for v in sys.argv[5].split(",")]
if len(sys.argv) > 6: a["grid"] = True
_, t = call("render_picture", **a)
print(t[:200], errors or "")
p.stdin.close(); p.wait()
