"""redsim command line.

    redsim info door.litematic
    redsim render door.litematic --y 1
    redsim run door.litematic --input 0:use:2,1,0 --ticks 40 --watch 5,1,0 --explain 5,1,0
    redsim test spec.json            # assertions over time, with explanations on failure
    redsim verify door.litematic --input 0:use:2,1,0 --ticks 60     # compare with the real game
    redsim locational door.litematic --input 0:use:2,1,0 --ticks 60 # does it depend on position?
    redsim world door.litematic --name "My door"                    # make a playable world save

Positions in --input/--watch are relative to the build's minimum corner. --origin sets
where that corner sits in the world (default 0,0,0); dust and piston behaviour can depend on it.
Add --json to any command for machine-readable output.
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import Counter
from pathlib import Path

from . import World, Block, load_schematic, MC_VERSION, __version__
from .schematic import Build


def _xyz(s):
    return tuple(int(v) for v in s.replace(" ", ",").split(","))


def parse_input(s: str) -> dict:
    """'T:ACTION:X,Y,Z[:STATE]' e.g. '0:use:1,1,0', '10:setblock:3,1,0:redstone_block', '5:remove:3,1,0'."""
    parts = s.split(":", 3)
    if len(parts) < 3:
        raise SystemExit(f"bad --input {s!r}: expected T:ACTION:X,Y,Z[:STATE]")
    t, action, pos = int(parts[0]), parts[1], _xyz(parts[2])
    d = {"at": t, "action": action, "pos": pos}
    if len(parts) == 4:
        rest = parts[3]
        # state may itself contain ':' (namespace)
        d["state"] = rest
    return d


INPUT_NOTES: list = []


def apply_input(w: World, inp: dict, origin):
    """Apply one input; inputs that have no effect are noted in INPUT_NOTES."""
    p = tuple(a + b for a, b in zip(inp["pos"], origin))
    act = inp["action"]
    before = w[p]
    if act == "use":
        ok = w.use(p)
        if not ok:
            INPUT_NOTES.append(f"tick {inp['at']}: use at {tuple(inp['pos'])} did nothing ({before.name} has no use action)")
    elif act == "press":
        if "button" not in before.name:
            INPUT_NOTES.append(f"tick {inp['at']}: press at {tuple(inp['pos'])}: {before.name} is not a button")
        else:
            w.press(p)
    elif act == "setblock":
        w.setblock(p, inp["state"])
    elif act == "place":
        w.place(p, inp["state"])
    elif act in ("remove", "break"):
        if before.name == "air":
            INPUT_NOTES.append(f"tick {inp['at']}: remove at {tuple(inp['pos'])}: already air")
        w.remove(p)
    else:
        raise SystemExit(f"unknown input action {act!r} (use, press, setblock, place, remove)")


def _load(path, origin, trace=False) -> tuple[World, Build]:
    b = load_schematic(path)
    w = World(trace=trace)
    w.paste(b, origin)
    return w, b


def simulate(w: World, inputs: list, ticks: int, origin, until_idle=False):
    by_tick = {}
    for i in inputs:
        by_tick.setdefault(i["at"], []).append(lambda ww, i=i: apply_input(ww, i, origin))
    last_input = max([i["at"] for i in inputs], default=0)
    if until_idle:
        tl = w.record(last_input + 1, by_tick)
        extra = w.run_until_idle(ticks)
        return tl, extra
    return w.record(ticks, by_tick), 0


def cmd_info(a):
    b = load_schematic(a.build)
    counts = Counter(s.split("[")[0].replace("minecraft:", "") for s in b.blocks.values())
    w = World()
    w.paste(b, a.origin)
    kinds = Counter(w[(x + a.origin[0], y + a.origin[1], z + a.origin[2])].kind for (x, y, z) in b.blocks)
    unsupported = sorted({s.split("[")[0] for p, s in b.blocks.items()
                          if w[(p[0] + a.origin[0], p[1] + a.origin[1], p[2] + a.origin[2])].kind == "Unsupported"})
    info = {"file": a.build, "mc_version": MC_VERSION, "size": b.size, "blocks": len(b.blocks),
            "block_counts": dict(counts.most_common()), "block_entities": len(b.block_entities),
            "pending_ticks": len(b.pending_ticks), "unsupported_blocks": unsupported}
    if a.json:
        print(json.dumps(info, indent=1))
        return 0
    print(f"{a.build}: {b.size[0]}x{b.size[1]}x{b.size[2]}, {len(b.blocks)} blocks (simulating Minecraft {MC_VERSION})")
    for n, c in counts.most_common():
        print(f"  {c:5d} {n}")
    if unsupported:
        print("NOT SIMULATED (behaviour of these blocks is ignored):", ", ".join(unsupported))
    return 0


def cmd_render(a):
    w, b = _load(a.build, a.origin)
    lo = a.origin
    hi = tuple(o + s - 1 for o, s in zip(a.origin, b.size))
    print(w.render(y=(a.y + a.origin[1]) if a.y is not None else None, a=lo, b=hi))
    return 0


def cmd_run(a):
    w, b = _load(a.build, a.origin, trace=bool(a.explain))
    inputs = [parse_input(s) for s in a.input]
    tl, extra = simulate(w, inputs, a.ticks, a.origin, until_idle=a.until_idle)
    rel = lambda p: tuple(x - o for x, o in zip(p, a.origin))
    watch = [tuple(x + o for x, o in zip(_xyz(s), a.origin)) for s in a.watch]
    changes = [c for c in tl if not watch or c.pos in watch]
    out = {
        "ticks_run": a.ticks if not a.until_idle else tl.end + extra,
        "settled": w.idle,
        "changes": [{"tick": c.tick, "phase": c.phase, "pos": rel(c.pos), "from": c.old.state, "to": c.new.state} for c in changes],
        "final": {",".join(map(str, rel(p))): w[p].state for p in watch},
        "zero_tick_flickers": [{"pos": rel(p), "tick": t} for p, t in tl.flickers()],
        "input_notes": list(INPUT_NOTES),
        "warnings": w.warnings,
    }
    if a.explain:
        out["explain"] = {s: w.explain(tuple(x + o for x, o in zip(_xyz(s), a.origin))) for s in a.explain}
    if a.save:
        w.save(a.save, a.origin, tuple(o + s - 1 for o, s in zip(a.origin, b.size)))
    if a.json:
        print(json.dumps(out, indent=1))
        return 0
    for c in out["changes"][: a.limit]:
        print(f"t={c['tick']:<4} {c['phase']:<15} {c['pos']}: {c['from'].replace('minecraft:', '')} -> {c['to'].replace('minecraft:', '')}")
    if len(out["changes"]) > a.limit:
        print(f"... {len(out['changes']) - a.limit} more changes (raise --limit or use --watch)")
    if watch:
        print("final:")
        for k, v in out["final"].items():
            print(f"  {k}: {v.replace('minecraft:', '')}")
    print(f"settled: {out['settled']}")
    for f in out["zero_tick_flickers"]:
        print(f"0-tick flicker at {f['pos']} during tick {f['tick']}")
    for n in out["input_notes"]:
        print("NOTE:", n)
    for wmsg in out["warnings"]:
        print("WARNING:", wmsg)
    for k, v in out.get("explain", {}).items():
        print(f"\nwhy {k}:\n{v}")
    return 0


def cmd_test(a):
    from .spec import run_spec
    spec = json.loads(Path(a.spec).read_text())
    base = Path(a.spec).parent
    rep = run_spec(spec, base)
    if a.json:
        print(json.dumps(rep, indent=1))
    else:
        print(rep["text"])
    return 0 if rep["passed"] else 1


def cmd_verify(a):
    """Run the build in redsim and in the real game; compare every block on every tick."""
    from .difftest import Scenario, check
    from .oracle import Oracle
    b = load_schematic(a.build)
    inputs = [parse_input(s) for s in a.input]
    actions = []
    for i in inputs:
        typ = {"use": "use", "press": "press", "setblock": "setblock", "remove": "destroy"}.get(i["action"])
        if typ is None:
            raise SystemExit(f"verify does not support input action {i['action']!r}")
        act = {"at": i["at"], "type": typ, "pos": list(i["pos"])}
        if "state" in i:
            act["state"] = i["state"] if ":" in i["state"].split("[")[0] else "minecraft:" + i["state"]
        actions.append(act)
    blocks = [(x, y, z, s) for (x, y, z), s in sorted(b.blocks.items(), key=lambda kv: (kv[0][1], kv[0][2], kv[0][0]))]
    bes = []
    for p, d in b.block_entities.items():
        e = {"pos": list(p)}
        if "Items" in d:
            e["items"] = [[it["Slot"], it["id"], it.get("count", 1)] for it in d["Items"]]
        if "OutputSignal" in d:
            e["output"] = d["OutputSignal"]
        bes.append(e)
    pad = 2
    size = tuple(s + 2 * pad for s in b.size)
    blocks = [(x + pad, y + pad, z + pad, s) for (x, y, z, s) in blocks]
    for e in bes:
        e["pos"] = [v + pad for v in e["pos"]]
    for act in actions:
        act["pos"] = [v + pad for v in act["pos"]]
    scen = Scenario(blocks=blocks, size=size, ticks=a.ticks, actions=actions, block_entities=bes,
                    origin=tuple(o - pad for o in a.origin))
    with Oracle() as o:
        d, res, sim = check(o, scen)
    ok = d is None
    out = {"match": ok, "ticks": a.ticks, "game_origin": res["origin"], "mc_version": o.version,
           "warnings": sim["world"].warnings()}
    if not ok:
        out["divergence"] = d.describe()
    if a.json:
        print(json.dumps(out, indent=1))
    else:
        print(("MATCH: identical to Minecraft %s on every tick (%d ticks)" % (o.version, a.ticks)) if ok
              else "DIVERGENCE from the real game:\n" + d.describe())
    return 0 if ok else 1


def _tick_states(w, inputs, ticks, origin, size):
    """Region snapshot (relative to origin) after every tick: what a player could observe."""
    by_tick = {}
    for i in inputs:
        by_tick.setdefault(i["at"], []).append(i)
    x0, y0, z0 = origin
    x1, y1, z1 = x0 + size[0] - 1, y0 + size[1] - 1, z0 + size[2] - 1
    out = []
    for t in range(ticks):
        for i in by_tick.get(t, []):
            apply_input(w, i, origin)
        w.tick()
        out.append(w._w.region(x0 - 2, y0 - 2, z0 - 2, x1 + 2, y1 + 2, z1 + 2))
    return out


LOCATIONAL_FAR = [(1000, 0, 1000), (-1000, 0, 2000), (12345, 0, -6789), (-54321, 0, 43210), (250001, 0, -250003),
                  (-4097, 0, -4099), (29999, 0, 777)]


def cmd_locational(a):
    """Run the same build at several world positions; report whether what happens changes.

    By default the world state at the end of every tick is compared (what you can observe in
    game). --strict also compares the order of block writes inside each tick, which differs with
    position for almost any dust because of Java hash order, usually without visible effect."""
    b = load_schematic(a.build)
    inputs = [parse_input(s) for s in a.input]
    near = [(dx, dy, dz) for dx in range(0, 16, max(1, 16 // a.samples)) for dy in (0, 7) for dz in (0, 5, 11)][: a.samples * 6]
    offsets = near + LOCATIONAL_FAR
    base = None
    differing = []
    first_tick = {}
    for off in offsets:
        origin = tuple(o + d for o, d in zip(a.origin, off))
        w = World()
        w.paste(b, origin)
        if a.strict:
            tl, _ = simulate(w, inputs, a.ticks, origin)
            sig = [(c.tick, tuple(p - o for p, o in zip(c.pos, origin)), c.new.state) for c in tl]
        else:
            sig = _tick_states(w, inputs, a.ticks, origin, b.size)
        if base is None:
            base = sig
        elif sig != base:
            differing.append(origin)
            if not a.strict:
                first_tick[origin] = next(t for t, (x, y) in enumerate(zip(sig, base)) if x != y) + 1
    out = {"positions_tested": len(offsets), "compared": "block write order" if a.strict else "state after every tick",
           "location_dependent": bool(differing), "differs_at": differing[:20],
           "first_differing_tick": {",".join(map(str, k)): v for k, v in list(first_tick.items())[:20]}}
    if a.json:
        print(json.dumps(out, indent=1))
    else:
        if differing:
            ex = differing[0]
            when = f" (first at tick {first_tick[ex]})" if ex in first_tick else ""
            print(f"LOCATION DEPENDENT: {out['compared']} differs at {len(differing)} of {len(offsets)} positions, e.g. origin {ex}{when}")
            print("(dust update order or piston HashMap order follows Java hash order of positions)")
        else:
            print(f"Same {out['compared']} at all {len(offsets)} positions tested (incl. {len(LOCATIONAL_FAR)} far away).")
    return 1 if differing else 0


def cmd_world(a):
    from .worldgen import make_world
    origin = (a.origin[0], a.origin[1] - a.sink, a.origin[2])
    b = load_schematic(a.build)
    path = make_world(a.build, a.name, origin, a.saves)
    x0, y0, z0 = origin
    print(f"world written to {path}")
    print(f"Open Minecraft {MC_VERSION}, Singleplayer -> '{a.name}'. You spawn at {x0 - 3} {y0} {z0 - 3}.")
    print(f"The build occupies x {x0}..{x0 + b.size[0] - 1}, y {y0}..{y0 + b.size[1] - 1}, z {z0}..{z0 + b.size[2] - 1};"
          f" build-relative (x, y, z) is world ({x0}+x, {y0}+y, {z0}+z). The superflat surface is y -61.")
    for s in a.input:
        i = parse_input(s)
        p = tuple(c + o for c, o in zip(i["pos"], origin))
        print(f"  input at tick {i['at']}: {i['action']} at world {p[0]} {p[1]} {p[2]}")
    return 0


def cmd_bench(a):
    from .bench import main as bench_main
    return bench_main(a)


def main(argv=None):
    ap = argparse.ArgumentParser(
        prog="redsim",
        description=f"Tick-exact Minecraft Java {MC_VERSION} redstone simulator: run builds from schematics, "
                    "check them with test specs, explain why blocks changed, and compare with the real game.",
        epilog="examples:\n"
               "  redsim run door.litematic --input 0:use:2,1,0 --ticks 40 --explain 5,1,0\n"
               "  redsim test spec.json\n"
               "  redsim verify door.litematic --input 0:use:2,1,0 --ticks 60\n"
               "Positions are relative to the build's minimum corner. More: the README and docs/AGENTS.md.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    ap.add_argument("--version", action="version", version=f"redsim {__version__} (Minecraft Java {MC_VERSION})")
    sub = ap.add_subparsers(dest="cmd", required=True)

    def common(p, inputs=True):
        p.add_argument("build", help=".litematic, .schem or .nbt")
        p.add_argument("--origin", type=_xyz, default=(0, 0, 0), help="world position of the build's min corner")
        p.add_argument("--json", action="store_true", help="machine-readable output")
        if inputs:
            p.add_argument("--input", action="append", default=[],
                           help="T:ACTION:X,Y,Z[:STATE], ACTION one of use, press, setblock, place, remove (repeatable)")
            p.add_argument("--ticks", type=int, default=40, help="game ticks to run (default 40)")

    p = sub.add_parser("info", help="summarize a schematic")
    common(p, inputs=False)
    p.set_defaults(fn=cmd_info)
    p = sub.add_parser("render", help="text view of the build")
    common(p, inputs=False)
    p.add_argument("--y", type=int, default=None, help="layer (relative); default all")
    p.set_defaults(fn=cmd_render)
    p = sub.add_parser("run", help="simulate and print what changes")
    common(p)
    p.add_argument("--watch", action="append", default=[], help="X,Y,Z to report (repeatable)")
    p.add_argument("--explain", action="append", default=[], help="X,Y,Z: print the cause chain of its last change")
    p.add_argument("--until-idle", action="store_true", help="after the last input, run until nothing is pending")
    p.add_argument("--save", help="write the final state to a schematic")
    p.add_argument("--limit", type=int, default=200, help="most block changes to print (default 200)")
    p.set_defaults(fn=cmd_run)
    p = sub.add_parser("test", help="run a JSON test spec (see docs/AGENTS.md)")
    p.add_argument("spec", help="test spec .json (format: python/redsim/spec.py)")
    p.add_argument("--json", action="store_true", help="machine-readable output")
    p.set_defaults(fn=cmd_test)
    p = sub.add_parser("verify", help="compare against the real game (needs the oracle set up)")
    common(p)
    p.set_defaults(fn=cmd_verify)
    p = sub.add_parser("locational", help="check whether behaviour depends on world position")
    common(p)
    p.add_argument("--samples", type=int, default=4, help="how many groups of six nearby positions to test, besides a fixed set of far ones (default 4)")
    p.add_argument("--strict", action="store_true", help="also compare the order of block writes within each tick")
    p.set_defaults(fn=cmd_locational)
    p = sub.add_parser("world", help="write a Minecraft world save containing the build")
    p.add_argument("build", help=".litematic, .schem or .nbt")
    p.add_argument("--name", default="redsim build", help="world name (default 'redsim build')")
    p.add_argument("--origin", type=_xyz, default=(0, -60, 0), help="where to put the build (default: on the superflat floor)")
    p.add_argument("--saves", default=None, help="saves directory (default: the Minecraft launcher's)")
    p.add_argument("--sink", type=int, default=0, help="lower the build by N blocks (e.g. 1 to set its floor into the ground)")
    p.add_argument("--input", action="append", default=[], help="T:ACTION:X,Y,Z (only printed, with world coordinates)")
    p.set_defaults(fn=cmd_world)
    p = sub.add_parser("bench", help="measure simulation speed")
    p.add_argument("--ticks", type=int, default=None, help="scale the run lengths (default 20000)")
    p.add_argument("--all", action="store_true", help="the full suite, not just the three headline builds")
    p.set_defaults(fn=cmd_bench)
    a = ap.parse_args(argv)
    from .oracle import OracleError
    try:
        return a.fn(a)
    except OracleError as e:
        print(f"redsim: error: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
