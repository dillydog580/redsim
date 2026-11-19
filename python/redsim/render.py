"""Text rendering of a layer, top-down (x to the right, z downward = north at the top).

Each cell is two characters. Arrows show the direction a signal or face points:
'^' north, 'v' south, '<' west, '>' east, 'u' up, 'd' down.

    . .  air                    ##  conductive solid block     []  other solid block
     7   dust with power 7      >2  repeater -> east, delay 2  >*  powered repeater
    >c   comparator (compare)   >s  subtract   (C/S = powered) T*  torch lit (t = wall torch)
    L*   lever on               B*  button pressed             O*  lamp lit
    RB   redstone block         E>  observer outputting east (e = powered)
    P>   piston facing east     S>  sticky piston             =>  piston head  ~~ moving block
    D|   door closed (D/ open)  d|  trapdoor   G| fence gate   N*  note block powered
    r|   rail (| - / \\ or a slope arrow)   q|  powered rail (q* powered)   a| activator   x| detector
    h>   tripwire hook (h* powered)   --  tripwire (-* pressed)   Y7  daylight detector power 7 (y = inverted)
    K.   sculk sensor (K* active, Kc cooldown; k = calibrated)   Am  amethyst block   Wl  wool
"""

from __future__ import annotations

ARROW = {"north": "^", "south": "v", "west": "<", "east": ">", "up": "u", "down": "d"}
OPP = {"north": "south", "south": "north", "west": "east", "east": "west", "up": "down", "down": "up"}


def glyph(b) -> str:
    return _glyph(b)


def _glyph(b) -> str:
    n = b.name
    p = b.props
    on = lambda k: p.get(k) == "true"
    if n in ("air", "cave_air", "void_air"):
        return ". "
    if n == "redstone_wire":
        return f"{int(p['power']):2d}"
    if n == "repeater":
        a = ARROW[OPP[p["facing"]]]
        return a + ("*" if on("powered") else p["delay"])
    if n == "comparator":
        a = ARROW[OPP[p["facing"]]]
        m = "c" if p["mode"] == "compare" else "s"
        return a + (m.upper() if on("powered") else m)
    if n == "redstone_torch":
        return "T*" if on("lit") else "T "
    if n == "redstone_wall_torch":
        return "t*" if on("lit") else "t "
    if n == "lever":
        return "L*" if on("powered") else "L "
    if n.endswith("_button"):
        return "B*" if on("powered") else "B "
    if n == "redstone_lamp":
        return "O*" if on("lit") else "O "
    if n == "redstone_block":
        return "RB"
    if n == "observer":
        return ("e" if on("powered") else "E") + ARROW[OPP[p["facing"]]]
    if n in ("piston", "sticky_piston"):
        return ("S" if n == "sticky_piston" else "P") + ARROW[p["facing"]]
    if n == "piston_head":
        return "=" + ARROW[p["facing"]]
    if n == "moving_piston":
        return "~~"
    if n.endswith("_door"):
        return "D/" if on("open") else "D|"
    if n.endswith("_trapdoor"):
        return "d/" if on("open") else "d|"
    if n.endswith("_fence_gate"):
        return "G/" if on("open") else "G|"
    if n == "note_block":
        return "N*" if on("powered") else "N "
    if n == "target":
        return "Tg"
    if n.endswith("rail"):
        shape = p.get("shape", "north_south")
        g = {"north_south": "|", "east_west": "-", "north_east": "\\", "south_west": "\\",
             "north_west": "/", "south_east": "/"}.get(shape)
        if g is None:  # ascending_<dir>: arrow up the slope
            g = ARROW[shape.split("_")[1]]
        letter = {"rail": "r", "powered_rail": "q", "activator_rail": "a", "detector_rail": "x"}[n]
        return letter + ("*" if on("powered") else g)
    if n == "tripwire_hook":
        return "h" + ("*" if on("powered") else ARROW[p["facing"]])
    if n == "tripwire":
        return "-*" if on("powered") else "--"
    if n == "daylight_detector":
        return ("y" if on("inverted") else "Y") + format(int(p["power"]), "x")
    if n in ("sculk_sensor", "calibrated_sculk_sensor"):
        ph = {"active": "*", "cooldown": "c"}.get(p["sculk_sensor_phase"], ".")
        return ("k" if n.startswith("calibrated") else "K") + ph
    if n == "amethyst_block":
        return "Am"
    if n.endswith("_wool"):
        return "Wl"
    if n in ("slime_block",):
        return "Sl"
    if n in ("honey_block",):
        return "Hn"
    if b.kind == "Plain" or b.kind == "Container":
        flags = _flags(b.id)
        if flags & 1:  # conductor
            return "##"
        return "[]"
    return n[:2]


def _flags(sid):
    from . import _redsim
    return _redsim.state_info(sid)[0]


def render_layers(world, y=None, a=None, b=None, legend=True) -> str:
    if a is None:
        bb = world.bounds()
        if bb is None:
            return "(empty world)"
        a, b = bb
    (x0, y0, z0), (x1, y1, z1) = a, b
    ys = [y] if y is not None else list(range(min(y0, y1), max(y0, y1) + 1))
    out = []
    used = {}
    for yy in ys:
        out.append(f"y={yy}   x {min(x0, x1)}..{max(x0, x1)} ->, z {min(z0, z1)}..{max(z0, z1)} downward")
        for z in range(min(z0, z1), max(z0, z1) + 1):
            row = []
            for x in range(min(x0, x1), max(x0, x1) + 1):
                blk = world[(x, yy, z)]
                g = glyph(blk)
                if g != ". ":
                    used.setdefault(g, blk.state.replace("minecraft:", ""))
                row.append(g)
            out.append(f"{z:>5} " + " ".join(row))
        out.append("")
    if legend and used:
        out.append("legend:")
        for g, st in sorted(used.items()):
            out.append(f"  {g!r:6} {st}")
    return "\n".join(out)


# Arrow characters re-mapped for vertical views. Front view: standing south of the plane looking
# north (x to the right, y up): east '>', west '<', up '^', down 'v', north (away) 'x', south
# (toward you) 'o'. Side view: standing west of the plane looking east (z to the right, y up):
# south '>', north '<', up '^', down 'v', east (away) 'x', west (toward you) 'o'.
FRONT = {"^": "x", "v": "o", "u": "^", "d": "v"}
SIDE = {"^": "<", "v": ">", ">": "x", "<": "o", "u": "^", "d": "v"}


def render_vertical(world, view="front", at=None, a=None, b=None, legend=True) -> str:
    if a is None:
        bb = world.bounds()
        if bb is None:
            return "(empty world)"
        a, b = bb
    (x0, y0, z0), (x1, y1, z1) = a, b
    xs = range(min(x0, x1), max(x0, x1) + 1)
    ys = range(max(y0, y1), min(y0, y1) - 1, -1)
    zs = range(min(z0, z1), max(z0, z1) + 1)
    used, out = {}, []
    if view == "front":
        planes = [at] if at is not None else list(zs)
        amap = FRONT
        for z in planes:
            out.append(f"front view z={z} (looking north): x {xs.start}..{xs.stop - 1} ->, y downward from {ys.start}")
            for y in ys:
                row = []
                for x in xs:
                    blk = world[(x, y, z)]
                    g = _view_glyph(blk, amap)
                    if g != ". ":
                        used.setdefault(g, blk.state.replace("minecraft:", ""))
                    row.append(g)
                out.append(f"{y:>5} " + " ".join(row))
            out.append("")
    elif view == "side":
        planes = [at] if at is not None else list(xs)
        amap = SIDE
        for x in planes:
            out.append(f"side view x={x} (looking east): z {zs.start}..{zs.stop - 1} ->, y downward from {ys.start}")
            for y in ys:
                row = []
                for z in zs:
                    blk = world[(x, y, z)]
                    g = _view_glyph(blk, amap)
                    if g != ". ":
                        used.setdefault(g, blk.state.replace("minecraft:", ""))
                    row.append(g)
                out.append(f"{y:>5} " + " ".join(row))
            out.append("")
    else:
        raise ValueError("view must be 'top', 'front' or 'side'")
    if legend and used:
        out.append("legend (arrows: ^ up, v down, < > along the screen, x away from you, o toward you):")
        for g, st in sorted(used.items()):
            out.append(f"  {g!r:6} {st}")
    return "\n".join(out)


def _view_glyph(blk, amap):
    g = _glyph(blk)
    if blk.name == "redstone_wire":
        return g
    return "".join(amap.get(c, c) if c in "^v<>ud" else c for c in g)
