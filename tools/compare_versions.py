"""Check that two redsim builds behave identically, write for write.

Speed work must not change behaviour, so this compares a build against an older one on a fixed
corpus: dense random builds from the fuzzer's generator (all components, plus a pass focused on
each component), the recorded test builds at their recorded positions, and long benchmark runs.
For every run it hashes:

  * the full causal trace: every setBlock, neighbour and shape update, scheduled tick and block
    event, in order, with its cause;
  * the ordered change log, per-tick snapshots of the region and its block entities, warnings,
    and the scheduled-tick and block-event queues at the end (untraced run);
  * that the traced and untraced runs saw the same thing.

    python tools/compare_versions.py make-corpus corpus.json --count 4000
    python tools/compare_versions.py digest corpus.json a.json     # with build A installed
    python tools/compare_versions.py digest corpus.json b.json     # with build B installed
    python tools/compare_versions.py diff a.json b.json

The corpus is generated once so that both builds run exactly the same scenarios.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import random
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def _h(obj) -> str:
    return hashlib.sha256(json.dumps(obj, sort_keys=True, default=str).encode()).hexdigest()[:24]


def make_corpus(path, count, seed):
    from redsim import fuzz
    from redsim.difftest import Scenario

    rng = random.Random(seed)
    out = []

    def origin():
        r = rng.random()
        if r < 0.5:
            span = 200
        elif r < 0.9:
            span = 100_000
        else:
            span = 29_000_000
        return [rng.randrange(-span, span), rng.randrange(-60, 250), rng.randrange(-span, span)]

    def add(s: Scenario, o, start_time, label):
        d = json.loads(s.to_json())
        d.update(origin=list(o), start_time=start_time, label=label)
        out.append(d)

    base_components = list(fuzz.COMPONENTS)
    base_solids = list(fuzz.SOLIDS)
    sizes = [(7, 5, 7), (7, 5, 7), (10, 5, 10), (14, 4, 14), (5, 8, 5)]
    for i in range(count):
        size = sizes[i % len(sizes)]
        ticks = rng.choice([None, None, 60, 120])
        day = rng.choice(fuzz.DAY_TIMES + [rng.randrange(24000)])
        s = fuzz.random_scenario(rng, size, ticks=ticks, day_time=day)
        add(s, origin(), rng.randrange(0, 1_000_000), f"random-{i}")
    # one focused pass per component, so rare ones get dense coverage too
    names = [n for n, _ in base_components]
    for k, name in enumerate(names):
        fuzz.COMPONENTS[:] = base_components
        fuzz.SOLIDS[:] = base_solids
        fuzz.focus([name])
        for j in range(12):
            day = rng.choice(fuzz.DAY_TIMES)
            s = fuzz.random_scenario(rng, (7, 5, 7), day_time=day)
            add(s, origin(), rng.randrange(0, 1_000_000), f"focus-{name}-{j}")
    fuzz.COMPONENTS[:] = base_components
    fuzz.SOLIDS[:] = base_solids
    # recorded builds at the game's own positions and times
    for d in sorted(list((ROOT / "tests").glob("*/*.json"))):
        rec = json.loads(d.read_text())
        exp = rec.get("expected") or {}
        if "blocks" not in rec:
            continue
        s = Scenario(blocks=[tuple(b) for b in rec["blocks"]], size=tuple(rec["size"]), ticks=rec["ticks"],
                     actions=rec["actions"], block_entities=rec.get("block_entities", []),
                     offset=tuple(rec["offset"]) if rec.get("offset") else None, day_time=rec.get("day_time", 0))
        o = exp.get("origin") or origin()
        add(s, o, exp.get("start_time", 0), f"recorded-{d.parent.name}/{d.stem}")
    # long runs of the benchmark builds (also exercise skipping of quiet stretches)
    out.extend({"bench": b, "label": f"bench-{b}"} for b in ("observer_clock", "piston_row", "dust_grid", "piston_row_far"))
    out.extend({"bench": b, "label": f"suite-{b}"} for b in _suite().SUITE)
    Path(path).write_text(json.dumps(out))
    print(f"wrote {len(out)} scenarios to {path}")


def _suite():
    """The benchmark builds from this checkout's python/redsim/bench.py, run against whichever
    redsim is installed (they use only the public World API)."""
    import importlib.util

    spec = importlib.util.spec_from_file_location("redsim_bench_suite", ROOT / "python" / "redsim" / "bench.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def _bench_digest(name):
    import redsim

    bench = _suite()
    if name in bench.SUITE:
        w, drive = bench.build(name)
        nw = w._w
        nw.record_changes(True)
        drive(w, 3000)
        changes = nw.take_changes()
        bes = sorted((p, nw.block_entity(*p)) for p in nw.block_entity_positions())
        return {"changes": _h(changes), "n_changes": len(changes), "blocks": _h(sorted(nw.blocks())),
                "pending": _h(nw.pending_ticks()), "warnings": _h(nw.warnings()), "be": _h(bes)}
    w = redsim.World(trace=False)
    if name == "observer_clock":
        bench.observer_clock(w)
        ticks, toggle = 4000, None
    elif name == "piston_row":
        bench.piston_row(w)
        ticks, toggle = 1500, None
    elif name == "piston_row_far":
        bench.piston_row(w, n=12, ox=-1_234_567, oz=7_654_321)
        ticks, toggle = 800, None
    else:
        bench.dust_grid(w, n=20)
        ticks, toggle = 400, (-1, 1, 0)
    nw = w._w
    nw.record_changes(True)
    for t in range(ticks):
        if toggle and t % 20 == 0:
            nw.use_block(*toggle)
        nw.tick(1)
    changes = nw.take_changes()
    return {"changes": _h(changes), "n_changes": len(changes), "blocks": _h(sorted(nw.blocks())),
            "pending": _h(nw.pending_ticks()), "warnings": _h(nw.warnings())}


def _run(d, traced):
    from redsim import _redsim
    from redsim.difftest import Scenario, run_sim

    s = Scenario.from_json(json.dumps(d))
    w = _redsim.World()
    if traced:
        w.enable_trace(50_000_000)
    else:
        w.record_changes(True)
    res = run_sim(s, tuple(d["origin"]), d["start_time"], world=w)
    steps = res["steps"]
    for st in steps:  # block-entity dict keys are tuples; make them JSON-stable
        for k in ("be", "act_be"):
            if k in st:
                st[k] = sorted(st[k], key=lambda e: (e[0], e[1], e[2]))
    out = {"steps": _h(steps), "warnings": _h(w.warnings()), "pending": _h(w.pending_ticks()),
           "events": _h(w.pending_block_events()), "blocks": _h(sorted(w.blocks())),
           "be": _h(sorted((p, w.block_entity(*p)) for p in w.block_entity_positions())), "idle": w.is_idle()}
    if traced:
        out["trace"] = _h(w.trace_json())
        out["trace_len"] = w.trace_len()
    else:
        ch = w.take_changes()
        out["changes"] = _h(ch)
        out["n_changes"] = len(ch)
    return out


def digest(corpus, path):
    items = json.loads(Path(corpus).read_text())
    out = {}
    t0 = time.time()
    for i, d in enumerate(items):
        label = d["label"]
        if "bench" in d:
            out[label] = _bench_digest(d["bench"])
            continue
        plain = _run(d, traced=False)
        traced = _run(d, traced=True)
        same = all(plain[k] == traced[k] for k in ("steps", "warnings", "pending", "events", "blocks", "be"))
        out[label] = {"plain": plain, "traced": traced, "trace_matches_plain": same}
        if (i + 1) % 500 == 0:
            print(f"  {i + 1}/{len(items)} ({time.time() - t0:.0f}s)", flush=True)
    Path(path).write_text(json.dumps(out, indent=0, sort_keys=True))
    bad = [k for k, v in out.items() if isinstance(v, dict) and v.get("trace_matches_plain") is False]
    print(f"digested {len(out)} runs in {time.time() - t0:.0f}s; traced/untraced mismatches: {len(bad)}")
    for k in bad[:10]:
        print("  traced and untraced runs differ:", k)


def diff(a, b):
    A = json.loads(Path(a).read_text())
    B = json.loads(Path(b).read_text())
    keys = sorted(set(A) | set(B))
    bad = [k for k in keys if A.get(k) != B.get(k)]
    total_trace = sum(v["traced"]["trace_len"] for v in A.values() if "traced" in v)
    total_changes = sum(v["plain"]["n_changes"] if "plain" in v else v["n_changes"] for v in A.values())
    print(f"{len(keys)} runs compared ({total_trace:,} trace events, {total_changes:,} block changes in A)")
    if not bad:
        print("IDENTICAL: every run matches write for write")
        return 0
    print(f"DIFFERENT: {len(bad)} runs differ")
    for k in bad[:20]:
        va, vb = A.get(k), B.get(k)
        parts = []
        if isinstance(va, dict) and isinstance(vb, dict):
            for sub in sorted(set(va) | set(vb)):
                if va.get(sub) != vb.get(sub):
                    if isinstance(va.get(sub), dict):
                        parts.append(f"{sub}: " + ",".join(x for x in va[sub] if va[sub].get(x) != (vb.get(sub) or {}).get(x)))
                    else:
                        parts.append(sub)
        print(f"  {k}: {'; '.join(parts) or 'missing'}")
    return 1


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("make-corpus")
    p.add_argument("out")
    p.add_argument("--count", type=int, default=4000)
    p.add_argument("--seed", type=int, default=20261117)
    p = sub.add_parser("digest")
    p.add_argument("corpus")
    p.add_argument("out")
    p = sub.add_parser("diff")
    p.add_argument("a")
    p.add_argument("b")
    a = ap.parse_args(argv)
    if a.cmd == "make-corpus":
        make_corpus(a.out, a.count, a.seed)
    elif a.cmd == "digest":
        digest(a.corpus, a.out)
    else:
        return diff(a.a, a.b)
    return 0


if __name__ == "__main__":
    sys.exit(main())
