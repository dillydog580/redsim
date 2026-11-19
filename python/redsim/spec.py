"""Declarative build tests: inputs over time + expectations, with explanations on failure.

Spec (JSON):
{
  "build": "door.litematic",            # or "blocks": [[x, y, z, "state"], ...]
  "origin": [0, 0, 0],                  # where the build's min corner goes (default 0,0,0)
  "ticks": 60,
  "inputs": [{"at": 0, "action": "use", "pos": [0, 1, 0]},
             {"at": 30, "action": "setblock", "pos": [0, 1, 1], "state": "redstone_block"}],
  "expect": [
    {"at": 10, "pos": [3, 2, 0], "is": "air"},                      # state after tick 10
    {"by": 12, "pos": [3, 2, 0], "is": "air"},                      # at some tick <= 12
    {"from": 5, "to": 25, "pos": [3, 1, 0], "is": "sticky_piston[extended=true]"},
    {"never": true, "pos": [4, 1, 0], "is": "air"},
    {"end": true, "pos": [3, 2, 0], "is": "stone"},
    {"settles": true}                                               # nothing pending at the end
  ]
}

Positions are relative to the build. Tick t means the state after t game ticks; inputs
"at" t happen right after that (between tick t and t+1), like a player action.
"is" matches the block name and any listed properties (unlisted properties are ignored).
"""

from __future__ import annotations

from pathlib import Path

from . import World, Block
from .schematic import Build, load as load_schematic


def _build_from_spec(spec, base: Path) -> Build:
    if "build" in spec:
        return load_schematic(base / spec["build"])
    blocks = {tuple(b[:3]): b[3] for b in spec["blocks"]}
    xs, ys, zs = zip(*blocks) if blocks else ((0,), (0,), (0,))
    b = Build(size=(max(xs) + 1, max(ys) + 1, max(zs) + 1))
    for p, s in blocks.items():
        b.blocks[p] = s if ":" in s.split("[")[0] else "minecraft:" + s
    return b


def run_spec(spec: dict, base: Path = Path(".")) -> dict:
    from .cli import apply_input, INPUT_NOTES
    INPUT_NOTES.clear()
    build = _build_from_spec(spec, base)
    origin = tuple(spec.get("origin", (0, 0, 0)))
    ticks = int(spec.get("ticks", 40))
    w = World(trace=True)
    w.paste(build, origin)
    inputs = spec.get("inputs", [])
    for i in inputs:
        i.setdefault("pos", [0, 0, 0])
    expects = spec.get("expect", [])
    watch = sorted({tuple(e["pos"]) for e in expects if "pos" in e})
    absp = lambda p: tuple(a + b for a, b in zip(p, origin))
    hist = {p: [w[absp(p)]] for p in watch}  # hist[p][t] = state after tick t
    for t in range(ticks):
        for i in inputs:
            if i["at"] == t:
                apply_input(w, {"at": t, "action": i["action"], "pos": tuple(i["pos"]), "state": i.get("state")}, origin)
        w.tick()
        for p in watch:
            hist[p].append(w[absp(p)])

    results = []
    for e in expects:
        res = {"expect": e, "ok": True, "detail": ""}
        if e.get("settles"):
            res["ok"] = w.idle
            if not w.idle:
                res["detail"] = f"still active at the end: {w.pending()}"
            results.append(res)
            continue
        p = tuple(e["pos"])
        pat = e["is"]
        h = hist[p]
        m = [st.matches(pat) for st in h]
        if "at" in e:
            t = e["at"]
            res["ok"] = m[t]
            if not res["ok"]:
                res["detail"] = f"at tick {t} it is {h[t].state}"
                res["tick"] = t
        elif "by" in e:
            hit = next((t for t in range(min(e["by"], ticks) + 1) if m[t]), None)
            res["ok"] = hit is not None
            res["detail"] = f"first true at tick {hit}" if hit is not None else f"not reached by tick {e['by']} (it is {h[min(e['by'], ticks)].state})"
            res["tick"] = min(e["by"], ticks)
        elif "from" in e:
            bad = next((t for t in range(e["from"], min(e["to"], ticks) + 1) if not m[t]), None)
            res["ok"] = bad is None
            if bad is not None:
                res["detail"] = f"at tick {bad} it is {h[bad].state}"
                res["tick"] = bad
        elif e.get("never"):
            bad = next((t for t in range(len(m)) if m[t]), None)
            res["ok"] = bad is None
            if bad is not None:
                res["detail"] = f"happened at tick {bad}"
                res["tick"] = bad
        elif e.get("end"):
            res["ok"] = m[-1]
            if not res["ok"]:
                res["detail"] = f"at the end it is {h[-1].state}"
                res["tick"] = ticks
        if not res["ok"]:
            # pistons near a block that did not do what was expected
            diags = []
            for dx in range(-2, 3):
                for dy in range(-2, 3):
                    for dz in range(-2, 3):
                        q = absp((p[0] + dx, p[1] + dy, p[2] + dz))
                        if w[q].name in ("piston", "sticky_piston"):
                            rq = tuple(a - b for a, b in zip(q, origin))
                            diags.append(f"piston at {rq}: " + w.why_not_piston(q).replace("\n", "; "))
            if diags:
                res["pistons_nearby"] = diags
            changes = []
            prev = None
            for t, st in enumerate(h):
                if st != prev:
                    changes.append(f"t={t}: {st.state.replace('minecraft:', '')}")
                prev = st
            res["history"] = changes
            res["why"] = w.explain(absp(p))
        results.append(res)

    passed = all(r["ok"] for r in results)
    lines = [("PASS" if passed else "FAIL") + f" ({sum(r['ok'] for r in results)}/{len(results)} expectations)"]
    for r in results:
        e = r["expect"]
        desc = ", ".join(f"{k}={v}" for k, v in e.items())
        lines.append(f"  [{'ok' if r['ok'] else 'FAIL'}] {desc}" + (f" -> {r['detail']}" if r["detail"] else ""))
        if not r["ok"] and "history" in r:
            lines.append("      history: " + "; ".join(r["history"][:12]))
            lines.append("      last change caused by:\n" + "\n".join("        " + l for l in r["why"].splitlines()))
            for dline in r.get("pistons_nearby", [])[:6]:
                lines.append("      " + dline)
    for n in INPUT_NOTES:
        lines.append(f"input note: {n}")
    if w.warnings:
        lines.append("warnings (not simulated):")
        lines += [f"  {x}" for x in w.warnings]
    return {"passed": passed, "results": results, "warnings": w.warnings, "input_notes": list(INPUT_NOTES),
            "text": "\n".join(lines)}
