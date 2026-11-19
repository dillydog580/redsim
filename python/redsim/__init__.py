"""redsim: a tick-exact, headless simulator of Minecraft Java Edition 26.3 redstone.

    import redsim as rs

    w = rs.World()
    w.fill((0, 0, 0), (4, 0, 0), "stone")
    w.set((0, 1, 0), "lever[face=floor,facing=east]")
    w.wire((1, 1, 0), (3, 1, 0))
    w.set((4, 1, 0), "redstone_lamp")
    w.use((0, 1, 0))              # flip the lever (between ticks, like a player)
    w.tick(2)
    w["4 1 0"]                    # Block('redstone_lamp[lit=true]')

Everything happens in a deterministic world that follows the game's own update
order. Coordinates are absolute world coordinates: redstone dust and pistons
behave differently at different positions in the real game, and so here.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Iterable

from . import _redsim
from .schematic import Build, load as load_schematic

__all__ = ["World", "Block", "Timeline", "Build", "load", "load_schematic", "MC_VERSION", "__version__"]

MC_VERSION = _redsim.mc_version()

try:
    from importlib.metadata import version as _dist_version

    __version__ = _dist_version("redsim")
except Exception:  # running from a source tree that was never installed
    __version__ = "0+unknown"

DIRS = {"north": (0, 0, -1), "south": (0, 0, 1), "west": (-1, 0, 0), "east": (1, 0, 0), "up": (0, 1, 0), "down": (0, -1, 0)}
OPPOSITE = {"north": "south", "south": "north", "west": "east", "east": "west", "up": "down", "down": "up"}


def _pos(p) -> tuple[int, int, int]:
    if isinstance(p, str):
        p = p.replace(",", " ").split()
    x, y, z = p
    return int(x), int(y), int(z)


def _sid(block) -> int:
    if isinstance(block, Block):
        return block.id
    if isinstance(block, int):
        return block
    text = str(block).strip()
    if ":" not in text.split("[")[0]:
        text = "minecraft:" + text
    return _redsim.parse_state(text)


class Block:
    """An immutable block state. Compare with strings: `w[p] == "redstone_lamp[lit=true]"`
    matches when the name matches and every listed property matches (others ignored)."""

    __slots__ = ("id",)

    def __init__(self, sid: int):
        self.id = sid

    @property
    def state(self) -> str:
        return _redsim.format_state(self.id)

    @property
    def name(self) -> str:
        return _redsim.block_name(self.id).split(":", 1)[1]

    @property
    def props(self) -> dict:
        s = self.state
        if "[" not in s:
            return {}
        return dict(kv.split("=", 1) for kv in s[s.index("[") + 1:-1].split(","))

    def __getitem__(self, key):
        return self.props[key]

    def get(self, key, default=None):
        return self.props.get(key, default)

    @property
    def kind(self) -> str:
        return _redsim.block_kind(self.id)

    @property
    def conducts(self) -> bool:
        """Is this a redstone conductor (a block that can be powered and pass power on)?"""
        return bool(_redsim.state_info(self.id)[0] & 1)

    @property
    def is_signal_source(self) -> bool:
        """Does this block emit redstone power itself (dust connects to signal sources)?"""
        return bool(_redsim.state_info(self.id)[0] & 2)

    def matches(self, pattern: str) -> bool:
        pattern = pattern.strip()
        name, _, rest = pattern.partition("[")
        name = name if ":" in name else "minecraft:" + name
        if _redsim.block_name(self.id) != name:
            return False
        if rest:
            props = self.props
            for kv in rest.rstrip("]").split(","):
                if kv.strip():
                    k, v = kv.split("=", 1)
                    if props.get(k.strip()) != v.strip():
                        return False
        return True

    def __eq__(self, other):
        if isinstance(other, Block):
            return self.id == other.id
        if isinstance(other, str):
            return self.matches(other)
        return NotImplemented

    def __hash__(self):
        return hash(self.id)

    def __repr__(self):
        return f"Block({self.state.replace('minecraft:', '')!r})"

    def __str__(self):
        return self.state


@dataclass(frozen=True)
class Change:
    tick: int          # game tick (relative to the start of the recording)
    phase: str         # "input" | "scheduled_ticks" | "block_events" | "block_entities"
    pos: tuple
    old: Block
    new: Block

    def __str__(self):
        return f"t={self.tick} [{self.phase}] {self.pos}: {self.old.state.replace('minecraft:', '')} -> {self.new.state.replace('minecraft:', '')}"


_PHASES = {0: "input", 1: "scheduled_ticks", 2: "block_events", 3: "block_entities"}


@dataclass
class Probe:
    """Per-tick signal at one position. power[t]: dust level or strongest signal received;
    powered[t]: would a lamp/piston/door there be powered (hasNeighborSignal)."""
    pos: tuple
    power: list
    powered: list

    def pulses(self) -> list[tuple[int, int | None]]:
        """(start_tick, length) of each interval where powered is true."""
        out, on = [], None
        for t, v in enumerate(self.powered):
            if v and on is None:
                on = t
            elif not v and on is not None:
                out.append((on, t - on))
                on = None
        if on is not None:
            out.append((on, None))
        return out


class Timeline:
    """Every block write during a recording, in the exact order they happened."""

    def __init__(self, changes: list[Change], start_tick: int, end_tick: int, probes: dict | None = None):
        self.changes = changes
        self.start = start_tick
        self.end = end_tick
        #: pos -> Probe with .power[t] and .powered[t]; index t = after tick t (0 = right after tick-0 inputs)
        self.probes = probes or {}

    def __len__(self):
        return len(self.changes)

    def __iter__(self):
        return iter(self.changes)

    def at(self, pos) -> list[Change]:
        p = _pos(pos)
        return [c for c in self.changes if c.pos == p]

    def history(self, pos) -> list[tuple[int, Block]]:
        """(tick, state) after the last write in each tick that changed `pos`."""
        out = {}
        for c in self.at(pos):
            out[c.tick] = c.new
        return sorted(out.items())

    def first(self, pos, pattern: str) -> int | None:
        """First tick at which `pos` becomes a state matching `pattern`."""
        for c in self.at(pos):
            if c.new.matches(pattern):
                return c.tick
        return None

    def flickers(self) -> list[tuple[tuple, int]]:
        """(pos, tick) where a block changed and changed back within the same tick (0-tick pulses)."""
        seen = {}
        out = []
        for c in self.changes:
            key = (c.pos, c.tick)
            if key in seen and seen[key].old == c.new:
                out.append(key)
            seen.setdefault(key, c)
        return out

    def pulses(self, pos, prop: str = "powered") -> list[tuple[int, int]]:
        """(start_tick, length_in_ticks) of each interval where `prop` was true at `pos`."""
        on_since = None
        out = []
        for t, st in self.history(pos):
            v = st.get(prop)
            on = v not in (None, "false", "0", "none")
            if on and on_since is None:
                on_since = t
            elif not on and on_since is not None:
                out.append((on_since, t - on_since))
                on_since = None
        if on_since is not None:
            out.append((on_since, None))
        return out

    def summary(self, limit: int = 200) -> str:
        lines = [str(c) for c in self.changes[:limit]]
        if len(self.changes) > limit:
            lines.append(f"... {len(self.changes) - limit} more changes")
        return "\n".join(lines) if lines else "(no changes)"

    def to_json(self) -> str:
        return json.dumps([{"tick": c.tick, "phase": c.phase, "pos": list(c.pos), "old": c.old.state, "new": c.new.state}
                           for c in self.changes])


class World:
    """A simulated Minecraft 26.3 world (overworld height -64..319, no entities, no fluids).

    Inputs (`use`, `press`, `setblock`, ...) happen *between* game ticks, exactly
    like player actions and commands in the real game.
    """

    def __init__(self, trace: bool = False):
        self._w = _redsim.World()
        if trace:
            self._w.enable_trace()
        self._trace = trace

    # ------------------------------------------------------------------ building

    def set(self, pos, block, **props) -> "World":
        """Put a block exactly as given, with no block updates (like pasting a schematic).
        Unlisted properties take the block's defaults. Extra keyword args set properties."""
        sid = _sid(block)
        if props:
            sid = _sid(Block(sid).state.split("[")[0] + "[" + ",".join(
                [f"{k}={v}" for k, v in {**Block(sid).props, **{k: str(v).lower() for k, v in props.items()}}.items()]) + "]")
        self._w.place(*_pos(pos), sid)
        return self

    def __setitem__(self, pos, block):
        self.set(pos, block)

    def fill(self, a, b, block) -> "World":
        """Raw-fill the box between corners a and b (inclusive)."""
        (x0, y0, z0), (x1, y1, z1) = _pos(a), _pos(b)
        sid = _sid(block)
        for y in range(min(y0, y1), max(y0, y1) + 1):
            for z in range(min(z0, z1), max(z0, z1) + 1):
                for x in range(min(x0, x1), max(x0, x1) + 1):
                    self._w.place(x, y, z, sid)
        return self

    def wire(self, *points, support: str | None = None) -> "World":
        """Lay redstone dust along straight segments between points (inclusive), then give each
        dust piece the connection shape the game would compute. `support` places that block under it."""
        pts = [_pos(p) for p in points]
        cells = []
        for a, b in zip(pts, pts[1:] if len(pts) > 1 else pts):
            n = max(abs(b[0] - a[0]), abs(b[1] - a[1]), abs(b[2] - a[2]))
            for i in range(n + 1):
                t = i / n if n else 0
                cells.append(tuple(round(a[k] + (b[k] - a[k]) * t) for k in range(3)))
        cross = _sid("redstone_wire[north=side,east=side,south=side,west=side]")
        for c in cells:
            if support:
                self._w.place(c[0], c[1] - 1, c[2], _sid(support))
            self._w.place(*c, cross)
        # the shape a player would end up with: a lone dust is a cross, lines are lines
        for c in cells:
            self._w.place(*c, self._w.dust_placement_state(*c))
        return self

    # ---- component helpers with unambiguous directions --------------------------------

    def repeater(self, pos, toward: str, delay: int = 1, locked: bool = False) -> "World":
        """Repeater whose signal flows `toward` that direction (its `facing` is the opposite)."""
        return self.set(pos, f"repeater[facing={OPPOSITE[toward]},delay={delay},locked={str(locked).lower()}]")

    def comparator(self, pos, toward: str, mode: str = "compare") -> "World":
        """Comparator whose output goes `toward` that direction; input from the opposite side."""
        return self.set(pos, f"comparator[facing={OPPOSITE[toward]},mode={mode}]")

    def observer(self, pos, watches: str) -> "World":
        """Observer watching the block in direction `watches`; it outputs from the opposite face."""
        return self.set(pos, f"observer[facing={watches}]")

    def piston(self, pos, facing: str, sticky: bool = False) -> "World":
        return self.set(pos, f"{'sticky_piston' if sticky else 'piston'}[facing={facing}]")

    def torch(self, pos, attached: str = "down", lit: bool = True) -> "World":
        """Redstone torch standing on the block below (attached='down') or on a wall
        (attached = direction of the block it hangs on)."""
        if attached == "down":
            return self.set(pos, f"redstone_torch[lit={str(lit).lower()}]")
        return self.set(pos, f"redstone_wall_torch[facing={OPPOSITE[attached]},lit={str(lit).lower()}]")

    def _attached(self, pos, block, attached, facing):
        if attached == "down":
            return self.set(pos, f"{block}[face=floor,facing={facing}]")
        if attached == "up":
            return self.set(pos, f"{block}[face=ceiling,facing={facing}]")
        return self.set(pos, f"{block}[face=wall,facing={OPPOSITE[attached]}]")

    def lever(self, pos, attached: str = "down", facing: str = "north") -> "World":
        """Lever on the block in direction `attached` ('down' = floor, 'up' = ceiling, or a side)."""
        return self._attached(pos, "lever", attached, facing)

    def button(self, pos, attached: str = "down", wood: bool = False, facing: str = "north") -> "World":
        return self._attached(pos, "oak_button" if wood else "stone_button", attached, facing)

    def door(self, pos, facing: str = "north", kind: str = "iron_door", hinge: str = "left") -> "World":
        """Both halves of a door, lower half at pos."""
        x, y, z = _pos(pos)
        self.set((x, y, z), f"{kind}[facing={facing},half=lower,hinge={hinge}]")
        return self.set((x, y + 1, z), f"{kind}[facing={facing},half=upper,hinge={hinge}]")

    def reshape(self, positions: Iterable | None = None) -> "World":
        """Recompute shape-dependent states (dust connections, fences, walls, stairs, door halves...)
        from neighbours, like `Block.updateFromNeighbourShapes`, without causing redstone updates."""
        if positions is None:
            positions = [(x, y, z) for (x, y, z, _) in self._w.blocks()]
        for p in positions:
            p = _pos(p)
            self._w.reshape(*p)
        return self

    def setblock(self, pos, block, flags: int = 3) -> bool:
        """`level.setBlock(pos, state, flags)` with full game semantics (3 = update neighbours + clients).
        This is what `/setblock` style edits and most block changes in the game do."""
        if self._trace:
            self._w.trace_input(*_pos(pos), f"setblock {Block(_sid(block)).state}")
        r = self._w.set_block(*_pos(pos), _sid(block), flags)
        if self._trace:
            self._w.end_input()
        return r

    def place(self, pos, block) -> bool:
        """Place like a player would (setBlock with updates, then onPlace/setPlacedBy).
        The given state is used as-is (orientation is not inferred from a player), except
        redstone dust, which gets the shape the game gives a placed dust (lone dust = cross), and
        tripwire, which connects to neighbouring wire and hooks facing it. A placed tripwire hook
        links up with a hook facing it across the wire (setPlacedBy)."""
        sid = _sid(block)
        name = Block(sid).name
        if name == "redstone_wire":
            sid = self._w.dust_placement_state(*_pos(pos), int(Block(sid)["power"]))
        elif name == "tripwire":
            sid = self._w.tripwire_placement_state(*_pos(pos), sid)
        if self._trace:
            self._w.trace_input(*_pos(pos), f"place {Block(sid).state}")
        r = self._w.place_like_player(*_pos(pos), sid)
        if self._trace:
            self._w.end_input()
        return r

    def remove(self, pos) -> bool:
        """Break the block (`destroyBlock`, no drops)."""
        if self._trace:
            self._w.trace_input(*_pos(pos), "break block")
        r = self._w.destroy_block(*_pos(pos))
        if self._trace:
            self._w.end_input()
        return r

    # ------------------------------------------------------------------ inputs

    def use(self, pos) -> bool:
        """Right-click with an empty hand: flip a lever, press a button, cycle a repeater's delay,
        toggle a comparator's mode, open a wooden door/trapdoor/fence gate, tune a note block,
        switch dust between dot and cross. Returns False if the block has no such action."""
        if self._trace:
            self._w.trace_input(*_pos(pos), f"use {self[pos].name}")
        r = self._w.use_block(*_pos(pos))
        if self._trace:
            self._w.end_input()
        return r

    def press(self, pos) -> bool:
        """Press a button (no-op if it is already pressed)."""
        b = self[pos]
        if "button" not in b.name:
            raise ValueError(f"{pos} is {b.name}, not a button")
        return self.use(pos)

    def toggle(self, pos) -> bool:
        """Flip a lever."""
        b = self[pos]
        if b.name != "lever":
            raise ValueError(f"{pos} is {b.name}, not a lever")
        return self.use(pos)

    def pulse(self, pos, ticks: int, block="redstone_block") -> None:
        """Place `block` at pos (with updates), run `ticks` ticks, then restore what was there."""
        old = self[pos]
        self.setblock(pos, block)
        self.tick(ticks)
        self.setblock(pos, old)

    # ------------------------------------------------------------------ time

    @property
    def time(self) -> int:
        """Game time (ticks since the world was created; settable)."""
        return self._w.game_time

    @time.setter
    def time(self, t: int):
        self._w.game_time = t

    @property
    def day_time(self) -> int:
        """The overworld day clock that daylight detectors read: 0 sunrise, 6000 noon, 12000 sunset,
        18000 midnight (24000 per day). Settable."""
        return self._w.day_time

    @day_time.setter
    def day_time(self, t: int):
        self._w.day_time = int(t)

    @property
    def advance_time(self) -> bool:
        """The advance_time game rule (default True, like a new world): the day clock moves one tick
        per game tick. Set False to hold the time of day (the oracle runs with it off)."""
        return self._w.advance_time

    @advance_time.setter
    def advance_time(self, v: bool):
        self._w.advance_time = bool(v)

    def sky_light(self, pos) -> int:
        """Sky light level at pos (0-15, settled), as a daylight detector there would read it."""
        return self._w.sky_light(*_pos(pos))

    def tick(self, n: int = 1) -> "World":
        """Run n game ticks (scheduled ticks, block events, block entities)."""
        self._w.tick(n)
        return self

    def run_until_idle(self, max_ticks: int = 10000) -> int:
        """Tick until nothing is pending (no scheduled ticks, block events or moving pistons).
        Returns the number of ticks run; == max_ticks means it never settled (a clock?)."""
        return self._w.run_until_idle(max_ticks)

    @property
    def idle(self) -> bool:
        return self._w.is_idle()

    # ------------------------------------------------------------------ reading

    def __getitem__(self, pos) -> Block:
        return Block(self._w.get(*_pos(pos)))

    def get(self, pos) -> Block:
        return self[pos]

    def power(self, pos) -> int:
        """Dust: its power level. Other blocks: strongest signal arriving from neighbours."""
        b = self[pos]
        if b.name == "redstone_wire":
            return int(b["power"])
        return self._w.received_power(*_pos(pos))

    def is_powered(self, pos) -> bool:
        """Would a lamp/piston/door at pos be powered (`hasNeighborSignal`)?"""
        return self._w.is_powered(*_pos(pos))

    def conducts(self, pos) -> bool:
        """Is the block at pos a redstone conductor?"""
        return self[pos].conducts

    def settle(self, max_ticks: int = 2000, skip: Iterable = ("piston", "sticky_piston", "observer")) -> int:
        """Make a pasted build internally consistent: every block (except kinds in `skip`) gets one
        neighbour update, in y, z, x order, then the world runs until nothing is pending.
        Returns the ticks that took (== max_ticks: it never settled, e.g. a clock).

        Recipe for builds with moving parts: lay out the circuit with its inputs in the state you
        want, settle(), then place pistons/observers in the matching state. settle(skip=()) lets
        pistons react too."""
        skip = {k if ":" in k else "minecraft:" + k for k in skip}
        for (x, y, z, sid) in sorted(self._w.blocks(), key=lambda b: (b[1], b[2], b[0])):
            if _redsim.block_name(sid) not in skip:
                self._w.update_block(x, y, z)
        return self.run_until_idle(max_ticks)

    def verify(self, inputs: Iterable = (), ticks: int = 40, a=None, b=None, pad: int = 2, oracle=None) -> dict:
        """Run this build in the real game (headless oracle) and in redsim at the same world
        coordinates; compare every block on every tick. `inputs`: (tick, action, pos[, state])
        with action in use, press, setblock, place, remove and absolute positions.
        Both sides run with the day clock held at this world's day_time (for daylight detectors).
        Needs `python tools/setup.py` once. The world must be idle (nothing pending)."""
        from .difftest import Scenario, check
        if not self.idle:
            raise ValueError("verify needs an idle world (scheduled ticks / moving blocks cannot be transferred)")
        if a is None:
            a, b = self.bounds()
        (x0, y0, z0), (x1, y1, z1) = _pos(a), _pos(b)
        x0, x1 = sorted((x0, x1)); y0, y1 = sorted((y0, y1)); z0, z1 = sorted((z0, z1))
        o = (x0 - pad, y0 - pad, z0 - pad)
        size = (x1 - x0 + 1 + 2 * pad, y1 - y0 + 1 + 2 * pad, z1 - z0 + 1 + 2 * pad)
        build = self.extract((o[0], o[1], o[2]), (o[0] + size[0] - 1, o[1] + size[1] - 1, o[2] + size[2] - 1))
        blocks = [(x, y, z, s) for (x, y, z), s in sorted(build.blocks.items(), key=lambda kv: (kv[0][1], kv[0][2], kv[0][0]))]
        bes = []
        for p, d in build.block_entities.items():
            e = {"pos": list(p)}
            if "Items" in d:
                e["items"] = [[it["Slot"], it["id"], it["count"]] for it in d["Items"]]
            if "OutputSignal" in d:
                e["output"] = d["OutputSignal"]
            bes.append(e)
        acts = []
        for inp in inputs:
            t, act, pos = inp[0], inp[1], _pos(inp[2])
            rel = [pos[0] - o[0], pos[1] - o[1], pos[2] - o[2]]
            typ = {"use": "use", "press": "press", "setblock": "setblock", "remove": "destroy", "place": "place"}[act]
            d = {"at": int(t), "type": typ, "pos": rel}
            if len(inp) > 3:
                d["state"] = Block(_sid(inp[3])).state
                if typ == "place" and Block(_sid(inp[3])).name == "redstone_wire":
                    d["type"] = "player_place"
            acts.append(d)
        scen = Scenario(blocks=blocks, size=size, ticks=ticks, actions=acts, block_entities=bes, origin=o,
                        day_time=self.day_time % 24000)
        if oracle is None:
            from .oracle import Oracle
            with Oracle() as orc:
                d, res, sim = check(orc, scen)
        else:
            d, res, sim = check(oracle, scen)
        return {"match": d is None, "ticks": ticks, "origin": o,
                "divergence": None if d is None else d.describe(), "warnings": sim["world"].warnings()}

    def strong_power(self, pos) -> int:
        """Strong ("direct") power going into the block at pos."""
        return self._w.strong_power(*_pos(pos))

    def blocks(self) -> list[tuple[tuple, Block]]:
        return [((x, y, z), Block(s)) for (x, y, z, s) in self._w.blocks()]

    def bounds(self):
        bs = self._w.blocks()
        if not bs:
            return None
        xs, ys, zs = [b[0] for b in bs], [b[1] for b in bs], [b[2] for b in bs]
        return (min(xs), min(ys), min(zs)), (max(xs), max(ys), max(zs))

    @property
    def warnings(self) -> list[str]:
        """Things the simulator could not reproduce exactly (unsupported blocks, fluids, entities)."""
        return [f"{(x, y, z)}: {m}" for (x, y, z, m) in self._w.warnings()]

    def pending(self) -> dict:
        """Pending scheduled ticks and block events (what will happen next)."""
        return {
            "scheduled_ticks": [{"pos": (x, y, z), "block": b, "in": t - self.time, "priority": p}
                                for (x, y, z, b, t, p, _) in self._w.pending_ticks()],
            "block_events": [{"pos": (x, y, z), "block": b, "type": a, "data": d}
                             for (x, y, z, b, a, d) in self._w.pending_block_events()],
        }

    def container(self, pos):
        return self._w.container_items(*_pos(pos))

    def set_container(self, pos, items) -> None:
        """Fill a container: items = [(slot, item_id, count), ...] (no comparator update, like a paste)."""
        self._w.set_container_items(*_pos(pos), [(int(s), str(i), int(c)) for s, i, c in items])

    # ------------------------------------------------------------------ recording & explanation

    # ------------------------------------------------------------------ block entities with redstone output

    def block_entity(self, pos) -> str | None:
        """Summary of the redstone-relevant block entity data at pos (the format the oracle reports):
        comparator:<output>, container:<slot>=<item>x<n>;..., lectern:<page>/<pages>,
        jukebox:<disc>:<playing>:<ticks>, sculk:<last frequency>:<event in flight>:<ticks left>, ..."""
        return self._w.block_entity(*_pos(pos))

    def set_lectern(self, pos, page: int, pages: int) -> bool:
        """Give a lectern (with has_book=true) a book of `pages` pages open at `page` (0-based), like a
        paste: no updates. Comparators read floor(page / (pages - 1) * 14) + 1."""
        return self._w.set_lectern(*_pos(pos), int(page), int(pages))

    def turn_page(self, pos, page: int) -> bool:
        """A player turning a lectern's book to `page`: a 2-tick redstone pulse and a comparator update."""
        return self._w.lectern_turn_page(*_pos(pos), int(page))

    def set_bookshelf_slot(self, pos, slot: int) -> bool:
        """Set a chiseled bookshelf's last interacted slot (0-5, -1 = none); comparators read slot + 1."""
        return self._w.set_bookshelf_slot(*_pos(pos), int(slot))

    def set_jukebox(self, pos, disc: str | None, ticks: int = 0) -> bool:
        """Put a music disc (e.g. "music_disc_cat") in a jukebox, playing from `ticks`, like a paste."""
        return self._w.set_jukebox(*_pos(pos), disc, int(ticks))

    def set_crafter_disabled(self, pos, mask: int) -> bool:
        """Disabled crafter slots as a 9-bit mask (bit i = slot i); comparators count them as filled."""
        return self._w.set_crafter_disabled(*_pos(pos), int(mask))

    def strike(self, pos) -> bool:
        """Lightning hits the lightning rod at pos: it powers for 8 ticks."""
        return self._w.strike_lightning_rod(*_pos(pos))

    def record(self, ticks: int, inputs: dict | None = None, probes: Iterable | None = None) -> Timeline:
        """Run `ticks` ticks recording every block write. `inputs` maps tick offsets to callables
        (or lists of callables) run *before* that tick, e.g. {0: lambda w: w.use(lever)}.
        `probes`: positions whose power is sampled every tick (see Timeline.probes), so a spot
        that stays powered is visible even though nothing is written there."""
        inputs = inputs or {}
        start = self.time
        probe_pos = [_pos(p) for p in (probes or [])]
        samples = {p: Probe(p, [], []) for p in probe_pos}

        def sample():
            for p in probe_pos:
                samples[p].power.append(self.power(p))
                samples[p].powered.append(self._w.is_powered(*p))

        self._w.record_changes(True)
        try:
            for t in range(ticks):
                acts = inputs.get(t, [])
                for a in (acts if isinstance(acts, (list, tuple)) else [acts]):
                    a(self)
                if t == 0:
                    sample()
                self.tick()
                sample()
            raw = self._w.take_changes()
        finally:
            self._w.record_changes(False)
        changes = [Change(tick - start, _PHASES[ph], (x, y, z), Block(o), Block(n)) for (tick, ph, x, y, z, o, n) in raw]
        return Timeline(changes, 0, ticks, samples)

    def enable_trace(self) -> None:
        """Record the causal trace needed by `explain` (slower; off by default)."""
        self._w.enable_trace()
        self._trace = True

    def explain(self, pos, which: int = -1, full: bool = False) -> str:
        """Why does the block at pos look the way it does? Walks back the chain of causes of
        the `which`-th change at pos (default: the latest) to the input that started it.
        Runs of dust-to-dust update hops are collapsed unless full=True."""
        if not self._trace:
            return "tracing is off: create World(trace=True) or call enable_trace() before running"
        ids = self._w.trace_sets_at(*_pos(pos))
        if not ids:
            return f"{_pos(pos)} has not changed since tracing started"
        chain = self._w.trace_chain(ids[which])
        lines = [self._w.trace_describe(i) for i in chain]
        if not full:
            out, run = [], []
            for l in lines:
                if "neighbor update from redstone_wire" in l:
                    run.append(l)
                    continue
                if len(run) > 2:
                    out += [run[0], f"... {len(run) - 2} more dust-to-dust update hops ...", run[-1]]
                else:
                    out += run
                run = []
                out.append(l)
            out += run
            lines = out
        return "\n".join(f"{'  ' * min(k, 12)}{'<- ' if k else ''}{l}" for k, l in enumerate(lines))

    def nets(self, a=None, b=None):
        """Redstone dust networks (see redsim.nets): which dust feeds which by the game's rules,
        one-way links (e.g. dust stairs on glass), blocks each network powers, and the signal
        sources touching it. Use it to catch signals that touch or bleed into each other."""
        from .nets import find_nets
        return find_nets(self, None if a is None else _pos(a), None if b is None else _pos(b))

    def net_of(self, pos):
        """The dust network containing pos (or None)."""
        p = _pos(pos)
        for n in self.nets():
            if p in n.dust:
                return n
        return None

    def route(self, start, end, **kw):
        """Wire dust from the source block `start` to the block `end` so it touches no other circuitry,
        proven with nets() and a simulation before anything is written (see redsim.route.route for
        the options). Returns a RouteResult (path, placed, delay_ticks, report, undo()); raises
        redsim.route.RouteError, leaving the world unchanged, when no safe route exists."""
        from .route import route
        return route(self, start, end, **kw)

    def why_not_piston(self, pos) -> str:
        """Diagnose a piston: where it gets power from (incl. quasi-connectivity) and whether it can move."""
        return self._w.piston_diagnose(*_pos(pos))

    # ------------------------------------------------------------------ schematics

    def paste(self, build, origin=(0, 0, 0)) -> "World":
        """Paste a Build (or schematic path) with its minimum corner at `origin`, raw (no updates)."""
        if not isinstance(build, Build):
            build = load_schematic(build)
        ox, oy, oz = _pos(origin)
        for (x, y, z) in sorted(build.blocks, key=lambda p: (p[1], p[2], p[0])):
            self._w.place(x + ox, y + oy, z + oz, _sid(build.blocks[(x, y, z)]))
        for (x, y, z), data in build.block_entities.items():
            p = (x + ox, y + oy, z + oz)
            if "Items" in data:
                try:
                    self.set_container(p, [(it["Slot"], it["id"], it.get("count", it.get("Count", 1))) for it in data["Items"]])
                except KeyError:
                    pass
            if "OutputSignal" in data:
                try:
                    self._w.set_comparator_output(*p, int(data["OutputSignal"]))
                except KeyError:
                    pass
        for t in build.pending_ticks:
            x, y, z = t["pos"]
            self._w.schedule_tick(x + ox, y + oy, z + oz, t["block"], int(t["delay"]), int(t.get("priority", 0)))
        return self

    def extract(self, a=None, b=None, name: str = "redsim build") -> Build:
        """Copy a box (default: everything) into a Build, with container/comparator data."""
        if a is None:
            bb = self.bounds()
            if bb is None:
                return Build(size=(1, 1, 1), name=name)
            a, b = bb
        (x0, y0, z0), (x1, y1, z1) = _pos(a), _pos(b)
        x0, x1 = sorted((x0, x1))
        y0, y1 = sorted((y0, y1))
        z0, z1 = sorted((z0, z1))
        out = Build(size=(x1 - x0 + 1, y1 - y0 + 1, z1 - z0 + 1), name=name)
        for (x, y, z, s) in self._w.blocks():
            if x0 <= x <= x1 and y0 <= y <= y1 and z0 <= z <= z1:
                out.blocks[(x - x0, y - y0, z - z0)] = _redsim.format_state(s)
                items = self._w.container_items(x, y, z)
                if items:
                    out.block_entities[(x - x0, y - y0, z - z0)] = {
                        "Items": [{"Slot": sl, "id": i, "count": c} for (sl, i, c) in items]}
                be = self._w.block_entity(x, y, z)
                if be and be.startswith("comparator:"):
                    out.block_entities[(x - x0, y - y0, z - z0)] = {"OutputSignal": int(be.split(":")[1])}
        for (x, y, z, blk, t, p, _) in self._w.pending_ticks():
            if x0 <= x <= x1 and y0 <= y <= y1 and z0 <= z <= z1:
                out.pending_ticks.append({"pos": (x - x0, y - y0, z - z0), "block": blk, "delay": t - self.time, "priority": p})
        return out

    def save(self, path, a=None, b=None, **kw) -> Build:
        """Save (part of) the world as .litematic / .schem / .nbt (by extension)."""
        build = self.extract(a, b)
        build.save(path, **kw)
        return build

    def copy(self) -> "World":
        w = World.__new__(World)
        w._w = self._w.copy()
        w._trace = self._trace
        return w

    # ------------------------------------------------------------------ display

    def render(self, y: int | None = None, a=None, b=None, legend: bool = True, view: str = "top",
               at: int | None = None) -> str:
        """Text view (see redsim.render for the symbols). view="top" draws horizontal layers
        (one layer if y is given); view="front" draws the vertical plane z=at seen from the south
        (x to the right, y up); view="side" draws the plane x=at seen from the west, looking east
        (z to the right, y up). Without `at`, every slice in the box is drawn."""
        from .render import render_layers, render_vertical
        if view == "top":
            return render_layers(self, y=y, a=a, b=b, legend=legend)
        return render_vertical(self, view=view, at=at, a=a, b=b, legend=legend)

    def __repr__(self):
        bb = self.bounds()
        return f"<redsim.World t={self.time} blocks={len(self._w.blocks())} bounds={bb}>"


def load(path, origin=(0, 0, 0), trace: bool = False) -> World:
    """Load a .litematic / .schem / .nbt into a new World at `origin`."""
    w = World(trace=trace)
    w.paste(load_schematic(path), origin)
    return w
