"""Dust network analysis: which redstone dust feeds which, by the game's exact rules.

Power moves between dust on the same level, up a step (only if the block between conducts and
the block above the lower dust does not), and down a step (only if the side block does not
conduct). Those rules are not symmetric: dust stairs on glass carry signal upward only.
"""

from __future__ import annotations

from dataclasses import dataclass, field

from . import _redsim


@dataclass
class Net:
    dust: list                                     # dust positions in this network
    one_way: list = field(default_factory=list)    # (from, to): power flows from->to but not back
    powers: list = field(default_factory=list)     # (pos, block) non-dust blocks the dust sends power into
    sources: list = field(default_factory=list)    # (pos, block) adjacent signal sources that can feed it

    def __contains__(self, pos):
        return tuple(pos) in set(self.dust)

    def __repr__(self):
        return (f"Net({len(self.dust)} dust, {len(self.one_way)} one-way links, powers {len(self.powers)} blocks, "
                f"{len(self.sources)} sources)")


def find_nets(world, a=None, b=None) -> list[Net]:
    from . import Block
    w = world._w
    dust = [(x, y, z) for (x, y, z, s) in w.blocks() if _redsim.block_name(s) == "minecraft:redstone_wire"]
    if a is not None:
        (x0, y0, z0), (x1, y1, z1) = a, b
        dust = [p for p in dust if min(x0, x1) <= p[0] <= max(x0, x1) and min(y0, y1) <= p[1] <= max(y0, y1)
                and min(z0, z1) <= p[2] <= max(z0, z1)]
    dset = set(dust)
    inputs = {p: [tuple(q) for q in w.dust_inputs(*p)] for p in dust}
    adj = {p: set() for p in dust}
    for p, ins in inputs.items():
        for q in ins:
            if q in adj:
                adj[p].add(q)
                adj[q].add(p)
    seen, nets = set(), []
    for p in sorted(dust):
        if p in seen:
            continue
        comp, stack = [], [p]
        seen.add(p)
        while stack:
            c = stack.pop()
            comp.append(c)
            for q in adj[c]:
                if q not in seen:
                    seen.add(q)
                    stack.append(q)
        cset = set(comp)
        one_way = sorted({(q, p2) for p2 in comp for q in inputs[p2] if q in cset and p2 not in inputs.get(q, [])})
        powers, sources = {}, {}
        for c in comp:
            for o in w.dust_outputs(*c):
                o = tuple(o)
                if o not in dset:
                    bl = Block(w.get(*o))
                    if bl.name not in ("air", "cave_air", "void_air"):
                        powers[o] = bl.name
            for d in ((1, 0, 0), (-1, 0, 0), (0, 0, 1), (0, 0, -1), (0, 1, 0), (0, -1, 0)):
                n = (c[0] + d[0], c[1] + d[1], c[2] + d[2])
                if n in dset:
                    continue
                bl = Block(w.get(*n))
                if bl.is_signal_source:
                    sources[n] = bl.name
        nets.append(Net(sorted(comp), one_way, sorted(powers.items()), sorted(sources.items())))
    return nets
