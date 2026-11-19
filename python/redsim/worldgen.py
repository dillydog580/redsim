"""Make a Minecraft world save containing a build, using the real game (oracle) so the
save is exactly what the game writes. Open it in Singleplayer to look at / play with it."""

from __future__ import annotations

import os
import platform
import shutil
from pathlib import Path

from . import nbt
from .schematic import load as load_schematic


def default_saves_dir() -> Path:
    home = Path.home()
    system = platform.system()
    if system == "Darwin":
        return home / "Library" / "Application Support" / "minecraft" / "saves"
    if system == "Windows":
        return Path(os.environ.get("APPDATA", home)) / ".minecraft" / "saves"
    return home / ".minecraft" / "saves"


def make_world(build_path, name: str, origin=(0, -60, 0), saves=None) -> Path:
    from .oracle import Oracle
    b = load_schematic(build_path)
    saves = Path(saves) if saves else default_saves_dir()
    saves.mkdir(parents=True, exist_ok=True)
    safe = "".join(c if c.isalnum() or c in " -_" else "_" for c in name).strip() or "redsim"
    dest = saves / safe
    n = 1
    while dest.exists():
        n += 1
        dest = saves / f"{safe} ({n})"
    blocks = [[x, y, z, s] for (x, y, z), s in sorted(b.blocks.items(), key=lambda kv: (kv[0][1], kv[0][2], kv[0][0]))]
    bes = []
    for p, d in b.block_entities.items():
        e = {"pos": list(p)}
        if "Items" in d:
            e["items"] = [[it["Slot"], it["id"], it.get("count", 1)] for it in d["Items"]]
        if "OutputSignal" in d:
            e["output"] = d["OutputSignal"]
        bes.append(e)
    with Oracle(slots=False, heap="2g") as o:
        o.request({"op": "save_world", "blocks": blocks, "block_entities": bes, "origin": list(origin),
                   "size": list(b.size), "out": str(dest)})
    # name the world
    lvl = dest / "level.dat"
    rootname, root = nbt.load(lvl)
    root["Data"]["LevelName"] = name
    nbt.dump(root, lvl, name=rootname)
    return dest
