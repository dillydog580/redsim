"""Record the example builds in the real game (oracle) as tests/known/*.json."""
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python"))
import redsim as rs
from redsim.examples import ALL
from redsim.difftest import Scenario, check
from redsim.fuzz import save_regression
from redsim.oracle import Oracle

OUT = Path(__file__).resolve().parents[1] / "tests" / "known"

def to_scenario(name, fn):
    w = rs.World()
    spec = fn(w, (0, 0, 0))
    blocks = w.blocks()
    xs = [p[0] for p, _ in blocks]; ys = [p[1] for p, _ in blocks]; zs = [p[2] for p, _ in blocks]
    pad = 2
    mn = (min(xs) - pad, min(ys) - pad, min(zs) - pad)
    size = (max(xs) - mn[0] + pad + 1, max(ys) - mn[1] + pad + 1, max(zs) - mn[2] + pad + 1)
    rel = lambda p: [p[0] - mn[0], p[1] - mn[1], p[2] - mn[2]]
    bl = [tuple(rel(p)) + (b.state,) for p, b in sorted(blocks, key=lambda pb: (pb[0][1], pb[0][2], pb[0][0]))]
    bes = []
    for p, b in blocks:
        items = w.container(p)
        if items:
            bes.append({"pos": rel(p), "items": [list(i) for i in items]})
    acts = []
    for t, lst in spec["inputs"].items():
        for inp in lst:
            a = {"at": t, "type": {"use": "use", "setblock": "setblock", "place": "place"}[inp[0]], "pos": rel(inp[1])}
            if len(inp) > 2:
                st = inp[2]
                a["state"] = st if ":" in st.split("[")[0] else "minecraft:" + st
            acts.append(a)
    return Scenario(blocks=bl, size=size, ticks=spec["ticks"], actions=acts, block_entities=bes, offset=(3, 10, 5), name=name)

with Oracle() as o:
    for name, fn in ALL.items():
        scen = to_scenario(name, fn)
        d, res, sim = check(o, scen)
        print(f"{name:24s} {'MATCHES GAME' if d is None else 'DIVERGES'}  ({scen.ticks} ticks)")
        if d: print(d.describe())
        for f in OUT.glob(f"{name}_*.json"): f.unlink()
        save_regression(scen, "known build", o, directory=OUT, name=name)
