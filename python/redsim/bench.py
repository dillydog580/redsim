"""Speed benchmark: busy redstone builds, measured in game ticks per second.

`redsim bench` runs the three headline builds; `redsim bench --all` adds a wider suite (torch
logic, repeater lines, comparators, doors and other powered blocks, rails, hoppers) that covers
most of the simulator. The builders use only the public World API, so the same builds can be run
in any version of redsim (tools/compare_versions.py does that to check speed work).
"""

from __future__ import annotations

import time

import redsim


def observer_clock(w, ox=0, oz=0):
    """Two observers facing each other: a 4-tick clock driving a lamp."""
    w.set((ox, 0, oz), "observer[facing=east]")
    w.set((ox - 1, 0, oz), "redstone_lamp")
    w.place((ox + 1, 0, oz), "observer[facing=west]")  # placing it starts the clock


def piston_row(w, n=8, ox=0, oz=0):
    """Observer clock -> dust line -> n sticky pistons pushing slime+stone up and down."""
    observer_clock(w, ox, oz)
    w.set((ox - 1, 0, oz), "stone")
    for i in range(n):
        x = ox - 1 - i
        w.set((x, -1, oz), "stone")
        w.set((x, 0, oz), "redstone_wire")
        w.set((x, -1, oz + 1), "stone")
        w.set((x, 0, oz + 1), "sticky_piston[facing=up]")
        w.set((x, 1, oz + 1), "slime_block")
        w.set((x, 2, oz + 1), "stone")
    w.reshape()


def dust_grid(w, n=32, ox=0, oz=0):
    """A 32x32 plane of dust toggled by a lever every 20 ticks (worst case dust updates)."""
    w.fill((ox, 0, oz), (ox + n - 1, 0, oz + n - 1), "stone")
    for x in range(n):
        for z in range(n):
            w.set((ox + x, 1, oz + z), "redstone_wire")
    w.reshape()
    w.set((ox - 1, 0, oz), "stone")
    w.set((ox - 1, 1, oz), "lever[face=floor,facing=east]")


def _bus(w, n, z=0):
    """Lever at (-1, 1, z) feeding a dust line along +x at y=1 (on stone), x = 0..n-1."""
    w.fill((-1, 0, z), (n - 1, 0, z), "stone")
    w.set((-1, 1, z), "lever[face=floor,facing=east]")
    for x in range(n):
        w.set((x, 1, z), "redstone_wire")


def _taps(w, n, every=1):
    """Repeaters at z=1 taking the bus signal south (input from the bus, output to z=2)."""
    for x in range(0, n, every):
        w.set((x, 0, 1), "stone")
        w.set((x, 1, 1), "repeater[facing=north]")


def torch_logic(w, n=14):
    """Lever bus -> repeaters -> blocks with torch inverters (a wall torch on the side, a torch on
    top), each driving a lamp."""
    _bus(w, n)
    _taps(w, n)
    for x in range(n):
        w.set((x, 0, 2), "stone")
        w.set((x, 1, 2), "stone")
        w.set((x, 2, 2), "redstone_torch")
        w.set((x, 3, 2), "redstone_lamp")
        w.set((x, 1, 3), "redstone_wall_torch[facing=south]")
        w.set((x, 2, 3), "redstone_lamp")
    w.reshape()


def repeater_line(w, n=24):
    """Lever -> a long line alternating repeaters (delays 1-4) and dust sitting on lamps."""
    w.set((-2, 0, 0), "stone")
    w.set((-1, 0, 0), "stone")
    w.set((-1, 1, 0), "lever[face=floor,facing=east]")
    for i in range(n):
        w.set((2 * i, 0, 0), "stone")
        w.set((2 * i, 1, 0), f"repeater[facing=west,delay={1 + i % 4}]")
        w.set((2 * i + 1, 0, 0), "redstone_lamp")
        w.set((2 * i + 1, 1, 0), "redstone_wire")
    w.reshape()


def comparator_bank(w, n=12):
    """Lever bus -> comparators (compare and subtract mode) with side inputs from dust branches,
    outputs into dust and lamps."""
    _bus(w, 2 * n)
    for i in range(n):
        x = 2 * i
        mode = "subtract" if i % 2 else "compare"
        w.fill((x, 0, 1), (x + 1, 0, 2), "stone")
        w.set((x, 1, 1), f"comparator[facing=north,mode={mode}]")
        w.set((x + 1, 1, 1), "redstone_wire")
        w.set((x, 1, 2), "redstone_wire")
        w.set((x, 1, 3), "redstone_lamp")
    w.reshape()


def powered_blocks(w, n=16):
    """Lever bus -> repeaters -> doors, trapdoors, fence gates, note blocks, dispensers,
    droppers, hoppers, copper bulbs and lamps."""
    _bus(w, n)
    _taps(w, n)
    side = ["iron_door", "iron_trapdoor[facing=north,half=bottom]", "oak_fence_gate[facing=north]",
            "note_block", "dispenser[facing=up]", "dropper[facing=up]", "hopper", "copper_bulb", "redstone_lamp"]
    for x in range(n):
        b = side[x % len(side)]
        w.set((x, 0, 2), "stone")
        if b == "iron_door":
            w.set((x, 1, 2), "iron_door[facing=south,half=lower]")
            w.set((x, 2, 2), "iron_door[facing=south,half=upper]")
        else:
            w.set((x, 1, 2), b)
    w.reshape()


def rail_line(w, n=30):
    """Lever -> powered and activator rails along a track (power reaches 8 rails), next to a
    line of plain rails."""
    w.fill((-1, 0, 0), (n, 0, 1), "stone")
    w.set((-1, 1, 0), "lever[face=floor,facing=east]")
    for x in range(n):
        w.set((x, 1, 0), "powered_rail[shape=east_west]" if (x // 9) % 2 == 0 else "activator_rail[shape=east_west]")
        w.set((x, 1, 1), "rail[shape=east_west]")
    w.reshape()


def hopper_chain(w, n=16):
    """A line of hoppers passing items along, each read by a comparator into dust; a lever locks
    and unlocks the first hopper."""
    w.fill((0, 0, -1), (n, 0, 2), "stone")
    for x in range(n):
        w.set((x, 1, 0), "hopper[facing=east]")
        w.set((x, 1, 1), "comparator[facing=north]")
        w.set((x, 1, 2), "redstone_wire")
    w.set((n, 1, 0), "chest[facing=north]")
    w.set((0, 1, -1), "lever[face=floor,facing=north]")
    w.reshape()
    w.set_container((0, 1, 0), [(s, "minecraft:cobblestone", 64) for s in range(5)])


# name: (builder, ticks per toggle (None = runs by itself), toggle position, ticks per sample)
SUITE = {
    "observer clock (4 gt period)": (observer_clock, None, None, 400000),
    "observer clock + 8 slime-stone pistons": (piston_row, None, None, 20000),
    "32x32 dust plane, lever every 20 gt": (dust_grid, 20, (-1, 1, 0), 2000),
    "torch inverters, lever every 20 gt": (torch_logic, 20, (-1, 1, 0), 20000),
    "repeater line, lever every 60 gt": (repeater_line, 60, (-1, 1, 0), 20000),
    "comparator bank, lever every 20 gt": (comparator_bank, 20, (-1, 1, 0), 20000),
    "doors, gates, bulbs..., lever every 20 gt": (powered_blocks, 20, (-1, 1, 0), 20000),
    "powered/activator rails, lever every 20 gt": (rail_line, 20, (-1, 1, 0), 20000),
    "hopper chain with comparators, lever every 100 gt": (hopper_chain, 100, (0, 1, -1), 20000),
}
HEADLINE = list(SUITE)[:3]


def build(name):
    """A fresh World with the named benchmark build, and its driver: run(world, ticks)."""
    builder, every, toggle, _ = SUITE[name]
    w = redsim.World()
    builder(w)

    def drive(w, ticks):
        if every is None:
            w.tick(ticks)
            return
        done = 0
        while done < ticks:
            if (w.time) % every == 0:
                w.use(toggle)
            step = min(every - w.time % every, ticks - done)
            w.tick(step)
            done += step

    return w, drive


def run(name, ticks=None):
    w, drive = build(name)
    ticks = ticks or SUITE[name][3]
    probe = w.record(40)
    if len(probe) == 0 and SUITE[name][1] is None:
        raise SystemExit(f"benchmark build '{name}' is idle (broken benchmark)")
    t0 = time.perf_counter()
    drive(w, ticks)
    dt = time.perf_counter() - t0
    tps = ticks / dt
    print(f"{name:52s} {tps:12,.0f} ticks/s  ({dt * 1e6 / ticks:8.2f} us/tick, {len(w.blocks())} blocks)")
    return tps


def main(a=None):
    names = list(SUITE) if getattr(a, "all", False) else HEADLINE
    scale = getattr(a, "ticks", None)
    print("redsim simulation speed (one core):")
    for name in names:
        n = SUITE[name][3]
        if scale:
            n = max(200, scale * n // 20000)
        run(name, n)
    print("The game runs 20 ticks/s in real time.")
    return 0


if __name__ == "__main__":
    main()
