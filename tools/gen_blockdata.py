"""Generate the simulator's block tables from data extracted from the real game.

Input:  data/blocks-<version>.json   (written by the oracle's `extract` op)
Output: crates/redsim-core/data/blocks.json  (compact runtime table, embedded in the binary)
        crates/redsim-core/src/gen.rs        (Rust constants for blocks, properties, tags)

These are facts about the game (ids, property lists, predicate results), not game code.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

FLAG_KEYS = ["conductor", "signal_source", "analog", "air", "replaceable", "has_be", "solid",
             "full_collision", "solid_render", "ticks_randomly", "collision_empty", "shape_full"]
PUSH = ["PUSH_PULL", "PUSH", "POPPED", "IMMOVEABLE", "IGNORE_ENTITY"]
HOOKS = ["updateShape", "neighborChanged", "tick", "randomTick", "onPlace", "affectNeighborsAfterRemoval", "getSignal",
         "getDirectSignal", "isSignalSource", "ownSignal", "hasAnalogOutputSignal", "getAnalogOutputSignal", "triggerEvent",
         "useWithoutItem", "useItemOn", "getTicker", "newBlockEntity", "entityInside", "stepOn", "onProjectileHit",
         "updateIndirectNeighbourShapes", "canSurvive", "shouldRedstoneWireConnectTo", "getStateForPlacement", "setPlacedBy",
         "playerWillDestroy", "onExplosionHit", "attack", "fallOn", "updateEntityMovementAfterFallOn", "isRandomlyTicking",
         "getCollisionShape", "getBlockSupportShape"]


def ident(s: str) -> str:
    s = re.sub(r"[^A-Za-z0-9]", "_", s).upper()
    if s[0].isdigit():
        s = "_" + s
    return s


def main(version: str = "26.3"):
    src = json.load(open(ROOT / "data" / f"blocks-{version}.json"))
    blocks = src["blocks"]

    props: dict[str, list[str]] = {}
    tags: list[str] = sorted({t for b in blocks for t in b["tags"]})
    tag_index = {t: i for i, t in enumerate(tags)}
    fluids: list[str] = [""]

    out_blocks = []
    states_out = []
    transforms = []
    dampening = []
    face_occ = []
    for b in blocks:
        bprops = []
        for p in b["properties"]:
            field = p["field"] or (b["name"] + "." + p["name"])
            if field in props and props[field] != p["values"]:
                raise SystemExit(f"property {field} has differing values")
            props.setdefault(field, p["values"])
            bprops.append(field)
        ids = [s["id"] for s in b["states"]]
        assert ids == list(range(ids[0], ids[0] + len(ids)))
        for s in b["states"]:
            flags = 0
            for i, k in enumerate(FLAG_KEYS):
                if s[k]:
                    flags |= 1 << i
            fl = s["fluid"]
            if fl not in fluids:
                fluids.append(fl)
            transforms.append(s.get("transform", [s["id"]] * 5))
            dampening.append("%x" % s["light_dampening"])
            if any(s["face_occ"]):
                face_occ.append([s["id"]] + s["face_occ"])
            states_out.append([flags, s["sturdy_full"], s["sturdy_center"], s["sturdy_rigid"], PUSH.index(s["push"]),
                               s["destroy_speed"], fluids.index(fl), s["light"], s["collision"]])
        hooks = 0
        for h in b["hooks"]:
            hooks |= 1 << HOOKS.index(h)
        out_blocks.append({
            "name": b["name"], "first": ids[0], "n": len(ids), "default": b["default"], "props": bprops,
            "classes": b["classes"], "hooks": hooks, "tags": [tag_index[t] for t in b["tags"]],
            "fields": dict(b["fields"], instrument=b["instrument"], instrument_above=b["instrument_above"],
                           container_size=b.get("container_size"), container_max_stack=b.get("container_max_stack"),
                           block_entity=b.get("block_entity")), "dyn": b["has_dynamic_shape"],
        })

    prop_names = list(props.keys())
    data = {
        "version": src["version"], "data_version": src["data_version"],
        "props": [{"field": f, "name": f.split(".")[-1].lower() if "." in f else None, "values": props[f]} for f in prop_names],
        "tags": tags, "fluids": fluids, "flag_keys": FLAG_KEYS, "push": PUSH, "hooks": HOOKS,
        "blocks": out_blocks, "states": states_out, "collision_shapes": src["collision_shapes"], "item_max_stack": src["item_max_stack"], "transforms": transforms, "jukebox_songs": src.get("jukebox_songs", {}),
        "light_dampening": "".join(dampening), "face_occ": face_occ, "face_shape_count": src["face_shape_count"],
        "face_occludes": src["face_occludes"], "daylight": src["daylight"],
    }
    # property 'name' (the in-game key, e.g. "facing") from the source data
    pname = {}
    for b in blocks:
        for p in b["properties"]:
            pname[p["field"] or (b["name"] + "." + p["name"])] = p["name"]
    for p in data["props"]:
        p["name"] = pname[p["field"]]

    out_dir = ROOT / "crates" / "redsim-core" / "data"
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "blocks.json").write_text(json.dumps(data, separators=(",", ":")))

    lines = ["// @generated by tools/gen_blockdata.py from data extracted from Minecraft "
             f"{src['version']}. Do not edit.", "#![allow(dead_code)]", "",
             f"pub const MC_VERSION: &str = \"{src['version']}\";",
             f"pub const DATA_VERSION: i32 = {src['data_version']};",
             f"pub const NUM_STATES: usize = {len(states_out)};",
             f"pub const NUM_BLOCKS: usize = {len(out_blocks)};", ""]
    lines.append("/// Block ids (index into the block table, same order as the game's block registry).")
    lines.append("pub mod blk {")
    for i, b in enumerate(out_blocks):
        lines.append(f"    pub const {ident(b['name'].split(':', 1)[1])}: u16 = {i};")
    lines.append("}")
    lines.append("")
    lines.append("/// Block state property ids, named after the game's BlockStateProperties fields.")
    lines.append("pub mod prop {")
    for i, f in enumerate(prop_names):
        lines.append(f"    pub const {ident(f.split('.')[-1] if f.count('.') == 1 and f.split('.')[0][0].isupper() else f)}: u16 = {i};")
    lines.append("}")
    lines.append("")
    lines.append("/// Block tag ids.")
    lines.append("pub mod tag {")
    for i, t in enumerate(tags):
        lines.append(f"    pub const {ident(t.split(':', 1)[1])}: u16 = {i};")
    lines.append("}")
    lines.append("")
    lines.append("/// Default state ids.")
    lines.append("pub mod st {")
    for b in out_blocks:
        lines.append(f"    pub const {ident(b['name'].split(':', 1)[1])}: u32 = {b['default']};")
    lines.append("}")
    lines.append("")
    gen = "\n".join(lines) + "\n"
    names = [l.split()[2] for l in lines if l.strip().startswith("pub const") and "prop" not in l]
    (ROOT / "crates" / "redsim-core" / "src" / "gen.rs").write_text(gen)
    print(f"{len(out_blocks)} blocks, {len(states_out)} states, {len(prop_names)} props, {len(tags)} tags")


if __name__ == "__main__":
    main(*sys.argv[1:])
