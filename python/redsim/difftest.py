"""Differential testing: run a scenario in the real game (oracle) and in redsim,
compare every block state (and redstone-relevant block entity data) on every tick.

A scenario is plain data, so the same thing runs on both sides:

    blocks:  [(x, y, z, state)]   relative to the scenario origin, placed raw (no updates)
    size:    (sx, sy, sz)         region that is recorded and compared
    ticks:   number of game ticks to run
    actions: [{"at": t, "type": "use"|"press"|"setblock"|"destroy", "pos": [x,y,z], ...}]
             actions with at=t happen between tick t and t+1 (at=0: before the first tick)
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Any

from . import _redsim


@dataclass
class Scenario:
    blocks: list
    size: tuple
    ticks: int
    actions: list = field(default_factory=list)
    block_entities: list = field(default_factory=list)
    offset: tuple | None = None
    name: str = ""
    origin: tuple | None = None   # absolute world position of the region's min corner (in the game too)
    day_time: int = 0             # overworld day clock during the run (time does not advance)

    def to_request(self) -> dict:
        req = {"op": "run", "size": list(self.size), "ticks": self.ticks,
               "blocks": [[int(b[0]), int(b[1]), int(b[2]), b[3] if isinstance(b[3], int) else str(b[3])] for b in self.blocks],
               "actions": list(self.actions), "block_entities": list(self.block_entities)}
        if self.offset is not None:
            req["offset"] = list(self.offset)
        if self.origin is not None:
            req["abs_origin"] = list(self.origin)
        return req

    def to_json(self) -> str:
        d = {"name": self.name, "blocks": self.blocks, "size": self.size, "ticks": self.ticks,
             "actions": self.actions, "block_entities": self.block_entities, "offset": self.offset}
        if self.day_time:
            d["day_time"] = self.day_time
        return json.dumps(d)

    @staticmethod
    def from_json(s: str) -> "Scenario":
        d = json.loads(s)
        return Scenario(blocks=[tuple(b) for b in d["blocks"]], size=tuple(d["size"]), ticks=d["ticks"],
                        actions=d.get("actions", []), block_entities=d.get("block_entities", []),
                        offset=tuple(d["offset"]) if d.get("offset") else None, name=d.get("name", ""),
                        day_time=d.get("day_time", 0))


def _state_id(s) -> int:
    return s if isinstance(s, int) else _redsim.parse_state(s)


class _Recorder:
    def __init__(self, world, origin, size):
        self.w, self.o, self.size = world, origin, size
        ox, oy, oz = origin
        sx, sy, sz = size
        self.box = (ox, oy, oz, ox + sx - 1, oy + sy - 1, oz + sz - 1)
        self.last = self._snap()
        self.last_be = self._be()

    def _snap(self):
        import array
        a = array.array("I")
        a.frombytes(self.w.region(*self.box))
        return a

    def _be(self):
        ox, oy, oz = self.o
        sx, sy, sz = self.size
        out = {}
        for (x, y, z) in self.w.block_entity_positions():
            rx, ry, rz = x - ox, y - oy, z - oz
            if 0 <= rx < sx and 0 <= ry < sy and 0 <= rz < sz:
                d = self.w.block_entity(x, y, z)
                if d is not None:
                    out[(rx, ry, rz)] = d
        return out

    def diff(self):
        sx, sy, sz = self.size
        now = self._snap()
        changes = []
        last = self.last
        for i in range(len(now)):
            if now[i] != last[i]:
                x = i % sx
                z = (i // sx) % sz
                y = i // (sx * sz)
                changes.append([x, y, z, now[i]])
        self.last = now
        return changes

    def diff_be(self):
        now = self._be()
        out = []
        for k, v in now.items():
            if self.last_be.get(k) != v:
                out.append([k[0], k[1], k[2], v])
        for k in self.last_be:
            if k not in now:
                out.append([k[0], k[1], k[2], None])
        self.last_be = now
        return out


def apply_action(w, a, origin):
    ox, oy, oz = origin
    x, y, z = a["pos"]
    x, y, z = x + ox, y + oy, z + oz
    t = a["type"]
    if t == "use":
        w.use_block(x, y, z)
    elif t == "press":
        s = w.get_str(x, y, z)
        if "_button" in s and "powered=false" in s:
            w.use_block(x, y, z)
    elif t == "setblock":
        w.set_block(x, y, z, _state_id(a["state"]), a.get("flags", 3))
    elif t == "destroy":
        w.destroy_block(x, y, z)
    elif t == "turn_page":
        w.lectern_turn_page(x, y, z, a["page"])
    elif t == "place":
        w.place_like_player(x, y, z, _state_id(a["state"]))
    elif t == "player_place":
        sid = _state_id(a["state"])
        if _redsim.block_name(sid) == "minecraft:redstone_wire":
            sid = w.dust_placement_state(x, y, z)
        elif _redsim.block_name(sid) == "minecraft:tripwire":
            sid = w.tripwire_placement_state(x, y, z, sid)
        else:
            raise ValueError("player_place is only modelled for redstone dust and tripwire")
        w.place_like_player(x, y, z, sid)
    else:
        raise ValueError(f"unknown action {t}")


def run_sim(scen: Scenario, origin, start_time: int, world=None) -> dict:
    """Run `scen` in redsim exactly the way the oracle runs it."""
    w = world or _redsim.World()
    w.game_time = start_time
    w.day_time = scen.day_time
    w.advance_time = False  # the oracle runs with the advance_time game rule off
    ox, oy, oz = origin
    for b in scen.blocks:
        w.place(b[0] + ox, b[1] + oy, b[2] + oz, _state_id(b[3]))
    for be in scen.block_entities:
        x, y, z = be["pos"]
        try:
            if "output" in be:
                w.set_comparator_output(x + ox, y + oy, z + oz, be["output"])
            if "items" in be:
                w.set_container_items(x + ox, y + oy, z + oz, [tuple(i) for i in be["items"]])
            if "lectern" in be:
                w.set_lectern(x + ox, y + oy, z + oz, be["lectern"][0], be["lectern"][1])
            if "bookshelf" in be:
                w.set_bookshelf_slot(x + ox, y + oy, z + oz, be["bookshelf"])
            if "jukebox" in be:
                w.set_jukebox(x + ox, y + oy, z + oz, be["jukebox"], 0)
        except KeyError:
            pass  # like the game: data for a block entity that does not exist is ignored
    rec = _Recorder(w, origin, scen.size)
    rec.last_be = {}
    sx, sy, sz = scen.size
    initial = [[i % sx, i // (sx * sz), (i // sx) % sz, v] for i, v in enumerate(rec.last) if v != 0]
    steps = [{"t": 0, "tick": initial, "be": rec.diff_be()}]

    def run_actions(step):
        acts = [a for a in scen.actions if a["at"] == step]
        if acts:
            for a in acts:
                apply_action(w, a, origin)
            steps[-1]["act"] = rec.diff()
            steps[-1]["act_be"] = rec.diff_be()

    run_actions(0)
    for t in range(1, scen.ticks + 1):
        w.tick()
        steps.append({"t": t, "tick": rec.diff(), "be": rec.diff_be()})
        if t < scen.ticks:
            run_actions(t)
    return {"steps": steps, "world": w}


@dataclass
class Divergence:
    step: int
    phase: str  # "tick" | "act" | "be" | "act_be"
    oracle: dict
    sim: dict
    warnings: list = field(default_factory=list)  # sim warnings: behaviour it knowingly did not simulate

    def describe(self, limit: int = 12) -> str:
        lines = [f"first divergence at step {self.step} ({self.phase})"]
        keys = sorted(set(self.oracle) | set(self.sim))
        for k in keys[:limit]:
            o, s = self.oracle.get(k), self.sim.get(k)
            fmt = (lambda v: _redsim.format_state(v) if isinstance(v, int) else str(v))
            lines.append(f"  {k}: game={fmt(o) if o is not None else '-'}  sim={fmt(s) if s is not None else '-'}")
        if len(keys) > limit:
            lines.append(f"  ... {len(keys) - limit} more")
        return "\n".join(lines)


def compare(oracle_steps, sim_steps) -> Divergence | None:
    """Compare cumulative region state step by step; returns the first divergence."""
    for i, (o, s) in enumerate(zip(oracle_steps, sim_steps)):
        for phase in ("tick", "be", "act", "act_be"):
            ol = {tuple(c[:3]): c[3] for c in o.get(phase, [])}
            sl = {tuple(c[:3]): c[3] for c in s.get(phase, [])}
            if ol != sl:
                diff_o = {k: v for k, v in ol.items() if sl.get(k, "<none>") != v}
                diff_s = {k: v for k, v in sl.items() if ol.get(k, "<none>") != v}
                return Divergence(o["t"], phase, diff_o, diff_s)
    if len(oracle_steps) != len(sim_steps):
        return Divergence(min(len(oracle_steps), len(sim_steps)), "length", {}, {})
    return None


def check(oracle, scen: Scenario):
    """Run one scenario on both sides; returns (divergence or None, oracle_result, sim_result)."""
    oracle.set_day_time(scen.day_time)
    res = oracle.request(scen.to_request())
    sim = run_sim(scen, res["origin"], res["start_time"])
    return compare(res["steps"], sim["steps"]), res, sim


def check_many(oracle, scens: list[Scenario]):
    """Pipelined: submits all scenarios to the oracle at once (it runs them in parallel slots)."""
    out = [None] * len(scens)
    groups: dict[int, list[int]] = {}
    for i, s in enumerate(scens):
        groups.setdefault(s.day_time, []).append(i)
    # the day clock is shared by all oracle slots: run one day time at a time
    for day_time in sorted(groups, key=lambda t: (t != oracle.day_time, t)):
        oracle.set_day_time(day_time)
        queues = [(i, oracle.submit(scens[i].to_request())) for i in groups[day_time]]
        for i, q in queues:
            s = scens[i]
            res = q.get(timeout=900)
            if not res.get("ok"):
                out[i] = (s, None, res)
                continue
            sim = run_sim(s, res["origin"], res["start_time"])
            d = compare(res["steps"], sim["steps"])
            if d is not None:
                d.warnings = sim["world"].warnings()
            out[i] = (s, d, res)
    return out
