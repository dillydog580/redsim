"""Every saved scenario must reproduce the real game's recorded output tick for tick.

Scenarios live in tests/regressions/ (minimized fuzz finds) and tests/known/ (hand-made
builds). Each file stores the game's per-tick output, so this runs without Minecraft.
Set REDSIM_ORACLE=1 to re-check against the live game instead.
"""
import json
import os
from pathlib import Path

import pytest

from redsim.difftest import Scenario, compare, run_sim

HERE = Path(__file__).parent
FILES = sorted(list((HERE / "regressions").glob("*.json")) + list((HERE / "known").glob("*.json")))


def load(p):
    d = json.loads(p.read_text())
    s = Scenario(blocks=[tuple(b) for b in d["blocks"]], size=tuple(d["size"]), ticks=d["ticks"], actions=d["actions"],
                 block_entities=d.get("block_entities", []), offset=tuple(d["offset"]) if d.get("offset") else None, name=d.get("name", ""),
                 day_time=d.get("day_time", 0))
    return s, d.get("expected")


@pytest.fixture(scope="session")
def oracle():
    if not os.environ.get("REDSIM_ORACLE"):
        yield None
        return
    from redsim.oracle import Oracle
    with Oracle() as o:
        yield o


@pytest.mark.parametrize("path", FILES, ids=[p.stem for p in FILES])
def test_matches_game(path, oracle):
    scen, expected = load(path)
    if oracle is not None:
        oracle.set_day_time(scen.day_time)
        res = oracle.request(scen.to_request())
        expected = {"origin": res["origin"], "start_time": res["start_time"], "steps": res["steps"]}
    if expected is None:
        pytest.skip("no recorded game output (run with REDSIM_ORACLE=1)")
    sim = run_sim(scen, expected["origin"], expected["start_time"])
    d = compare(expected["steps"], sim["steps"])
    assert d is None, d.describe()
