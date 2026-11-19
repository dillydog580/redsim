"""Random differential fuzzing against the real game.

Generates dense random builds from the supported components (random block states,
random inputs at random ticks, random absolute positions), runs each in the oracle
and in redsim, and minimizes any divergence to a small reproducible scenario.

    python -m redsim.fuzz --count 2000 --seed 1
"""

from __future__ import annotations

import argparse
import json
import os
import random
from dataclasses import replace
import sys
import time
from pathlib import Path

from . import _redsim
from .difftest import Scenario, check_many, compare, run_sim

ROOT = Path(__file__).resolve().parents[2]
REGRESSIONS = ROOT / "tests" / "regressions"

_TABLE = None


def table():
    global _TABLE
    if _TABLE is None:
        _TABLE = {b[0]: b for b in _redsim.blocks_table()}
    return _TABLE


def random_state(rng: random.Random, name: str, constrain=None) -> int:
    """Random state of block `name` (each property uniformly random unless constrained)."""
    name = name if ":" in name else "minecraft:" + name
    _, first, n, default, kind, props = table()[name]
    s = default
    vals = {}
    for pname, values in props:
        if constrain and pname in constrain:
            v = constrain[pname]
            v = v(rng) if callable(v) else v
        else:
            v = rng.choice(values)
        vals[pname] = v
    text = name + ("[" + ",".join(f"{k}={v}" for k, v in vals.items()) + "]" if vals else "")
    return _redsim.parse_state(text)


SOLIDS = [("stone", 50), ("glass", 10), ("slime_block", 10), ("honey_block", 5), ("obsidian", 4), ("redstone_block", 4),
          ("target", 3), ("redstone_lamp", 6), ("observer", 6), ("dirt", 4), ("note_block", 3), ("dispenser", 2),
          ("dropper", 2), ("hopper", 2), ("copper_bulb", 2), ("chest", 2), ("barrel", 1), ("composter", 1),
          ("cake", 1), ("end_portal_frame", 1), ("respawn_anchor", 1), ("smooth_stone_slab", 2), ("cobblestone_wall", 2),
          ("oak_fence", 2), ("nether_brick_fence", 1), ("glass_pane", 2), ("iron_bars", 1), ("oak_stairs", 2), ("stone_brick_stairs", 1),
          ("grass_block", 3), ("sand", 0), ("snow_block", 1), ("white_carpet", 1), ("tnt", 2), ("lectern", 2),
          ("jukebox", 2), ("chiseled_bookshelf", 2), ("crafter", 2), ("water_cauldron", 1), ("cauldron", 1),
          ("amethyst_block", 2), ("white_wool", 2), ("decorated_pot", 1), ("copper_golem_statue", 1)]
COMPONENTS = [("redstone_wire", 30), ("repeater", 12), ("comparator", 8), ("redstone_torch", 8), ("redstone_wall_torch", 6),
              ("lever", 6), ("stone_button", 3), ("oak_button", 2), ("piston", 8), ("sticky_piston", 8), ("observer", 6),
              ("redstone_lamp", 3), ("redstone_block", 2), ("piston_head", 1), ("target", 1), ("iron_trapdoor", 3),
              ("oak_trapdoor", 2), ("oak_fence_gate", 2), ("iron_door", 3), ("oak_door", 2), ("note_block", 2),
              ("stone_pressure_plate", 1), ("heavy_weighted_pressure_plate", 1), ("skeleton_skull", 1), ("copper_bulb", 2),
              ("dispenser", 1), ("hopper", 1), ("poppy", 2), ("short_grass", 2), ("oak_sapling", 1), ("dead_bush", 1),
              ("white_carpet", 1), ("torch", 1), ("cake", 1), ("lightning_rod", 2), ("bell", 2),
              ("rail", 4), ("powered_rail", 4), ("activator_rail", 2), ("detector_rail", 2),
              ("tripwire_hook", 3), ("tripwire", 4), ("daylight_detector", 3), ("sculk_sensor", 3),
              ("calibrated_sculk_sensor", 2), ("oak_shelf", 3), ("birch_shelf", 1), ("big_dripleaf", 2), ("big_dripleaf_stem", 1)]

CONSTRAINTS = {
    "piston": {"extended": lambda r: "true" if r.random() < 0.1 else "false"},
    "sticky_piston": {"extended": lambda r: "true" if r.random() < 0.1 else "false"},
    "redstone_wire": {"power": lambda r: str(r.choice([0, 0, 0, 0, r.randint(1, 15)]))},
    "observer": {"powered": lambda r: "true" if r.random() < 0.15 else "false"},
    "target": {"power": lambda r: "0"},
    "smooth_stone_slab": {"waterlogged": "false"},
    "oak_trapdoor": {"waterlogged": "false"},
    "iron_trapdoor": {"waterlogged": "false"},
    "cobblestone_wall": {"waterlogged": "false"},
    "oak_fence": {"waterlogged": "false"},
    "nether_brick_fence": {"waterlogged": "false"},
    "glass_pane": {"waterlogged": "false"},
    "iron_bars": {"waterlogged": "false"},
    "oak_stairs": {"waterlogged": "false"},
    "stone_brick_stairs": {"waterlogged": "false"},
    "chest": {"type": "single", "waterlogged": "false"},
    "stone_pressure_plate": {"powered": "false"},
    "composter": {"level": lambda r: str(r.randint(0, 7))},
    "lightning_rod": {"waterlogged": "false"},
    "crafter": {"crafting": "false"},
    "lectern": {"powered": lambda r: "true" if r.random() < 0.1 else "false"},
    "heavy_weighted_pressure_plate": {"power": "0"},
    "rail": {"waterlogged": "false"},
    "powered_rail": {"waterlogged": "false"},
    "activator_rail": {"waterlogged": "false"},
    "detector_rail": {"waterlogged": "false"},
    "oak_shelf": {"waterlogged": "false"},
    "birch_shelf": {"waterlogged": "false"},
    "big_dripleaf": {"waterlogged": "false"},
    "big_dripleaf_stem": {"waterlogged": "false"},
    "decorated_pot": {"waterlogged": "false"},
    "copper_golem_statue": {"waterlogged": "false"},
    "tripwire": {"powered": lambda r: "true" if r.random() < 0.2 else "false"},
    "daylight_detector": {"power": lambda r: str(r.choice([0, 0, r.randint(1, 15)]))},
    "sculk_sensor": {"waterlogged": "false", "sculk_sensor_phase": lambda r: r.choice(["inactive"] * 6 + ["active", "cooldown"]),
                     "power": lambda r: str(r.choice([0, 0, 0, r.randint(1, 15)]))},
    "calibrated_sculk_sensor": {"waterlogged": "false", "sculk_sensor_phase": lambda r: r.choice(["inactive"] * 6 + ["active", "cooldown"]),
                                "power": lambda r: str(r.choice([0, 0, 0, r.randint(1, 15)]))},
}

CONTAINERS = ("chest", "barrel", "hopper", "dropper", "oak_shelf", "birch_shelf", "decorated_pot")
CONTAINER_SIZE = {"hopper": 5, "dispenser": 9, "dropper": 9, "oak_shelf": 3, "birch_shelf": 3, "decorated_pot": 1}
ITEMS = ["minecraft:stone", "minecraft:ender_pearl", "minecraft:diamond_sword", "minecraft:redstone", "minecraft:snowball"]


def weighted(rng, items):
    tot = sum(w for _, w in items)
    x = rng.uniform(0, tot)
    for name, w in items:
        x -= w
        if x <= 0:
            return name
    return items[-1][0]


DAY_TIMES = [0, 1000, 6000, 11000, 12500, 13000, 13700, 18000, 22500, 23500]


def random_scenario(rng: random.Random, size=(7, 5, 7), ticks=None, density=None, day_time: int = 0) -> Scenario:
    sx, sy, sz = size
    density = density if density is not None else rng.uniform(0.35, 0.75)
    grid = {}
    for y in range(sy):
        for z in range(sz):
            for x in range(sx):
                if rng.random() > density:
                    continue
                below = grid.get((x, y - 1, z))
                below_solid = below is not None and below[1] == "solid"
                if (below_solid and rng.random() < 0.6) or rng.random() < 0.25:
                    name = weighted(rng, COMPONENTS)
                    st = random_state(rng, name, CONSTRAINTS.get(name))
                    if name.endswith("_door"):
                        if y + 1 >= sy or (x, y + 1, z) in grid:
                            continue
                        lower = _redsim.format_state(st).replace("half=upper", "half=lower")
                        st = _redsim.parse_state(lower)
                        grid[(x, y + 1, z)] = (_redsim.parse_state(lower.replace("half=lower", "half=upper")), "comp")
                    grid[(x, y, z)] = (st, "comp")
                else:
                    name = weighted(rng, SOLIDS)
                    grid[(x, y, z)] = (random_state(rng, name, CONSTRAINTS.get(name)), "solid")
    blocks = [(x, y, z, s) for (x, y, z), (s, _) in sorted(grid.items(), key=lambda kv: (kv[0][1], kv[0][2], kv[0][0]))]
    block_entities = []
    for (x, y, z, s) in blocks:
        bn = _redsim.block_name(s)
        if bn == "minecraft:lectern" and "has_book=true" in _redsim.format_state(s):
            pages = rng.randint(1, 15)
            block_entities.append({"pos": [x, y, z], "lectern": [rng.randrange(pages), pages]})
        elif bn == "minecraft:chiseled_bookshelf":
            block_entities.append({"pos": [x, y, z], "bookshelf": rng.randint(-1, 5)})
        elif bn == "minecraft:jukebox" and "has_record=true" in _redsim.format_state(s):
            block_entities.append({"pos": [x, y, z], "jukebox": rng.choice(["minecraft:music_disc_cat", "minecraft:music_disc_11",
                                                                           "minecraft:music_disc_5", "minecraft:music_disc_strad"])})
        if any(bn == "minecraft:" + c for c in CONTAINERS) and rng.random() < 0.6:
            csize = CONTAINER_SIZE.get(_redsim.block_name(s).split(":")[1], 27)
            nslots = 1 if "dropper" in _redsim.block_name(s) else rng.randint(1, min(csize, 6))  # >1 = random slot choice
            items = [[slot, rng.choice(ITEMS), rng.randint(1, 64)] for slot in rng.sample(range(csize), nslots)]
            items = [[sl, it, min(c, {"minecraft:ender_pearl": 16, "minecraft:diamond_sword": 1, "minecraft:snowball": 16}.get(it, 64))] for sl, it, c in items]
            block_entities.append({"pos": [x, y, z], "items": items})
    ticks = ticks or rng.randint(20, 60)
    actions = []
    usable = [(x, y, z) for (x, y, z, s) in blocks
              if any(k in _redsim.block_name(s) for k in ("lever", "button", "repeater", "comparator", "door", "fence_gate", "note_block",
                                                          "daylight_detector"))]
    lecterns = [(x, y, z) for (x, y, z, s) in blocks if "lectern" in _redsim.block_name(s) and "has_book=true" in _redsim.format_state(s)]
    for _ in range(rng.randint(1, 8)):
        at = rng.randint(0, max(1, ticks // 2))
        r = rng.random()
        if lecterns and r < 0.15:
            actions.append({"at": at, "type": "turn_page", "pos": list(rng.choice(lecterns)), "page": rng.randint(0, 14)})
            continue
        if usable and r < 0.55:
            actions.append({"at": at, "type": "use", "pos": list(rng.choice(usable))})
        else:
            pos = [rng.randrange(sx), rng.randrange(sy), rng.randrange(sz)]
            choice = rng.random()
            if choice < 0.3:
                st = "minecraft:redstone_block"
            elif choice < 0.55:
                st = "minecraft:air"
            elif choice < 0.7:
                st = "minecraft:stone"
            else:
                name = weighted(rng, COMPONENTS)
                st = random_state(rng, name, CONSTRAINTS.get(name))
                if not name.endswith("_door") and rng.random() < 0.3:
                    # placed like a player (setBlock, then setPlacedBy: repeaters, pistons, hooks...)
                    actions.append({"at": at, "type": "place", "pos": pos, "state": st})
                    continue
            actions.append({"at": at, "type": "setblock", "pos": pos, "state": st})
    actions.sort(key=lambda a: a["at"])
    offset = (rng.randrange(16), rng.randrange(40), rng.randrange(16))
    return Scenario(blocks=blocks, size=size, ticks=ticks, actions=actions, block_entities=block_entities, offset=offset,
                    day_time=day_time)


def _prune_bes(s: Scenario) -> Scenario:
    present = {(b[0], b[1], b[2]) for b in s.blocks}
    s.block_entities = [be for be in s.block_entities if tuple(be["pos"]) in present]
    return s


def _diverges(oracle, scens):
    scens = [_prune_bes(s) for s in scens]
    res = check_many(oracle, scens)
    return [(s, d if (d is not None and not d.warnings) else None) for (s, d, r) in res]


def minimize(oracle, scen: Scenario, max_rounds: int = 60) -> Scenario:
    """Greedy parallel reduction: drop blocks/actions while the divergence persists."""
    cur = scen
    for _ in range(max_rounds):
        cands = []
        # drop actions one at a time
        for i in range(len(cur.actions)):
            cands.append(replace(cur, actions=cur.actions[:i] + cur.actions[i + 1:]))
        # drop blocks in chunks, then single
        n = len(cur.blocks)
        for chunk in (max(1, n // 4), 1) if n > 4 else (1,):
            for i in range(0, n, chunk):
                cands.append(replace(cur, blocks=cur.blocks[:i] + cur.blocks[i + chunk:]))
        found = None
        for k in range(0, len(cands), 36):
            batch = cands[k:k + 36]
            for s, d in _diverges(oracle, batch):
                if d is not None:
                    found = s
                    break
            if found:
                break
        if not found:
            break
        cur = found
    # shorten ticks
    lo = 1
    for t in range(1, cur.ticks):
        s = replace(cur, ticks=t, actions=[a for a in cur.actions if a["at"] < t])
        if _diverges(oracle, [s])[0][1] is not None:
            return s
    return cur


def save_regression(scen: Scenario, note: str, oracle=None, directory: Path = REGRESSIONS, name: str | None = None) -> Path:
    """Save a scenario (with the game's recorded result, if an oracle is given) as a test case."""
    import hashlib
    directory.mkdir(parents=True, exist_ok=True)
    blocks = [(x, y, z, _redsim.format_state(s) if isinstance(s, int) else s) for (x, y, z, s) in scen.blocks]
    actions = [dict(a, state=_redsim.format_state(a["state"])) if isinstance(a.get("state"), int) else a for a in scen.actions]
    data = {"name": name or scen.name or "fuzz", "note": note, "mc_version": _redsim.mc_version(), "blocks": blocks,
            "size": list(scen.size), "ticks": scen.ticks, "actions": actions, "block_entities": scen.block_entities,
            "offset": list(scen.offset) if scen.offset else None}
    if scen.day_time:
        data["day_time"] = scen.day_time
    if oracle is not None:
        oracle.set_day_time(scen.day_time)
        res = oracle.request(replace(scen, blocks=blocks, actions=actions).to_request())
        data["expected"] = {"origin": res["origin"], "start_time": res["start_time"], "steps": res["steps"]}
    h = hashlib.sha1(json.dumps({k: data[k] for k in ("blocks", "actions", "ticks", "offset")}, sort_keys=True).encode()).hexdigest()[:10]
    p = directory / f"{name or 'fuzz'}_{h}.json"
    p.write_text(json.dumps(data, separators=(",", ":")))
    return p


def focus(names, factor: int = 10):
    """Make the named blocks much more common in generated scenarios (both palettes)."""
    names = [n.strip().replace("minecraft:", "") for n in names if n.strip()]
    for pal in (COMPONENTS, SOLIDS):
        for i, (n, w) in enumerate(pal):
            if n in names:
                pal[i] = (n, max(w, 1) * factor)
    known = {n for n, _ in COMPONENTS} | {n for n, _ in SOLIDS}
    for n in names:
        if n not in known:
            COMPONENTS.append((n, 10 * factor))


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("--count", type=int, default=500)
    ap.add_argument("--seed", type=int, default=int(time.time()))
    ap.add_argument("--size", default="7,5,7")
    ap.add_argument("--no-minimize", action="store_true")
    ap.add_argument("--stop-after", type=int, default=5, help="stop after this many distinct divergences")
    ap.add_argument("--focus", default="", help="comma-separated block names to make much more common (e.g. rail,powered_rail)")
    args = ap.parse_args(argv)
    if args.focus:
        focus(args.focus.split(","))
    from .oracle import Oracle
    size = tuple(int(v) for v in args.size.split(","))
    rng = random.Random(args.seed)
    found = 0
    done = 0
    warned = 0
    t0 = time.time()
    with Oracle() as oracle:
        while done < args.count and found < args.stop_after:
            # one day time per batch: the oracle's day clock is shared by all its slots
            day_time = rng.choice(DAY_TIMES + [rng.randrange(24000)])
            batch = [random_scenario(rng, size, day_time=day_time) for _ in range(min(36, args.count - done))]
            for s, d, r in check_many(oracle, batch):
                done += 1
                if d is None:
                    continue
                if d.warnings:
                    warned += 1
                    continue
                if isinstance(r, dict) and not r.get("ok", True):
                    print("oracle error:", r.get("error"), flush=True)
                    continue
                found += 1
                print(f"\nDIVERGENCE #{found} (scenario {done}):\n{d.describe()}", flush=True)
                if not args.no_minimize:
                    m = minimize(oracle, s)
                    _, dm, _ = check_many(oracle, [m])[0]
                    note = dm.describe() if dm else d.describe()
                    p = save_regression(m, note, oracle)
                    print(f"minimized to {len(m.blocks)} blocks, {len(m.actions)} actions, {m.ticks} ticks -> {p}")
                    print(note, flush=True)
            print(f"{done} scenarios, {found} divergences, {warned} diverged with a not-simulated warning, "
                  f"{done / (time.time() - t0):.1f}/s", flush=True)
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
