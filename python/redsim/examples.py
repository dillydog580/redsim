"""Known builds: classic contraptions used as examples and as tests against the real game.

Each builder places blocks into a World relative to `o` (origin) and returns a dict with
`inputs` (tick -> list of (action, rel_pos[, state])), `ticks`, and `expect`
(spec-style expectations, positions relative to the origin).
"""

from __future__ import annotations


def _add(o, p):
    return (o[0] + p[0], o[1] + p[1], o[2] + p[2])


def door_2x2(w, o=(0, 0, 0)):
    """Classic 2x2 sticky-piston door in the z=0 wall plane, doorway x=0..1, y=1..2.
    Lever ON closes it (pistons extend), OFF opens it. Lower pistons are powered by
    quasi-connectivity from the block next to the upper piston."""
    s = lambda p, b: w.set(_add(o, p), b)
    for x in range(-4, 6):
        for z in (-1, 0, 1):
            s((x, 0, z), "stone")
    for y in (1, 2):
        s((-2, y, 0), "sticky_piston[facing=east]")
        s((3, y, 0), "sticky_piston[facing=west]")
        s((-1, y, 0), "smooth_stone")
        s((2, y, 0), "smooth_stone")
    for x in (-3, 4):
        s((x, 1, 0), "stone")
        s((x, 2, 0), "stone")
    for x in range(-4, 5):
        s((x, 2, 1), "stone")
    s((-4, 3, 1), "lever[face=floor,facing=east]")
    w.wire(_add(o, (-3, 3, 1)), _add(o, (4, 3, 1)))
    w.wire(_add(o, (-3, 3, 0)), _add(o, (-3, 3, 0)))
    w.wire(_add(o, (4, 3, 0)), _add(o, (4, 3, 0)))
    w.reshape([_add(o, p) for p in [(-3, 3, 1), (-3, 3, 0), (4, 3, 1), (4, 3, 0)]])
    doorway = [(0, 1, 0), (0, 2, 0), (1, 1, 0), (1, 2, 0)]
    return {
        "inputs": {0: [("use", (-4, 3, 1))], 20: [("use", (-4, 3, 1))]},
        "ticks": 40,
        "expect": [{"at": 4, "pos": p, "is": "smooth_stone"} for p in doorway]
        + [{"at": 24, "pos": p, "is": "air"} for p in doorway],
    }


def iron_door(w, o=(0, 0, 0)):
    """Stone button on a block next to an iron door: opens for 20 ticks."""
    s = lambda p, b: w.set(_add(o, p), b)
    s((0, 0, 0), "stone")
    s((1, 0, 0), "stone")
    s((0, 1, 0), "iron_door[facing=south,half=lower]")
    s((0, 2, 0), "iron_door[facing=south,half=upper]")
    s((1, 1, 0), "stone")
    s((1, 1, 1), "stone_button[face=wall,facing=south]")
    return {
        "inputs": {0: [("use", (1, 1, 1))]},
        "ticks": 30,
        "expect": [{"at": 1, "pos": (0, 1, 0), "is": "iron_door[open=true]"},
                   {"at": 1, "pos": (0, 2, 0), "is": "iron_door[open=true]"},
                   {"at": 21, "pos": (0, 1, 0), "is": "iron_door[open=false]"}],
    }


def copper_bulb_tff(w, o=(0, 0, 0)):
    """T flip-flop: each button press toggles a copper bulb; a comparator reads it out to a lamp."""
    s = lambda p, b: w.set(_add(o, p), b)
    for x in range(0, 5):
        s((x, 0, 0), "stone")
    s((0, 1, 0), "stone")
    s((0, 1, 1), "stone_button[face=wall,facing=south]")
    s((1, 1, 0), "waxed_copper_bulb")
    s((2, 1, 0), "comparator[facing=west]")
    s((3, 1, 0), "redstone_wire")
    s((4, 1, 0), "redstone_lamp")
    w.reshape([_add(o, (3, 1, 0))])
    return {
        "inputs": {0: [("use", (0, 1, 1))], 40: [("use", (0, 1, 1))]},
        "ticks": 80,
        "expect": [{"at": 6, "pos": (4, 1, 0), "is": "redstone_lamp[lit=true]"},
                   {"from": 6, "to": 40, "pos": (4, 1, 0), "is": "redstone_lamp[lit=true]"},
                   {"at": 48, "pos": (4, 1, 0), "is": "redstone_lamp[lit=false]"}],
    }


def observer_clock(w, o=(0, 0, 0)):
    """Two observers watching each other: a 4-tick clock blinking a lamp."""
    s = lambda p, b: w.set(_add(o, p), b)
    s((0, 0, 0), "observer[facing=east]")
    s((-1, 0, 0), "redstone_lamp")
    return {"inputs": {0: [("place", (1, 0, 0), "observer[facing=west]")]}, "ticks": 40,
            "expect": [{"by": 6, "pos": (-1, 0, 0), "is": "redstone_lamp[lit=true]"}]}


def repeater_loop(w, o=(0, 0, 0)):
    """A pulse circling a ring of repeaters (a clock), started by a button."""
    s = lambda p, b: w.set(_add(o, p), b)
    for x in range(0, 4):
        for z in range(0, 4):
            s((x, 0, z), "stone")
    s((1, 1, 0), "repeater[facing=west,delay=1]")
    s((2, 1, 0), "repeater[facing=west,delay=1]")
    s((3, 1, 1), "repeater[facing=north,delay=1]")
    s((3, 1, 2), "repeater[facing=north,delay=1]")
    s((2, 1, 3), "repeater[facing=east,delay=1]")
    s((1, 1, 3), "repeater[facing=east,delay=1]")
    s((0, 1, 2), "repeater[facing=south,delay=1]")
    s((0, 1, 1), "repeater[facing=south,delay=1]")
    for c in [(0, 1, 0), (3, 1, 0), (3, 1, 3), (0, 1, 3)]:
        s(c, "redstone_wire")
    s((-1, 0, 0), "stone")
    w.reshape([_add(o, c) for c in [(0, 1, 0), (3, 1, 0), (3, 1, 3), (0, 1, 3)]])
    # kick it with a 1-tick pulse from a redstone block next to the first corner
    return {"inputs": {0: [("setblock", (-1, 1, 0), "redstone_block")], 1: [("setblock", (-1, 1, 0), "air")]},
            "ticks": 80, "expect": [{"from": 60, "to": 80, "pos": (1, 1, 0), "is": "repeater"}]}


def bud_switch(w, o=(0, 0, 0)):
    """Quasi-connectivity BUD: a piston powered from above-diagonal does not extend until it
    gets a block update (here: placing a block next to it)."""
    s = lambda p, b: w.set(_add(o, p), b)
    s((0, 0, 0), "stone")
    s((0, 1, 0), "piston[facing=east]")
    s((-1, 1, 0), "stone")
    s((-1, 2, 0), "redstone_block")
    return {"inputs": {10: [("setblock", (0, 1, 1), "stone")]}, "ticks": 20,
            "expect": [{"at": 10, "pos": (0, 1, 0), "is": "piston[extended=false]"},
                       {"at": 13, "pos": (0, 1, 0), "is": "piston[extended=true]"}]}


def comparator_subtraction(w, o=(0, 0, 0)):
    """A comparator in subtract mode reading a chest, minus a side signal of 5."""
    s = lambda p, b: w.set(_add(o, p), b)
    for x in range(0, 4):
        s((x, 0, 0), "stone")
    s((0, 1, 0), "chest[facing=north]")
    w.set_container(_add(o, (0, 1, 0)), [(i, "minecraft:stone", 64) for i in range(14)])
    s((1, 1, 0), "comparator[facing=west,mode=subtract]")
    s((2, 1, 0), "redstone_wire")
    s((3, 1, 0), "redstone_wire")
    for z in range(1, 12):
        s((1, 0, z), "stone")
    w.wire(_add(o, (1, 1, 10)), _add(o, (1, 1, 1)))
    w.reshape()
    # chest signal 8; side signal 6 (redstone block at the end of a 10-long dust line) -> output 2;
    # removing the side signal raises the output to 8.
    return {"inputs": {0: [("setblock", (1, 1, 11), "redstone_block")], 10: [("setblock", (1, 1, 11), "air")]},
            "ticks": 30, "expect": [{"at": 8, "pos": (3, 1, 0), "is": "redstone_wire[power=1]"},
                                    {"at": 18, "pos": (3, 1, 0), "is": "redstone_wire[power=7]"}]}


def hopper_clock(w, o=(0, 0, 0)):
    """Two hoppers feeding each other, alternately locked by comparators reading them: a slow clock."""
    s = lambda p, b: w.set(_add(o, p), b)
    for x in range(-1, 3):
        for z in range(0, 3):
            s((x, 0, z), "stone")
    s((0, 1, 0), "hopper[facing=east]")
    s((1, 1, 0), "hopper[facing=west]")
    w.set_container(_add(o, (0, 1, 0)), [(0, "minecraft:redstone", 5)])
    s((0, 1, 1), "comparator[facing=north]")
    s((1, 1, 1), "comparator[facing=north]")
    s((0, 1, 2), "sticky_piston[facing=east]")
    s((1, 1, 2), "redstone_block")
    return {"inputs": {0: [("setblock", (2, 1, 0), "redstone_block")], 30: [("setblock", (2, 1, 0), "air")]},
            "ticks": 200, "expect": [{"by": 60, "pos": (1, 1, 1), "is": "comparator[powered=true]"}]}


def hopper_chain(w, o=(0, 0, 0)):
    """A chest of items drained by a chain of hoppers into a second chest, read by a comparator."""
    s = lambda p, b: w.set(_add(o, p), b)
    for x in range(0, 6):
        s((x, 0, 1), "stone")
    s((0, 2, 0), "chest[facing=south]")
    w.set_container(_add(o, (0, 2, 0)), [(0, "minecraft:stone", 20), (3, "minecraft:ender_pearl", 3)])
    s((0, 1, 0), "hopper[facing=east]")
    s((1, 1, 0), "hopper[facing=east]")
    s((2, 1, 0), "hopper[facing=down]")
    s((2, 0, 0), "chest[facing=south]")
    s((3, 0, 0), "comparator[facing=west]")
    s((4, 0, 0), "redstone_wire")
    s((4, -1, 0), "stone")
    s((3, -1, 0), "stone")
    s((0, 1, 1), "lever[face=wall,facing=south]")
    return {"inputs": {100: [("use", (0, 1, 1))], 140: [("use", (0, 1, 1))]}, "ticks": 300,
            "expect": [{"by": 40, "pos": (4, 0, 0), "is": "redstone_wire[power=1]"}]}


def dropper_loop(w, o=(0, 0, 0)):
    """Two droppers facing each other pass one item back and forth on each pulse (dropper T flip-flop core)."""
    s = lambda p, b: w.set(_add(o, p), b)
    s((0, 0, 0), "dropper[facing=east]")
    s((1, 0, 0), "dropper[facing=west]")
    w.set_container(_add(o, (0, 0, 0)), [(4, "minecraft:stone", 1)])
    s((0, 0, 1), "comparator[facing=north]")
    s((0, -1, 1), "stone")
    s((0, 1, 0), "stone")
    s((0, 2, 0), "stone_button[face=floor,facing=north]")
    s((1, 1, 0), "stone")
    s((1, 2, 0), "stone_button[face=floor,facing=north]")
    return {"inputs": {0: [("use", (0, 2, 0))], 40: [("use", (1, 2, 0))], 80: [("use", (0, 2, 0))]}, "ticks": 120,
            "expect": [{"at": 10, "pos": (0, 0, 1), "is": "comparator[powered=false]"}]}


ALL = {f.__name__: f for f in [door_2x2, iron_door, copper_bulb_tff, observer_clock, repeater_loop,
                               bud_switch, comparator_subtraction, hopper_clock, hopper_chain, dropper_loop]}
