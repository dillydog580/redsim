"""Wiring assistant: lay a redstone dust line from a signal source to a target that stays separate
from all other circuitry, then prove it with `World.nets()` and a simulation.

    import redsim as rs
    from redsim.route import route

    w = rs.World()
    w.fill((-2, 0, -4), (14, 0, 4), "stone")
    w.lever((0, 1, 0))                       # the source
    w.set((12, 1, 0), "redstone_lamp")       # the target
    r = route(w, (0, 1, 0), (12, 1, 0))      # lever -> lamp; raises RouteError if impossible
    print(r.report)                          # path, repeaters, delay and every check that passed
    w.use((0, 1, 0)); w.tick(r.delay_ticks)
    assert w[(12, 1, 0)] == "redstone_lamp[lit=true]"

Semantics
---------
start  The SOURCE block itself (not the first dust): a lever, button, redstone torch, repeater,
       comparator, observer, redstone block (any signal source), or a conductor block that something
       else strongly powers (e.g. the block a lever is attached to, the block a repeater points into).
       The first dust goes in a free cell next to it that the source actually feeds: the output side
       of a repeater/comparator, the back of an observer, the sides (and top/bottom where dust can
       stand) of a lever, button, torch or redstone block, the top/sides/bottom of a conductor.
       Planning assumes the source gives `source_power` (15) to the first dust.
end    The block to POWER: a lamp, piston, door, trapdoor, fence gate, note block, dispenser,
       dropper, hopper, copper bulb, a repeater/comparator (fed from its input side), or a plain
       conductor block (which then powers its own neighbours, on purpose). The last dust either sits
       on top of it (not on a piston: an extending piston drops it) or stands next to it pointing
       into it. A target that conducts (lamp, note block, dispenser...) passes the power on to its
       own neighbours, like any powered block: if that would switch something else, the route is
       refused unless that position is listed in `allow`.
       If `end` is an AIR cell instead, the last dust goes exactly there (nothing gets powered; use
       this to bring a signal to a spot you will build at later).

Everything the route adds is new: dust, the `support` block under every dust/repeater that has
nothing to stand on, and repeaters (`allow_repeaters`) wherever the signal would otherwise die out
(each dust costs 1 power; a repeater restores 15 and costs 2 game ticks per delay step).

Steps follow the game's rules for which way power flows between dust at different heights:
a step UP from dust L to dust U (one block over, one up) carries power only if the block above L
is not a conductor; a step DOWN carries power only if the block the upper dust stands on IS a
conductor and the block above the lower dust is not. So with a transparent `support` (glass) the
route can climb but never descend.

What "separate" means (checked for every cell while searching)
---------------------------------------------------------------
1. No other dust within connection reach of a route cell: its 4 sides, and diagonally one block
   up or down (12 cells), whatever is between. Nothing can connect to it or exchange power.
2. Nothing else can feed it or connect to it: no signal source that powers or connects to a route
   cell (a repeater standing sideways or an observer not facing it with its back is fine; torches,
   levers, buttons, comparators, redstone blocks never are), and no conductor next to a route cell
   that another source strongly powers (a repeater's or observer's output side, the block above a
   torch, a lever's or button's own block...). The start source next to the first dust is the
   only exception.
3. It powers nothing else. A dust powers the block under it and each block its shape points into
   (a lone dust is a cross and points everywhere, a line points along itself only). None of those
   may be a component other than the target (lamps, pistons, doors, trapdoors, fence gates, note
   blocks, dispensers, droppers, hoppers, copper bulbs, repeaters/comparators fed from that side,
   unknown blocks). A powered conductor passes power to its 6 neighbours (same check; a torch on
   it would turn off), and every block that receives power also quasi-powers a piston, dispenser or
   dropper right below it (checked). A conductor the route powers may not also be powered by other
   dust (the two lines would share a block).
4. New blocks change nothing else and nothing moves them: none next to other dust, next to
   fences/walls/panes (they would connect), in a cell an observer watches, in the push path of a
   piston, or standing on a block a piston can push.
5. The route never runs next to itself (no shortcuts, so a repeater can never loop back) and does
   not come back next to its source or its target.

Verification (`verify=True`, the default) is done on copies before anything is written:
* the route is laid on a copy; every dust gets the shape the game computes, which must match the
  planned shape; no pre-existing block would change shape (fences, other dust...);
* `World.nets()`: the new dust forms its own network(s) without any pre-existing dust, powers
  nothing unintended, has no signal source but the start (and its own repeaters); every
  pre-existing network is identical to before;
* simulation: two copies, one with the route and one without, get the same input (the start
  lever is flipped on / the button pressed; any other source is replaced by a redstone block,
  after first being replaced by glass and the world settled). Every block's history must be the
  same in both except the target and what the target itself then does (traced with `explain`),
  the target must get power, and with the source off the route must stay at power 0.
If a check fails, the cells near the problem are excluded and the search runs again (`retries`);
the world is only modified once everything passed.
"""

from __future__ import annotations

import heapq
import itertools
import json
from collections import Counter
from dataclasses import dataclass, field

from . import _redsim

__all__ = ["route", "RouteResult", "RouteError"]

DIRS = {"north": (0, 0, -1), "east": (1, 0, 0), "south": (0, 0, 1), "west": (-1, 0, 0)}
HORIZ = ("north", "east", "south", "west")
OPP = {"north": "south", "south": "north", "east": "west", "west": "east", "up": "down", "down": "up"}
VEC6 = {**DIRS, "up": (0, 1, 0), "down": (0, -1, 0)}

# Block kinds (redsim's classification) that do nothing when they receive redstone power.
_INERT = frozenset({"Air", "Plain", "WaterlogOnly", "Wall", "Fence", "Bars", "Stairs", "Cake", "Carpet",
                    "DecoTorch", "DecoWallTorch", "Vegetation", "Snowy", "Container", "Composter",
                    "Observer", "Lever", "Button", "PressurePlate", "PoweredBlock", "Target"})
# Blocks that also react to power arriving at the block above them (quasi-connectivity).
_QC = frozenset({"Piston", "Dispenser"})
# Search costs on top of 1 per route cell: prefer existing ground, flat runs, few repeaters.
_SUPPORT_COST = 1.0
_STEP_COST = 0.5
_REPEATER_COST = 2.0
# Blocks whose shape follows a solid neighbour (a new support next to them would change them).
_SHAPE_SENSITIVE = frozenset({"Wall", "Fence", "Bars"})


def _p(pos) -> tuple:
    if isinstance(pos, str):
        pos = pos.replace(",", " ").split()
    x, y, z = pos
    return int(x), int(y), int(z)


def _sid(block) -> int:
    if isinstance(block, int):
        return block
    if hasattr(block, "id"):
        return block.id
    text = str(block).strip()
    if ":" not in text.split("[")[0]:
        text = "minecraft:" + text
    return _redsim.parse_state(text)


def _add(p, v, k=1):
    return (p[0] + v[0] * k, p[1] + v[1] * k, p[2] + v[2] * k)


def _up(p):
    return (p[0], p[1] + 1, p[2])


def _down(p):
    return (p[0], p[1] - 1, p[2])


def _n6(p):
    return [_add(p, v) for v in VEC6.values()]


def _n12(p):
    """The 4 horizontal neighbours on the same level, one level up and one level down: every cell a
    dust at p can connect to or exchange power with."""
    out = []
    for v in DIRS.values():
        for dy in (-1, 0, 1):
            out.append((p[0] + v[0], p[1] + dy, p[2] + v[2]))
    return out


def _hdir(a, b):
    """Name of the horizontal direction from a to b when b's column is next to a's, else None."""
    dx, dz = b[0] - a[0], b[2] - a[2]
    for name, (vx, _, vz) in DIRS.items():
        if (dx, dz) == (vx, vz):
            return name
    return None


def _shape(conns) -> frozenset:
    """Sides a dust points into, given the sides it connects to (RedstoneWireBlock.getConnectionState):
    no connection -> cross; one connection or two in line -> a line; otherwise just the connections."""
    s = {c for c in conns if c}
    ns_empty = "north" not in s and "south" not in s
    ew_empty = "east" not in s and "west" not in s
    if ns_empty:
        s |= {"east", "west"}
    if ew_empty:
        s |= {"north", "south"}
    return frozenset(s)


class _Info:
    """Static facts about one block state."""
    __slots__ = ("sid", "name", "kind", "conducts", "source", "dust_top", "rigid_top", "props")

    def __init__(self, sid: int):
        self.sid = sid
        self.name = _redsim.block_name(sid).split(":", 1)[1]
        self.kind = _redsim.block_kind(sid)
        flags, full, _center, rigid, _push, _speed = _redsim.state_info(sid)
        self.conducts = bool(flags & 1)
        self.source = bool(flags & 2)
        self.dust_top = bool(full & 2) or self.kind == "Hopper"     # dust can stand on it (Up face sturdy)
        self.rigid_top = bool(rigid & 2)                             # a repeater can stand on it
        st = _redsim.format_state(sid)
        self.props = dict(kv.split("=", 1) for kv in st[st.index("[") + 1:-1].split(",")) if "[" in st else {}

    @property
    def air(self):
        return self.kind == "Air"


_INFO: dict = {}


def _info(sid: int) -> _Info:
    i = _INFO.get(sid)
    if i is None:
        i = _INFO[sid] = _Info(sid)
    return i


# ---------------------------------------------------------------------------------------- results

class RouteError(Exception):
    """No safe route was found, or the found route failed verification. The world is unchanged.
    `.reasons` counts why candidate cells were rejected (most common first in the message)."""

    def __init__(self, message: str, reasons: Counter | None = None, examples: dict | None = None):
        self.reasons = reasons or Counter()
        self.examples = examples or {}
        lines = [message]
        if self.reasons:
            lines.append("most common reasons cells were rejected:")
            for r, n in self.reasons.most_common(8):
                ex = self.examples.get(r)
                lines.append(f"  {n:6d} x {r}" + (f"  (e.g. {ex})" if ex else ""))
        self.report = "\n".join(lines)
        super().__init__(self.report)


@dataclass
class RouteResult:
    source: tuple                     # the start (source) block
    target: tuple | None              # the block powered (None when `end` was an air cell)
    path: list                        # route cells in signal order (dust and repeaters)
    dust: list                        # [(pos, planned power with the source on)]
    repeaters: list                   # [(pos, toward)]
    supports: list                    # support blocks placed under dust/repeaters
    placed: list                      # [(pos, state)] every block written, in order
    previous: dict                    # pos -> state before (all air); used by undo()
    delay_ticks: int                  # game ticks added by repeaters (2 per delay step)
    end_power: int                    # power of the last dust with the source on (simulated if verified)
    nets: list = field(default_factory=list)      # the route's own redsim.nets.Net objects
    checks: list = field(default_factory=list)    # verification steps that passed
    notes: list = field(default_factory=list)     # things worth knowing (not failures)
    report: str = ""

    def undo(self, world) -> None:
        """Remove the route again (raw, like `World.set`): every placed block gets its old state."""
        for pos, _ in reversed(self.placed):
            world._w.place(*pos, _sid(self.previous[pos]))

    def __str__(self):
        return self.report


# ---------------------------------------------------------------------------------------- planner

class _Node:
    __slots__ = ("pos", "conn", "power", "g", "parent", "cells", "supports", "clear", "first", "terminal",
                 "sup", "rep", "dy")

    def __init__(self, pos, conn, power, g, parent, cells, supports, clear, first, terminal, sup, rep, dy):
        self.pos = pos            # dust position
        self.conn = conn          # horizontal side toward the previous element (or the source), or None
        self.power = power        # planned power of this dust with the source on
        self.g = g
        self.parent = parent
        self.cells = cells        # frozenset: own dust + repeater positions so far
        self.supports = supports  # frozenset: support blocks to place so far
        self.clear = clear        # frozenset: cells that must stay free of own blocks (above own dust)
        self.first = first
        self.terminal = terminal  # next to the target: may only end here
        self.sup = sup            # new support under this dust, or None
        self.rep = rep            # (repeater_pos, toward, new_support_or_None) placed right before this dust
        self.dy = dy              # height change from the previous element


class _Ctx:
    __slots__ = ("cells", "supports", "first")

    def __init__(self, cells, supports, first):
        self.cells, self.supports, self.first = cells, supports, first


class _Planner:
    def __init__(self, world, source, end, support, box, allow_repeaters, repeater_delay, source_power, allow,
                 banned, margin):
        self.world = world
        self.w = world._w
        self.S0 = source
        self.sup = _info(_sid(support))
        if not (self.sup.dust_top and self.sup.rigid_top):
            raise ValueError(f"support {support!r}: dust and repeaters cannot stand on it")
        self.allow_repeaters = allow_repeaters
        self.repeater_delay = repeater_delay
        self.source_power = source_power
        self.banned = banned
        self.reasons = Counter()
        self.examples = {}
        self._at = {}
        self._cache = {}

        sb = self.at(source)
        if sb.air:
            raise ValueError(f"start {source} is air: give the source block (lever, torch, repeater, powered block...)")
        if sb.kind == "Wire":
            raise ValueError(f"start {source} is redstone dust: routing out of existing dust would join its network; "
                             "give the block that feeds it instead")
        if not sb.source and not sb.conducts:
            raise ValueError(f"start {source} is {sb.name}: neither a signal source nor a conductor that could be powered")
        eb = self.at(end)
        if eb.kind == "Wire":
            raise ValueError(f"end {end} is redstone dust of another network; give the block to power, or an air cell")
        if end == source:
            raise ValueError("start and end are the same block")
        if eb.air:
            self.T, self.end_dust = None, end
            self.end_cells = [end]
        else:
            self.T, self.end_dust = end, None
            self.end_cells = self._end_cells()
        tb = self.at(self.T) if self.T else None
        # a target that is a plain conductor is a relay: powering its neighbours is the point
        self.T_relay = bool(tb and tb.conducts and tb.kind in _INERT)
        self.intended = set(allow)
        if self.T:
            self.intended.add(self.T)
            if self.T_relay:
                for n in _n6(self.T):
                    self.intended.add(n)
                    self.intended.add(_down(n))

        if box is None:
            pts = [source, end]
            lo = [min(p[k] for p in pts) for k in range(3)]
            hi = [max(p[k] for p in pts) for k in range(3)]
            box = ((lo[0] - margin, lo[1] - 4, lo[2] - margin), (hi[0] + margin, hi[1] + 4, hi[2] + margin))
        (x0, y0, z0), (x1, y1, z1) = _p(box[0]), _p(box[1])
        self.box = ((min(x0, x1), max(min(y0, y1), -64), min(z0, z1)), (max(x0, x1), min(max(y0, y1), 319), max(z0, z1)))
        self._scan()

    # ------------------------------------------------------------------ world facts

    def at(self, p) -> _Info:
        i = self._at.get(p)
        if i is None:
            i = self._at[p] = _info(self.w.get(*p))
        return i

    def in_box(self, p):
        (x0, y0, z0), (x1, y1, z1) = self.box
        return x0 <= p[0] <= x1 and y0 <= p[1] <= y1 and z0 <= p[2] <= z1

    def _scan(self):
        """One pass over the world: other dust, what it powers, observers' watched cells, piston paths."""
        self.dust = set()
        self.watched = set()
        self.piston_paths = set()
        for (x, y, z, sid) in self.w.blocks():
            p = (x, y, z)
            b = _info(sid)
            self._at[p] = b
            if b.kind == "Wire":
                self.dust.add(p)
            elif b.kind == "Observer":
                self.watched.add(_add(p, VEC6[b.props["facing"]]))
            elif b.kind == "Piston":
                f = VEC6[b.props["facing"]]
                for k in range(1, 14):              # the head cell, the pushed blocks, the cell they move into
                    c = _add(p, f, k)
                    self.piston_paths.add(c)
                    if self.at(c).air:
                        break
                if b.name == "sticky_piston":
                    self.piston_paths.add(_add(p, f, 2))   # the block a retracting sticky piston pulls
        self.dust_out = set()                       # blocks other dust sends power into
        for p in self.dust:
            for o in self.w.dust_outputs(*p):
                self.dust_out.add(tuple(o))

    def _end_cells(self):
        T, tb = self.T, self.at(self.T)
        if tb.kind in ("Repeater", "Comparator"):
            return [_add(T, VEC6[tb.props["facing"]])]        # the input side
        cells = [_add(T, DIRS[d]) for d in HORIZ]
        # dust on top, unless the target stops holding it when it reacts (an extending piston's base
        # is no longer a full block, so dust on it pops off; a piston facing up pushes it)
        if tb.dust_top and not (tb.kind == "Piston" and tb.props.get("facing") != "down"):
            cells.insert(0, _up(T))
        return cells

    def connectable(self, b: _Info, d: str) -> bool:
        """Would dust connect toward a block b lying in direction d from it (shouldConnectTo)?"""
        if b.kind == "Wire":
            return True
        if b.kind == "Repeater":
            return b.props["facing"] in (d, OPP[d])
        if b.kind == "Observer":
            return b.props["facing"] == d
        return b.source

    # ------------------------------------------------------------------ rejection bookkeeping

    def _no(self, reason, detail=None):
        self.reasons[reason] += 1
        if detail and reason not in self.examples:
            self.examples[reason] = detail
        return reason

    # ------------------------------------------------------------------ static cell rules

    def cell_problem(self, p):
        """Rules for a dust/repeater cell that depend only on the world (cached)."""
        key = ("cell", p)
        if key in self._cache:
            return self._cache[key]
        r = None
        if not self.in_box(p):
            r = ("outside the search box", None)
        elif p in self.banned:
            r = ("excluded after a failed check", str(p))
        elif not self.at(p).air:
            r = ("cell not empty", None)
        elif p in self.watched:
            r = ("an observer watches this cell", str(p))
        elif p in self.piston_paths:
            r = ("in a piston's push path", str(p))
        else:
            for q in _n12(p):
                if q in self.dust:
                    r = ("too close to other dust", f"{p} next to dust at {q}")
                    break
        self._cache[key] = r
        return r

    def touches(self, b: _Info, m, p) -> bool:
        """Would the signal source b at m power, or connect to, a route cell at p next to it?
        A repeater only connects along its axis and only powers its front; an observer only through
        its back. Every other source (torch, lever, button, comparator, redstone block...) counts."""
        d = next(k for k, v in VEC6.items() if _add(p, v) == m)      # direction from p to the source
        if b.kind == "Repeater":
            return b.props["facing"] in (d, OPP[d])
        if b.kind == "Observer":
            return _add(m, VEC6[OPP[b.props["facing"]]]) == p
        return True

    def strongly_powers(self, b: _Info, m, n) -> bool:
        """Does the signal source b at m strongly power the conductor n next to it (so that dust
        next to n would get power from it)? Unknown sources are assumed to."""
        k = b.kind
        if k in ("Repeater", "Comparator"):
            return _add(m, DIRS[OPP[b.props["facing"]]]) == n          # its front
        if k in ("Torch", "WallTorch"):
            return n == _up(m)
        if k in ("Lever", "Button"):
            face = b.props.get("face")
            if face == "floor":
                return n == _down(m)
            if face == "ceiling":
                return n == _up(m)
            return n == _add(m, DIRS[OPP[b.props["facing"]]])          # the wall it hangs on
        if k == "Observer":
            return _add(m, VEC6[OPP[b.props["facing"]]]) == n          # its back
        if k == "PressurePlate":
            return n == _down(m)
        if k in ("PoweredBlock", "Wire"):
            return False
        return True

    def strong_neighbour(self, n, skip):
        """A signal source (other than the start source and `skip`) strongly powering conductor n."""
        for m in _n6(n):
            if m == skip or m == self.S0:
                continue
            bm = self.at(m)
            if bm.source and bm.kind != "Wire" and self.strongly_powers(bm, m, n):
                return m, bm
        return None

    def incoming_problem(self, p, first):
        """Nothing but the start source may feed (or connect to) a route cell at p (cached)."""
        key = ("in", p, first)
        if key in self._cache:
            return self._cache[key]
        r = None
        for n in _n6(p):
            if n == self.S0:
                if not first:
                    r = ("would pass next to the source again", f"{p}")
                    break
                continue
            if n == self.T:
                continue        # cells next to the target are terminal: only the last dust stands there
            b = self.at(n)
            if b.source and b.kind != "Wire" and self.touches(b, n, p):
                r = ("next to another signal source", f"{p} next to {b.name} at {n}")
                break
            if b.conducts:
                hit = self.strong_neighbour(n, p)
                if hit:
                    r = ("next to a block another signal source powers",
                         f"{p} next to {b.name} at {n}, powered by {hit[1].name} at {hit[0]}")
                    break
        self._cache[key] = r
        return r

    def support_problem(self, s, for_repeater):
        """(new, problem): can s hold dust (or a repeater), or can a support block be placed there? (cached)"""
        key = ("sup", s, for_repeater)
        if key in self._cache:
            return self._cache[key]
        b = self.at(s)
        res = (False, None)
        if not b.air:
            if b.kind == "Wire":
                res = (False, ("would stand on other dust", str(s)))
            elif s in self.piston_paths:
                res = (False, ("block below is in a piston's push path", f"{b.name} at {s}"))
            elif for_repeater and not b.rigid_top:
                res = (False, ("block below cannot hold a repeater", f"{b.name} at {s}"))
            elif not for_repeater and not b.dust_top:
                res = (False, ("block below cannot hold dust", f"{b.name} at {s}"))
        else:
            res = (True, None)
            if not self.in_box(s):
                res = (True, ("support would be outside the search box", None))
            elif s in self.banned:
                res = (True, ("excluded after a failed check", str(s)))
            elif s in self.watched:
                res = (True, ("an observer watches the support cell", str(s)))
            elif s in self.piston_paths:
                res = (True, ("support in a piston's push path", str(s)))
            else:
                for q in _n6(s):
                    if q in self.dust:
                        res = (True, ("support would change other dust", f"support {s} next to dust at {q}"))
                        break
                    if q[1] == s[1] and self.at(q).kind in _SHAPE_SENSITIVE:
                        res = (True, ("support would change a fence/wall/pane", f"support {s} next to {self.at(q).name} at {q}"))
                        break
                if res[1] is None and self.sup.conducts:
                    hit = self.strong_neighbour(s, None)
                    if hit:
                        res = (True, ("support would be powered by another signal source",
                                      f"support {s} next to {hit[1].name} at {hit[0]}"))
        self._cache[key] = res
        return res

    # ------------------------------------------------------------------ what the route powers

    def reacts(self, b: _Info, x, e, via) -> bool:
        """Does block b at x do something when e (a dust, or a conductor powered by dust) powers it?"""
        k = b.kind
        if k in _INERT:
            return False
        if k == "Wire":
            return via == "dust"           # dust ignores blocks powered by dust
        if k == "Torch":
            return via == "conductor" and e == _down(x)
        if k == "WallTorch":
            return via == "conductor" and e == _add(x, DIRS[OPP[b.props["facing"]]])
        if k == "Repeater":
            return e == _add(x, DIRS[b.props["facing"]])
        if k == "Comparator":
            f = b.props["facing"]
            if e == _add(x, DIRS[f]):
                return True
            return via == "dust" and e != _add(x, DIRS[OPP[f]])
        return True

    def block(self, x, ctx):
        return self.sup if x in ctx.supports else self.at(x)

    def qc_problem(self, x, ctx):
        """Power arriving at the block above x: would a piston/dispenser/dropper at x notice it?"""
        if x in ctx.cells or x in ctx.supports or x in self.intended:
            return None
        b = self.at(x)
        if b.kind in _QC:
            return ("would quasi-power a piston/dispenser/dropper", f"{b.name} at {x}")
        return None

    def via_conductor_problem(self, n, c, ctx):
        """n is next to a conductor c that the route powers."""
        if n in ctx.cells or n in self.intended:
            return None
        b = self.block(n, ctx)
        if not b.air and self.reacts(b, n, c, "conductor"):
            what = "the source" if n == self.S0 else b.name
            return ("would power a component through a block", f"{what} at {n} through {self.block(c, ctx).name} at {c}")
        return self.qc_problem(_down(n), ctx)

    def target_problem(self, ctx):
        """The target gets power. A component that conducts (lamp, note block...) passes it on."""
        tb = self.at(self.T)
        if self.T_relay:
            return None
        if tb.conducts:
            for n in _n6(self.T):
                r = self.via_conductor_problem(n, self.T, ctx)
                if r:
                    return ("powering the target would also power its neighbour", r[1])
        return self.qc_problem(_down(self.T), ctx)

    def emit_problem(self, w, e, ctx):
        """Dust at e sends power into w."""
        if w in ctx.cells:
            return None
        if w == self.T:
            return self.target_problem(ctx)
        if w in self.intended:
            return None
        if w == self.S0:
            return None if ctx.first else ("would power the source", str(w))
        b = self.block(w, ctx)
        if b.air:
            return self.qc_problem(_down(w), ctx)
        if b.kind == "Wire":
            return ("would power other dust", str(w))
        if self.reacts(b, w, e, "dust"):
            return ("would power a component", f"{b.name} at {w}")
        if b.conducts:
            if w in self.dust_out and w not in ctx.supports:
                return ("would share a powered block with other dust", f"{b.name} at {w}")
            for n in _n6(w):
                if n != e:
                    r = self.via_conductor_problem(n, w, ctx)
                    if r:
                        return r
        return self.qc_problem(_down(w), ctx)

    def outputs_problem(self, p, sides, ctx):
        for w in [_down(p)] + [_add(p, DIRS[d]) for d in sorted(sides)]:
            r = self.emit_problem(w, p, ctx)
            if r:
                return r
        return None

    # ------------------------------------------------------------------ search

    def first_cells(self):
        """Cells next to the source that it feeds, with the side the first dust connects to (or None)."""
        S0, b = self.S0, self.at(self.S0)
        k = b.kind
        if k in ("Repeater", "Comparator"):
            cands = [_add(S0, DIRS[OPP[b.props["facing"]]])]
        elif k == "Observer":
            cands = [_add(S0, VEC6[OPP[b.props["facing"]]])]
        elif k == "Torch":
            cands = [_add(S0, DIRS[d]) for d in HORIZ]
        elif k == "WallTorch":
            cands = [_add(S0, DIRS[d]) for d in HORIZ if d != OPP[b.props["facing"]]] + [_down(S0)]
        else:
            cands = [_add(S0, DIRS[d]) for d in HORIZ] + [_up(S0), _down(S0)]
        out = []
        for c in cands:
            d = _hdir(c, S0) if c[1] == S0[1] else None
            out.append((c, d if d and self.connectable(b, d) else None))
        return out

    def viable_end_cells(self):
        """End cells that pass every rule that does not depend on the path (fail fast otherwise)."""
        out = []
        for c in self.end_cells:
            r = self.cell_problem(c) or self.incoming_problem(c, c in _n6(self.S0))
            if r:
                self._no(*r)
                continue
            s = _down(c)
            new_sup, r = (False, None) if s == self.T else self.support_problem(s, False)
            if r:
                self._no(*r)
                continue
            if self.T is not None:
                r = self.target_problem(_Ctx(frozenset({c}), frozenset({s}) if new_sup else frozenset(), False))
                if r:
                    self._no(r[0] + " (pass allow=[...] if that is intended)", r[1])
                    continue
            out.append(c)
        return out

    def h(self, p):
        return min(max(abs(p[0] - c[0]) + abs(p[2] - c[2]), abs(p[1] - c[1])) for c in self.end_cells)

    def is_terminal(self, p):
        return self.T is not None and (p in _n6(self.T))

    def goal_shape(self, node):
        """Shape of the last dust and whether it powers the target (None if it cannot end here)."""
        p = node.pos
        if self.end_dust is not None:
            return _shape({node.conn}) if p == self.end_dust else None
        if p not in self.end_cells:
            return None
        conns = {node.conn}
        a = _hdir(p, self.T) if p[1] == self.T[1] else None
        if a and self.connectable(self.at(self.T), a):
            conns.add(a)
        sides = _shape(conns)
        if p == _up(self.T) or (a and a in sides):
            return sides
        self._no("last dust would not point into the target", f"{p}")
        return None

    def _new_dust(self, node, prev_pos, n, dy, first_out_sides, extra_g, rep=None):
        """Try to append dust at n after the element at prev_pos (node's dust, or a repeater)."""
        cells, supports, clear = node.cells, node.supports, node.clear
        if rep:
            cells = cells | {rep[0]}
            if rep[2]:
                supports = supports | {rep[2]}
            clear = clear | {_up(rep[0])}
        if n in cells or n in supports or n in clear:
            return self._no("route would run into itself")
        r = self.cell_problem(n) or self.incoming_problem(n, False)
        if r:
            return self._no(*r)
        for q in _n12(n):
            if q in cells and q != prev_pos:
                return self._no("route would run next to itself", f"{n} next to {q}")
        s = _down(n)
        if s in cells:
            return self._no("route would run into itself")
        new_sup, r = self.support_problem(s, False)
        if r:
            return self._no(*r)
        if new_sup and s in clear:
            return self._no("support would sit right above route dust", str(s))
        if _up(n) in supports:
            return self._no("route would run under its own support", str(n))
        cells2 = cells | {n}
        sup2 = supports | {s} if new_sup else supports
        # the previous dust's shape is now known: check everything it powers
        ctx = _Ctx(cells2, sup2, node.first)
        r = self.outputs_problem(node.pos, first_out_sides, ctx)
        if r:
            return self._no(*r)
        power = 15 if rep else node.power - 1
        if power < 1:
            return self._no("signal would die out (no repeater possible here)")
        g = node.g + 1 + extra_g + (_SUPPORT_COST if new_sup else 0) + (_STEP_COST if dy else 0)
        return _Node(n, _hdir(n, prev_pos), power, g, node, cells2, sup2, clear | {_up(n)}, False,
                     self.is_terminal(n), s if new_sup else None, rep, dy)

    def expand(self, node):
        P = node.pos
        out = []
        for d in HORIZ:
            v = DIRS[d]
            side = _add(P, v)
            sides = _shape({node.conn, d})
            for dy in (0, 1, -1):
                n = (side[0], side[1] + dy, side[2])
                if dy == 1:
                    a = _up(P)       # above the lower dust: must not conduct, or power cannot climb
                    if self.at(a).conducts or a in node.supports:
                        self._no("step up blocked: block above the dust conducts", str(a))
                        continue
                elif dy == -1:
                    s = _down(P)     # the upper dust's block must conduct, the cell beside it must not
                    if not self.block(s, _Ctx((), node.supports, False)).conducts:
                        self._no("step down impossible: block under the dust does not conduct", str(s))
                        continue
                    if self.at(side).conducts or side in node.supports or side in node.cells:
                        self._no("step down blocked: block above the lower dust conducts", str(side))
                        continue
                nn = self._new_dust(node, P, n, dy, sides, 0.0)
                if isinstance(nn, _Node):
                    out.append(nn)
            # a repeater only where the signal runs low (keeps the search small, puts repeaters late)
            if self.allow_repeaters and (node.power <= 6 or node.power - 1 - self.h(side) < 1):
                rp = side
                n = _add(rp, v)
                nn = self._repeater(node, rp, d, n, sides)
                if isinstance(nn, _Node):
                    out.append(nn)
        return out

    def _repeater(self, node, rp, d, n, sides):
        if rp in node.cells or rp in node.supports or rp in node.clear:
            return self._no("route would run into itself")
        r = self.cell_problem(rp) or self.incoming_problem(rp, False)
        if r:
            return self._no(*r)
        if self.T is not None and rp in _n6(self.T):
            return self._no("repeater would stand next to the target", str(rp))
        for q in _n12(rp):
            if q in node.cells and q != node.pos:
                return self._no("route would run next to itself", f"{rp} next to {q}")
        s = _down(rp)
        if s in node.cells:
            return self._no("route would run into itself")
        new_sup, r = self.support_problem(s, True)
        if r:
            return self._no(*r)
        if new_sup and s in node.clear:
            return self._no("support would sit right above route dust", str(s))
        if _up(rp) in node.supports:
            return self._no("route would run under its own support", str(rp))
        return self._new_dust(node, rp, n, 0, sides, _REPEATER_COST + (_SUPPORT_COST if new_sup else 0),
                              rep=(rp, d, s if new_sup else None))

    def search(self, max_cells):
        tick = itertools.count()
        heap = []
        best = {}

        def push(nd):
            key = (nd.pos, nd.conn, nd.first)
            lst = best.setdefault(key, [])
            for g2, p2 in lst:
                if g2 <= nd.g + 1e-9 and p2 >= nd.power:
                    return
            lst.append((nd.g, nd.power))
            heapq.heappush(heap, (nd.g + self.h(nd.pos), self.h(nd.pos), next(tick), nd))

        self.end_cells = self.viable_end_cells()
        if not self.end_cells:
            what = f"the target {self.T}" if self.T else f"the end cell {self.end_dust}"
            raise RouteError(f"no usable cell for the last dust at {what}", self.reasons, self.examples)
        for c, conn in self.first_cells():
            r = self.cell_problem(c) or self.incoming_problem(c, True)
            if r:
                self._no(*r)
                continue
            s = _down(c)
            if s == self.S0:
                new_sup, r = False, (None if self.at(s).dust_top else ("block below cannot hold dust", str(s)))
            else:
                new_sup, r = self.support_problem(s, False)
            if r:
                self._no(*r)
                continue
            nd = _Node(c, conn, self.source_power, _SUPPORT_COST if new_sup else 0.0, None, frozenset({c}),
                       frozenset({s}) if new_sup else frozenset(), frozenset({_up(c)}), True,
                       self.is_terminal(c), s if new_sup else None, None, 0)
            push(nd)
        if not heap:
            raise RouteError(f"no free cell next to the source {self.S0} that it can feed", self.reasons, self.examples)
        expanded = 0
        while heap:
            _, _, _, nd = heapq.heappop(heap)
            sides = self.goal_shape(nd)
            if sides is not None:
                r = self.outputs_problem(nd.pos, sides, _Ctx(nd.cells, nd.supports, nd.first))
                if r is None:
                    return self._plan(nd, sides)
                self._no(*r)
            if nd.terminal:
                continue
            expanded += 1
            if expanded > max_cells:
                raise RouteError(f"gave up after examining {max_cells} cells (raise max_cells or give a box)",
                                 self.reasons, self.examples)
            for ch in self.expand(nd):
                push(ch)
        raise RouteError(f"no route from {self.S0} to {self.T or self.end_dust} inside box {self.box}",
                         self.reasons, self.examples)

    def _plan(self, last, last_sides):
        nodes = []
        nd = last
        while nd is not None:
            nodes.append(nd)
            nd = nd.parent
        nodes.reverse()
        elems = []            # dicts in signal order
        for nd in nodes:
            if nd.rep:
                rp, d, rs = nd.rep
                elems.append({"pos": rp, "kind": "repeater", "toward": d, "support": rs})
            elems.append({"pos": nd.pos, "kind": "dust", "power": nd.power, "support": nd.sup, "conn": nd.conn,
                          "dy": nd.dy})
        # final shapes: each dust connects to its neighbours in the route (+ source / target)
        for i, el in enumerate(elems):
            if el["kind"] != "dust":
                continue
            conns = set()
            if i == 0:
                conns.add(el["conn"])
            else:
                conns.add(_hdir(el["pos"], elems[i - 1]["pos"]))
            if i + 1 < len(elems):
                conns.add(_hdir(el["pos"], elems[i + 1]["pos"]))
            el["sides"] = last_sides if i == len(elems) - 1 else _shape(conns)
        return elems

    # ------------------------------------------------------------------ full recheck of a finished plan

    def recheck(self, elems):
        """All rules again with the complete route known (later supports can matter to earlier dust)."""
        cells = frozenset(e["pos"] for e in elems)
        supports = frozenset(e["support"] for e in elems if e["support"])
        problems = []
        for i, el in enumerate(elems):
            p = el["pos"]
            first = i == 0
            r = self.cell_problem(p) or self.incoming_problem(p, first)
            if r:
                problems.append((r, [p]))
            nbrs = {elems[j]["pos"] for j in (i - 1, i + 1) if 0 <= j < len(elems)}
            for q in _n12(p):
                if q in cells and q not in nbrs:
                    problems.append((("route runs next to itself", f"{p} next to {q}"), [p, q]))
            if _up(p) in supports and el["kind"] == "dust":
                problems.append((("own support right above route dust", str(p)), [p]))
            if el["kind"] == "dust":
                r = self.outputs_problem(p, el["sides"], _Ctx(cells, supports, first))
                if r:
                    problems.append((r, [p]))
        return problems


# ---------------------------------------------------------------------------------------- placing

def _place(world, elems, support_sid, delay):
    """Write the route raw (no updates) into `world`. Returns [(pos, sid)] in order."""
    w = world._w
    placed = []
    for el in elems:
        if el["support"]:
            w.place(*el["support"], support_sid)
            placed.append((el["support"], support_sid))
    cross = _sid("redstone_wire[north=side,east=side,south=side,west=side]")
    for el in elems:
        if el["kind"] == "dust":
            w.place(*el["pos"], cross)
        else:
            sid = _sid(f"repeater[facing={OPP[el['toward']]},delay={delay},locked=false,powered=false]")
            w.place(*el["pos"], sid)
            placed.append((el["pos"], sid))
    for el in elems:
        if el["kind"] == "dust":
            sid = w.dust_placement_state(*el["pos"])
            w.place(*el["pos"], sid)
            placed.append((el["pos"], sid))
    return placed


def _sides_of(sid):
    props = _info(sid).props
    return frozenset(d for d in HORIZ if props.get(d, "none") != "none")


# ---------------------------------------------------------------------------------------- verification

def _net_sig(n):
    return (tuple(n.dust), tuple(n.powers), tuple(n.sources), tuple(n.one_way))


def _check_nets(pl, before, after, elems):
    """(route nets, checks, problems) from World.nets() before and after laying the route."""
    mine = {e["pos"] for e in elems if e["kind"] == "dust"}
    reps = {e["pos"] for e in elems if e["kind"] == "repeater"}
    sups = {e["support"] for e in elems if e["support"]}
    problems, my_nets, beside = [], [], set()
    for n in after:
        ds = set(n.dust)
        if ds & mine:
            my_nets.append(n)
            if not ds <= mine:
                other = sorted(ds - mine)
                problems.append((("route dust joined an existing network", f"with dust at {other[:3]}"), other))
            for pos, name in n.powers:
                if pos in sups or pos in reps or pos == pl.S0 or pos in pl.intended:
                    continue
                b = pl.at(pos)
                if b.kind not in _INERT:
                    problems.append((("route network powers a component", f"{name} at {pos}"), [pos]))
            for pos, name in n.sources:
                if pos in reps or pos == pl.S0 or pos == pl.T:
                    continue
                # nets lists every source next to the dust; only one that powers or connects to it matters
                b = pl.at(pos)
                if any(q in mine and pl.touches(b, pos, q) for q in _n6(pos)):
                    problems.append((("route network touches another signal source", f"{name} at {pos}"), [pos]))
                else:
                    beside.add(pos)
    old = {tuple(n.dust): _net_sig(n) for n in before}
    new = {tuple(n.dust): _net_sig(n) for n in after if not set(n.dust) & mine}
    for k, sig in old.items():
        if new.get(k) != sig:
            problems.append((("an existing dust network changed", f"network with dust {list(k)[:3]}"), list(k)))
    checks = []
    if not problems:
        checks.append(f"nets: route forms {len(my_nets)} network(s) of its own; the {len(old)} existing "
                      f"network(s) are unchanged"
                      + (f"; {len(beside)} signal source(s) next to it neither feed nor connect to it "
                         f"(sideways repeaters / observers)" if beside else ""))
    return my_nets, checks, problems


def _check_foreign_shapes(world, scratch, placed_pos):
    """Would placing the route change the shape of any pre-existing neighbour (fence, wall, dust...)?"""
    near = set()
    for p in placed_pos:
        for q in _n6(p):
            if q not in placed_pos:
                near.add(q)
    a, b = world.copy(), scratch.copy()
    problems = []
    for q in sorted(near):
        if _info(world._w.get(*q)).air:
            continue
        a._w.reshape(*q)
        b._w.reshape(*q)
        if a._w.get(*q) != b._w.get(*q):
            problems.append((("placing the route would change a neighbouring block's shape",
                              f"{_info(world._w.get(*q)).name} at {q}"), [q]))
    return problems


def _stimulus(pl):
    """(prepare, turn_on, description): inputs that switch the source off (prepare, then settle; None if
    it is already off) and on. Levers and buttons are used as they are; any other source is replaced by
    glass (off) and then by a redstone block (on), the same way in both simulated worlds."""
    S0, b = pl.S0, pl.at(pl.S0)
    if b.kind == "Lever":
        on_now = b.props.get("powered") == "true"
        prep = (lambda w: w.use(S0)) if on_now else None
        return prep, (lambda w: w.use(S0)), "flip the lever at %s on" % (S0,)
    if b.kind == "Button":
        prep = (lambda w: None) if b.props.get("powered") == "true" else None   # wait for it to pop out
        return prep, (lambda w: w.use(S0)), "press the button at %s" % (S0,)
    return ((lambda w: w.setblock(S0, "glass")), (lambda w: w.setblock(S0, "redstone_block")),
            "replace the source at %s by a redstone block" % (S0,))


def _histories(tl):
    h = {}
    for c in tl.changes:
        h.setdefault(c.pos, []).append((c.tick, c.new.id))
    return h


def _caused_by(events, eid, roots):
    """Does the cause chain of trace event eid go through one of `roots`? An event located at a root
    (including a neighbour/shape update it received) means what follows is that block's own doing,
    e.g. an observer watching the target fires because the target changed."""
    seen = 0
    cur = events[eid]
    while cur is not None and seen < 100000:
        seen += 1
        pos = (cur["pos"]["x"], cur["pos"]["y"], cur["pos"]["z"])
        if pos in roots:
            return True
        c = cur["cause"]
        if c is None:
            if cur["kind"]["type"] == "block_entity_tick":
                # a moving block: continue from whatever created the moving_piston at that spot
                prev = None
                for e in reversed(events[:cur["id"]]):
                    if (e["pos"] == cur["pos"] and e["kind"]["type"] == "set_block"
                            and _info(e["kind"]["new"]).name == "moving_piston"):
                        prev = e
                        break
                cur = prev
                continue
            return False
        cur = events[c]
    return False


def _simulate(world, scratch, pl, elems, delay_ticks, ticks):
    """Differential simulation. Returns (checks, notes, problems, end_power)."""
    own = {e["pos"] for e in elems} | {e["support"] for e in elems if e["support"]}
    dust = [e["pos"] for e in elems if e["kind"] == "dust"]
    reps = [e["pos"] for e in elems if e["kind"] == "repeater"]
    last = dust[-1]
    prep, on, how = _stimulus(pl)
    base, test = world.copy(), scratch.copy()
    settle = 0
    if prep:
        prep(base)
        prep(test)
        settle = base.run_until_idle(400)
        test.tick(settle)
    test._w.enable_trace()
    test._trace = True
    probes = [last] + ([pl.T] if pl.T else [])
    tb = base.record(ticks, inputs={0: on}, probes=probes)
    tt = test.record(ticks, inputs={0: on}, probes=probes)
    checks, notes, problems = [], [], []
    end_power = max(tt.probes[last].power)
    if pl.T:
        powered = tt.probes[pl.T].powered
        first_on = next((t for t, v in enumerate(powered) if v), None)
        if first_on is None:
            problems.append((("simulation: the target never got power", f"last dust power {end_power}"), [last]))
        elif any(tb.probes[pl.T].powered):
            notes.append(f"the target {pl.T} also gets power without the route (from other circuitry)")
        if first_on is not None:
            checks.append(f"simulation ({how}): target {pl.T} powered from tick {first_on} on, last dust at power {end_power}")
    else:
        if end_power < 1:
            problems.append((("simulation: the last dust never got power", str(last)), [last]))
        else:
            checks.append(f"simulation ({how}): last dust {last} reaches power {end_power}")
    hb, ht = _histories(tb), _histories(tt)
    events = None
    unintended = []
    for pos in sorted(set(hb) | set(ht)):
        if pos in own or hb.get(pos) == ht.get(pos):
            continue
        if pos in pl.intended:
            continue
        if pos not in ht:
            unintended.append(pos)
            continue
        if events is None:
            events = json.loads(test._w.trace_json())
        ids = [e["id"] for e in events if e["kind"]["type"] == "set_block"
               and (e["pos"]["x"], e["pos"]["y"], e["pos"]["z"]) == pos]
        if ids and all(_caused_by(events, i, pl.intended) for i in ids):
            continue
        unintended.append(pos)
    for pos in unintended:
        before = [(t, _info(s).name) for t, s in hb.get(pos, [])][:4]
        after = [(t, _redsim.format_state(s).replace("minecraft:", "")) for t, s in ht.get(pos, [])][:4]
        why = test.explain(pos).splitlines()
        problems.append((("simulation: a pre-existing block behaves differently with the route",
                          f"{pos}: without route {before}, with route {after}; cause: {' <- '.join(l.strip() for l in why[-3:])}"),
                         [pos]))
    if not unintended:
        checks.append(f"simulation: no pre-existing block behaved differently over {ticks} ticks "
                      f"(compared with the same input and no route)")
    # bleed-in: with the source off nothing may power the route
    off = scratch.copy()
    if prep:
        prep(off)
        off.tick(settle)
    for p in [e["pos"] for e in elems]:
        off._w.update_block(*p)
    off.tick(delay_ticks + 6)
    hot = [p for p in dust if off.power(p) > 0] + [p for p in reps if off[p].get("powered") == "true"]
    if hot:
        problems.append((("route gets power with the source off (something else feeds it)", f"at {hot[:3]}"), hot))
    else:
        checks.append("source off: the route stays unpowered (nothing else feeds it)")
    return checks, notes, problems, end_power


# ---------------------------------------------------------------------------------------- API

def route(world, start, end, *, support="stone", max_cells: int = 50_000, box=None, allow_repeaters: bool = True,
          repeater_delay: int = 1, source_power: int = 15, allow=(), verify: bool = True, update: bool = True,
          retries: int = 3, margin: int = 12, verify_ticks: int | None = None) -> RouteResult:
    """Wire a signal from the source block `start` to the block `end` with dust that touches nothing else.

    start         the source block (lever, button, torch, repeater, comparator, observer, redstone block,
                  or a conductor that is strongly powered); the first dust goes next to it, where it feeds.
    end           the block to power (lamp, piston, door, repeater input...; dust ends on top of it or next
                  to it pointing into it), or an air cell = exactly where the last dust goes.
    support       block placed under route cells that have nothing to stand on. A conductor (default stone)
                  allows steps down; a transparent block (glass) only steps up.
    max_cells     search budget: number of route positions examined before giving up.
    box           (corner, corner): all placed blocks stay inside it. Default: start/end bounding box
                  grown by `margin` sideways and 4 up/down.
    allow_repeaters  insert repeaters (delay `repeater_delay`) where the signal would drop to 0.
    source_power  power the source gives the first dust (15 for levers, torches, repeaters, blocks).
    allow         extra positions that may be powered/changed by the route (e.g. a second lamp meant to
                  light together with the target).
    verify        prove separation with World.nets() and a differential simulation before writing.
    update        after writing, give each new dust/repeater a neighbour update, so a route whose source is
                  already on powers up like hand-placed dust (repeaters then need `tick`s). With update=False
                  the route is written raw at power 0, like `World.wire`.
    retries       when verification fails, exclude the cells near the problem and search again this often.
    margin        how far the default box reaches beyond start and end (blocks, sideways).
    verify_ticks  ticks to simulate after switching the source on (default: repeater delay + 20).

    Returns a RouteResult (path, placed blocks, repeaters, delay_ticks, end_power, report, undo()).
    Raises RouteError (world unchanged) when no safe route exists or verification keeps failing, and
    ValueError for unusable arguments.
    """
    S0, E = _p(start), _p(end)
    allow = {_p(a) for a in allow}
    if not 1 <= repeater_delay <= 4:
        raise ValueError("repeater_delay must be 1..4")
    before_nets = world.nets() if verify else None
    banned: set = set()
    failures = []
    reasons, examples = Counter(), {}
    for attempt in range(retries + 1):
        pl = _Planner(world, S0, E, support, box, allow_repeaters, repeater_delay, source_power, allow, banned, margin)
        try:
            elems = pl.search(max_cells)
        except RouteError as e:
            reasons.update(e.reasons)
            for k, v in e.examples.items():
                examples.setdefault(k, v)
            msg = str(e).splitlines()[0]
            if failures:
                msg += "\nearlier attempts failed verification:\n" + "\n".join("  " + f for f in failures)
            raise RouteError(msg, reasons, examples) from None
        reasons.update(pl.reasons)
        for k, v in pl.examples.items():
            examples.setdefault(k, v)
        problems = pl.recheck(elems)
        scratch = placed = None
        my_nets, checks, notes, end_power = [], [], [], elems[-1]["power"]
        n_rep = sum(1 for e in elems if e["kind"] == "repeater")
        delay_ticks = 2 * repeater_delay * n_rep
        if not problems:
            scratch = world.copy()
            placed = _place(scratch, elems, pl.sup.sid, repeater_delay)
            placed_pos = {p for p, _ in placed}
            for el in elems:
                if el["kind"] == "dust":
                    got = _sides_of(scratch._w.get(*el["pos"]))
                    if got != el["sides"]:
                        problems.append((("dust shape differs from the plan",
                                          f"{el['pos']}: planned {sorted(el['sides'])}, game gives {sorted(got)}"),
                                         [el["pos"]]))
            problems += _check_foreign_shapes(world, scratch, placed_pos)
            if not problems:
                checks.append(f"static rules: {len(elems)} route cells checked against all neighbours")
                checks.append("shapes: every dust got the planned shape; no neighbouring block changes shape")
            if verify and not problems:
                my_nets, c2, p2 = _check_nets(pl, before_nets, scratch.nets(), elems)
                checks += c2
                problems += p2
            if verify and not problems:
                ticks = verify_ticks or (delay_ticks + 20)
                c3, n3, p3, end_power = _simulate(world, scratch, pl, elems, delay_ticks, ticks)
                checks += c3
                notes += n3
                problems += p3
        if not problems:
            break
        added = False
        for (reason, detail), culprits in problems:
            failures.append(f"attempt {attempt + 1}: {reason}" + (f" ({detail})" if detail else ""))
            for c in culprits:
                for el in elems:
                    for q in (el["pos"], el["support"]):
                        if q and q not in banned and max(abs(q[0] - c[0]), abs(q[1] - c[1]), abs(q[2] - c[2])) <= 2:
                            banned.add(q)
                            added = True
        if not added:
            break
    if problems:
        raise RouteError("no route passed verification:\n" + "\n".join("  " + f for f in failures), reasons, examples)

    # ---- write it for real
    previous = {}
    for pos, sid in placed:
        previous.setdefault(pos, _redsim.format_state(world._w.get(*pos)))
        world._w.place(*pos, sid)
    if update:
        for el in elems:
            world._w.update_block(*el["pos"])
    if not verify:
        my_nets = [n for n in world.nets() if set(n.dust) & {e["pos"] for e in elems}]
    dust = [(e["pos"], e["power"]) for e in elems if e["kind"] == "dust"]
    reps = [(e["pos"], e["toward"]) for e in elems if e["kind"] == "repeater"]
    sups = [e["support"] for e in elems if e["support"]]
    if failures:
        notes.append(f"{len(failures)} earlier candidate(s) failed verification and were avoided: " + "; ".join(failures[:3]))
    if source_power != 15 and verify:
        notes.append("verification powers the route with 15; planning used source_power=%d" % source_power)
    res = RouteResult(source=S0, target=pl.T, path=[e["pos"] for e in elems], dust=dust, repeaters=reps,
                      supports=sups, placed=[(p, _redsim.format_state(s).replace("minecraft:", "")) for p, s in placed],
                      previous=previous, delay_ticks=delay_ticks, end_power=end_power, nets=my_nets,
                      checks=checks, notes=notes)
    res.report = _report(pl, res, elems)
    return res


def _report(pl, res, elems):
    src = pl.at(pl.S0).name
    dst = f"{pl.at(pl.T).name} at {pl.T}" if pl.T else f"dust at {pl.end_dust}"
    ups = sum(1 for e in elems if e.get("dy") == 1)
    downs = sum(1 for e in elems if e.get("dy") == -1)
    lines = [f"route: {src} at {pl.S0} -> {dst}",
             f"  {len(res.dust)} dust, {len(res.repeaters)} repeater(s), {len(res.supports)} support block(s) placed, "
             f"{ups} step(s) up, {downs} step(s) down; delay {res.delay_ticks} game ticks; "
             f"last dust power {res.end_power}"]
    seg = []
    for e in elems:
        if e["kind"] == "dust":
            seg.append(f"{e['pos']}:{e['power']}")
        else:
            seg.append(f"[repeater {e['pos']} -> {e['toward']}]")
    lines.append("  path: " + " ".join(seg))
    for c in res.checks:
        lines.append("  ok  " + c)
    for n in res.notes:
        lines.append("  note " + n)
    return "\n".join(lines)
