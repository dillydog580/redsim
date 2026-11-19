"""Schematic formats: Litematica (.litematic), Sponge/WorldEdit (.schem v2/v3) and
vanilla structure files (.nbt). All load into / save from a `Build`.

A Build is a box of blocks with coordinates relative to its minimum corner, plus
block entity data (container contents, comparator output) and optional pending ticks.
"""

from __future__ import annotations

import math
import time
from dataclasses import dataclass, field
from pathlib import Path

from . import nbt
from . import _redsim

AIR = "minecraft:air"


@dataclass
class Build:
    size: tuple[int, int, int]
    blocks: dict = field(default_factory=dict)          # (x, y, z) -> "minecraft:block[props]" (non-air only)
    block_entities: dict = field(default_factory=dict)  # (x, y, z) -> dict (NBT-like, plain Python values)
    pending_ticks: list = field(default_factory=list)   # [{"pos": (x,y,z), "block": name, "delay": int, "priority": int}]
    name: str = "redsim build"
    author: str = "redsim"
    description: str = ""

    def __len__(self):
        return len(self.blocks)

    def bounds(self):
        if not self.blocks:
            return (0, 0, 0), (0, 0, 0)
        xs, ys, zs = zip(*self.blocks)
        return (min(xs), min(ys), min(zs)), (max(xs), max(ys), max(zs))

    def save(self, path, **kw):
        save(self, path, **kw)

    def _remap(self, fpos, which):
        tf = (lambda st: _redsim.format_state(_redsim.state_transform(_redsim.parse_state(st), which))) if which is not None else (lambda st: st)
        nb = {fpos(p): tf(st) for p, st in self.blocks.items()}
        nbe = {fpos(p): d for p, d in self.block_entities.items()}
        nt = [dict(t, pos=fpos(t["pos"])) for t in self.pending_ticks]
        allp = list(nb) + list(nbe)
        mn = [min(p[i] for p in allp) for i in range(3)] if allp else [0, 0, 0]
        sh = lambda p: (p[0] - mn[0], p[1] - mn[1], p[2] - mn[2])
        out = Build(size=self.size, name=self.name, author=self.author, description=self.description)
        out.blocks = {sh(p): st for p, st in nb.items()}
        out.block_entities = {sh(p): d for p, d in nbe.items()}
        out.pending_ticks = [dict(t, pos=sh(t["pos"])) for t in nt]
        return out

    def rotate(self, turns: int = 1) -> "Build":
        """Rotate clockwise (seen from above) by 90 degrees `turns` times, like a structure block:
        positions turn and every block state turns with the game's own rotate() (north -> east)."""
        turns %= 4
        if turns == 0:
            return self._remap(lambda p: tuple(p), None)
        sx, sy, sz = self.size
        f = {1: lambda p: (-p[2], p[1], p[0]), 2: lambda p: (-p[0], p[1], -p[2]), 3: lambda p: (p[2], p[1], -p[0])}[turns]
        out = self._remap(f, turns - 1)
        out.size = (sz, sy, sx) if turns % 2 else (sx, sy, sz)
        return out

    def mirror(self, axis: str = "x") -> "Build":
        """Mirror across a plane: axis='x' flips east/west (x), axis='z' flips north/south (z), with
        every block state mirrored by the game's own mirror()."""
        if axis == "x":
            return self._remap(lambda p: (-p[0], p[1], p[2]), 4)
        if axis == "z":
            return self._remap(lambda p: (p[0], p[1], -p[2]), 3)
        raise ValueError("axis must be 'x' or 'z'")


# ---------------------------------------------------------------------------- helpers

def _state_from_palette(entry: dict) -> str:
    # 26.x (data version >= 5006) uses "id"/"properties"; older files "Name"/"Properties"
    name = entry.get("id") or entry.get("Name") or entry.get("")
    props = entry.get("properties") or entry.get("Properties") or {}
    if not props:
        return name
    return name + "[" + ",".join(f"{k}={v}" for k, v in sorted(props.items())) + "]"


def _normalize(state: str) -> str:
    """Canonical game form (all properties, game order) via the block table."""
    try:
        return _redsim.format_state(_redsim.parse_state(state))
    except ValueError:
        return state


def _palette_entry(state: str) -> dict:
    """Both key styles: 26.x readers use id/properties, older tools Name/Properties."""
    if "[" in state:
        name, rest = state.split("[", 1)
        props = dict(kv.split("=", 1) for kv in rest[:-1].split(",") if kv)
        return {"id": name, "properties": props, "Name": name, "Properties": dict(props)}
    return {"id": state, "Name": state}


def _be_plain(tag: dict) -> dict:
    return nbt.unwrap(tag)


def _be_to_nbt(data: dict) -> dict:
    """Plain dict -> NBT with sensible tag types for known keys."""
    out = {}
    for k, v in data.items():
        if k == "Items":
            items = []
            for it in v:
                e = {"Slot": nbt.Byte(it["Slot"]), "id": it["id"], "count": nbt.Int(it.get("count", it.get("Count", 1)))}
                for extra in ("components",):
                    if extra in it:
                        e[extra] = it[extra]
                items.append(e)
            out[k] = items
        elif k == "OutputSignal":
            out[k] = nbt.Int(v)
        elif isinstance(v, bool):
            out[k] = nbt.Byte(v)
        else:
            out[k] = v
    return out


def _data_version() -> int:
    return _redsim.data_version()


# ---------------------------------------------------------------------------- litematica

def _lit_bits(palette_len: int) -> int:
    return max(2, (palette_len - 1).bit_length())


def _lit_unpack(longs, n, bits):
    mask = (1 << bits) - 1
    arr = [x & 0xFFFFFFFFFFFFFFFF for x in longs]
    out = [0] * n
    for i in range(n):
        start = i * bits
        si = start >> 6
        ei = ((i + 1) * bits - 1) >> 6
        off = start & 63
        if si == ei:
            out[i] = (arr[si] >> off) & mask
        else:
            out[i] = ((arr[si] >> off) | (arr[ei] << (64 - off))) & mask
    return out


def _lit_pack(values, bits):
    n = len(values)
    nlongs = (n * bits + 63) // 64
    arr = [0] * nlongs
    for i, v in enumerate(values):
        start = i * bits
        si = start >> 6
        ei = ((i + 1) * bits - 1) >> 6
        off = start & 63
        arr[si] |= (v << off) & 0xFFFFFFFFFFFFFFFF
        if si != ei:
            arr[ei] |= v >> (64 - off)
    return [x - (1 << 64) if x >= (1 << 63) else x for x in arr]


def load_litematic(path) -> Build:
    _, root = nbt.load(path)
    regions = root["Regions"]
    meta = root.get("Metadata", {})
    parsed = []
    for rname, r in regions.items():
        px, py, pz = (int(r["Position"][k]) for k in "xyz")
        sx, sy, sz = (int(r["Size"][k]) for k in "xyz")
        x0 = px + (sx + 1 if sx < 0 else 0)
        y0 = py + (sy + 1 if sy < 0 else 0)
        z0 = pz + (sz + 1 if sz < 0 else 0)
        ax, ay, az = abs(sx), abs(sy), abs(sz)
        palette = [_state_from_palette(nbt.unwrap(e)) for e in r["BlockStatePalette"]]
        bits = _lit_bits(len(palette))
        vals = _lit_unpack(nbt.unwrap(r["BlockStates"]), ax * ay * az, bits)
        parsed.append((x0, y0, z0, ax, ay, az, palette, vals, r))
    mx = min(p[0] for p in parsed)
    my = min(p[1] for p in parsed)
    mz = min(p[2] for p in parsed)
    Mx = max(p[0] + p[3] for p in parsed)
    My = max(p[1] + p[4] for p in parsed)
    Mz = max(p[2] + p[5] for p in parsed)
    b = Build(size=(Mx - mx, My - my, Mz - mz), name=str(nbt.unwrap(meta.get("Name", "litematic"))),
              author=str(nbt.unwrap(meta.get("Author", ""))), description=str(nbt.unwrap(meta.get("Description", ""))))
    for (x0, y0, z0, ax, ay, az, palette, vals, r) in parsed:
        ox, oy, oz = x0 - mx, y0 - my, z0 - mz
        layer = ax * az
        for i, v in enumerate(vals):
            st = palette[v]
            if st == AIR or st in ("minecraft:cave_air", "minecraft:void_air"):
                continue
            y, rem = divmod(i, layer)
            z, x = divmod(rem, ax)
            b.blocks[(x + ox, y + oy, z + oz)] = _normalize(st)
        for te in r.get("TileEntities", []):
            te = _be_plain(te)
            pos = (te.pop("x") + ox, te.pop("y") + oy, te.pop("z") + oz)
            b.block_entities[pos] = te
        for t in r.get("PendingBlockTicks", []):
            t = nbt.unwrap(t)
            b.pending_ticks.append({"pos": (t["x"] + ox, t["y"] + oy, t["z"] + oz), "block": t["Block"],
                                    "delay": int(t.get("Time", 1)), "priority": int(t.get("Priority", 0))})
    return b


def save_litematic(b: Build, path, include_air: bool = True):
    sx, sy, sz = b.size
    palette = [AIR]
    index = {AIR: 0}
    vals = []
    for y in range(sy):
        for z in range(sz):
            for x in range(sx):
                st = b.blocks.get((x, y, z), AIR)
                if st not in index:
                    index[st] = len(palette)
                    palette.append(st)
                vals.append(index[st])
    bits = _lit_bits(len(palette))
    now = int(time.time() * 1000)
    tes = []
    for (x, y, z), data in sorted(b.block_entities.items()):
        e = _be_to_nbt(data)
        e.update({"x": nbt.Int(x), "y": nbt.Int(y), "z": nbt.Int(z)})
        tes.append(e)
    ticks = [{"Block": t["block"], "Priority": nbt.Int(t.get("priority", 0)), "SubTick": nbt.Long(i),
              "Time": nbt.Int(t["delay"]), "x": nbt.Int(t["pos"][0]), "y": nbt.Int(t["pos"][1]), "z": nbt.Int(t["pos"][2])}
             for i, t in enumerate(b.pending_ticks)]
    region = {
        "Position": {"x": nbt.Int(0), "y": nbt.Int(0), "z": nbt.Int(0)},
        "Size": {"x": nbt.Int(sx), "y": nbt.Int(sy), "z": nbt.Int(sz)},
        "BlockStatePalette": [_palette_entry(s) for s in palette],
        "BlockStates": nbt.LongArray(_lit_pack(vals, bits)),
        "TileEntities": tes,
        "Entities": [],
        "PendingBlockTicks": ticks,
        "PendingFluidTicks": [],
    }
    root = {
        "MinecraftDataVersion": nbt.Int(_data_version()),
        "Version": nbt.Int(6),
        "SubVersion": nbt.Int(1),
        "Metadata": {
            "Name": b.name, "Author": b.author, "Description": b.description,
            "RegionCount": nbt.Int(1), "TotalBlocks": nbt.Int(len(b.blocks)), "TotalVolume": nbt.Int(sx * sy * sz),
            "EnclosingSize": {"x": nbt.Int(sx), "y": nbt.Int(sy), "z": nbt.Int(sz)},
            "TimeCreated": nbt.Long(now), "TimeModified": nbt.Long(now),
        },
        "Regions": {b.name or "build": region},
    }
    nbt.dump(root, path)


# ---------------------------------------------------------------------------- sponge .schem

def _varints(data):
    out, i = [], 0
    data = [x & 0xFF for x in data]
    while i < len(data):
        v, shift = 0, 0
        while True:
            byte = data[i]
            i += 1
            v |= (byte & 0x7F) << shift
            if not byte & 0x80:
                break
            shift += 7
        out.append(v)
    return out


def _to_varints(vals):
    out = []
    for v in vals:
        while True:
            if v & ~0x7F:
                out.append((v & 0x7F) | 0x80)
                v >>= 7
            else:
                out.append(v)
                break
    return [x - 256 if x > 127 else x for x in out]


def load_schem(path) -> Build:
    _, root = nbt.load(path)
    if "Schematic" in root:  # v3 nests everything under "Schematic"
        root = root["Schematic"]
    version = int(nbt.unwrap(root.get("Version", 2)))
    w, h, l = (int(nbt.unwrap(root[k])) for k in ("Width", "Height", "Length"))
    if version >= 3:
        blocks = root["Blocks"]
        palette_tag = blocks["Palette"]
        data = nbt.unwrap(blocks["Data"])
        bes = blocks.get("BlockEntities", [])
    else:
        palette_tag = root["Palette"]
        data = nbt.unwrap(root["BlockData"])
        bes = root.get("BlockEntities", [])
    palette = {int(nbt.unwrap(v)): k for k, v in palette_tag.items()}
    vals = _varints(data)
    b = Build(size=(w, h, l), name=Path(path).stem)
    for i, v in enumerate(vals):
        st = palette[v]
        if st.split("[")[0] in (AIR, "minecraft:cave_air", "minecraft:void_air"):
            continue
        y, rem = divmod(i, w * l)
        z, x = divmod(rem, w)
        b.blocks[(x, y, z)] = _normalize(st)
    for be in bes:
        be = _be_plain(be)
        x, y, z = be.pop("Pos")
        payload = be.pop("Data", None) if version >= 3 else None
        data_ = dict(payload) if payload is not None else be
        data_.pop("Id", None)
        b.block_entities[(x, y, z)] = data_
    return b


def save_schem(b: Build, path, version: int = 3):
    w, h, l = b.size
    palette = {AIR: 0}
    vals = []
    for y in range(h):
        for z in range(l):
            for x in range(w):
                st = b.blocks.get((x, y, z), AIR)
                if st not in palette:
                    palette[st] = len(palette)
                vals.append(palette[st])
    pal_tag = {k: nbt.Int(v) for k, v in palette.items()}
    data = nbt.ByteArray(_to_varints(vals))
    bes = []
    for (x, y, z), d in sorted(b.block_entities.items()):
        st = b.blocks.get((x, y, z), AIR)
        be_id = _redsim.block_entity_type(_redsim.parse_state(st)) if st != AIR else ""
        if version >= 3:
            bes.append({"Pos": nbt.IntArray([x, y, z]), "Id": be_id, "Data": _be_to_nbt(d)})
        else:
            e = _be_to_nbt(d)
            e.update({"Pos": nbt.IntArray([x, y, z]), "Id": be_id})
            bes.append(e)
    common = {"Version": nbt.Int(version), "DataVersion": nbt.Int(_data_version()), "Width": nbt.Short(w),
              "Height": nbt.Short(h), "Length": nbt.Short(l), "Offset": nbt.IntArray([0, 0, 0])}
    if version >= 3:
        root = {"Schematic": dict(common, Blocks={"Palette": pal_tag, "Data": data, "BlockEntities": bes},
                                  Metadata={"Name": b.name, "Author": b.author})}
    else:
        root = dict(common, PaletteMax=nbt.Int(len(palette)), Palette=pal_tag, BlockData=data, BlockEntities=bes,
                    Metadata={"Name": b.name, "Author": b.author})
    nbt.dump(root, path, name="" if version >= 3 else "Schematic")


# ---------------------------------------------------------------------------- vanilla structure .nbt

def load_structure(path) -> Build:
    _, root = nbt.load(path)
    size = tuple(int(v) for v in nbt.unwrap(root["size"]))
    palette = root.get("palette") or root["palettes"][0]
    palette = [_state_from_palette(nbt.unwrap(e)) for e in palette]
    b = Build(size=size, name=Path(path).stem)
    for e in root["blocks"]:
        e = nbt.unwrap(e)
        st = palette[e["state"]]
        pos = tuple(e["pos"])
        if st.split("[")[0] in (AIR, "minecraft:cave_air", "minecraft:void_air", "minecraft:structure_void"):
            continue
        b.blocks[pos] = _normalize(st)
        if "nbt" in e:
            d = dict(e["nbt"])
            d.pop("id", None)
            b.block_entities[pos] = d
    return b


def save_structure(b: Build, path, include_air: bool = True):
    sx, sy, sz = b.size
    palette = []
    index = {}
    blocks = []
    for y in range(sy):
        for z in range(sz):
            for x in range(sx):
                st = b.blocks.get((x, y, z), AIR)
                if st == AIR and not include_air:
                    continue
                if st not in index:
                    index[st] = len(palette)
                    palette.append(st)
                e = {"pos": [nbt.Int(x), nbt.Int(y), nbt.Int(z)], "state": nbt.Int(index[st])}
                if (x, y, z) in b.block_entities:
                    d = _be_to_nbt(b.block_entities[(x, y, z)])
                    d["id"] = _redsim.block_entity_type(_redsim.parse_state(st))
                    e["nbt"] = d
                blocks.append(e)
    root = {"DataVersion": nbt.Int(_data_version()), "size": [nbt.Int(sx), nbt.Int(sy), nbt.Int(sz)],
            "palette": [_palette_entry(s) for s in palette], "blocks": blocks, "entities": []}
    nbt.dump(root, path)


# ---------------------------------------------------------------------------- dispatch

def load(path) -> Build:
    p = str(path).lower()
    if p.endswith(".litematic"):
        return load_litematic(path)
    if p.endswith(".schem") or p.endswith(".schematic"):
        return load_schem(path)
    if p.endswith(".nbt"):
        return load_structure(path)
    raise ValueError(f"unknown schematic type: {path} (use .litematic, .schem or .nbt)")


def save(b: Build, path, **kw):
    p = str(path).lower()
    if p.endswith(".litematic"):
        return save_litematic(b, path, **kw)
    if p.endswith(".schem"):
        return save_schem(b, path, **kw)
    if p.endswith(".nbt"):
        return save_structure(b, path, **kw)
    raise ValueError(f"unknown schematic type: {path} (use .litematic, .schem or .nbt)")
