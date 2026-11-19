"""Client for the redsim oracle: the real Minecraft 26.3 game code, run headless.

The oracle is a Java process (``oracle/``) that runs a minimal Minecraft server
with no networking and ticks as fast as it can. It places a build, applies
actions between ticks and reports every block-state change per tick. It is the
ground truth that the simulator is tested against.

Requires the game jar and a JDK 25 in ``.cache/`` (see ``tools/setup.py``).
"""

from __future__ import annotations

import json
import os
import subprocess
import threading
import queue
import itertools
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


_EXE = ".exe" if os.name == "nt" else ""


def _java() -> str:
    for home in sorted((ROOT / ".cache" / "jdk").glob("*")):
        for cand in (home / "Contents" / "Home" / "bin" / f"java{_EXE}", home / "bin" / f"java{_EXE}"):
            if cand.exists():
                return str(cand)
    return "java"


class OracleError(RuntimeError):
    pass


class Oracle:
    """A running oracle process. Thread-safe; requests are pipelined."""

    def __init__(self, universe: str | None = None, heap: str = "3g", log_path: str | None = None, slots: bool = True,
                 keep_files: bool = False):
        build = ROOT / "oracle" / "build"
        cp_file = build / "classpath.txt"
        if not (ROOT / "oracle" / "src").is_dir():
            raise OracleError("the oracle runs from a clone of the redsim repository installed with "
                              "`maturin develop`; this redsim was installed without one")
        if not cp_file.exists():
            raise OracleError("oracle not built: run `python tools/setup.py` in the repository")
        cp = os.pathsep.join([str(build), cp_file.read_text().strip()])
        tag = f"{os.getpid()}-{id(self) & 0xffff:x}"
        universe = universe or str(ROOT / "oracle" / "run" / f"universe-{tag}")
        log_path = log_path or str(ROOT / "oracle" / "run" / f"oracle-{tag}.log")
        self._universe, self._log_path, self._keep = universe, log_path, keep_files
        self._failed = False
        Path(log_path).parent.mkdir(parents=True, exist_ok=True)
        self._log = open(log_path, "w")
        self.proc = subprocess.Popen(
            [_java(), f"-Xmx{heap}", "-XX:+UseParallelGC",
             f"-Dlog4j2.configurationFile={ROOT / 'oracle' / 'log4j2.xml'}",
             "-cp", cp, "redsim.oracle.Main", universe] + ([] if slots else ["noslots"]),
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self._log,
            cwd=str(ROOT), text=True, bufsize=1,
        )
        self._pending: dict[str, queue.Queue] = {}
        self._lock = threading.Lock()
        self._ids = itertools.count()
        self._ready = threading.Event()
        self.version = None
        self._reader = threading.Thread(target=self._read_loop, daemon=True)
        self._reader.start()
        if not self._ready.wait(180):
            self.close()
            raise OracleError(f"oracle did not start; see {log_path}")

    def _read_loop(self):
        for line in self.proc.stdout:
            line = line.strip()
            if not line:
                continue
            msg = json.loads(line)
            if msg.get("op") == "ready":
                self.version = msg.get("version")
                self._ready.set()
                continue
            q = self._pending.pop(msg.get("id", ""), None)
            if q is not None:
                q.put(msg)
        for q in list(self._pending.values()):
            q.put({"ok": False, "error": "oracle exited"})

    def request(self, req: dict, timeout: float = 600) -> dict:
        rid = req.get("id") or f"r{next(self._ids)}"
        req = dict(req, id=rid)
        q: queue.Queue = queue.Queue()
        self._pending[rid] = q
        with self._lock:
            self.proc.stdin.write(json.dumps(req, separators=(",", ":")) + "\n")
            self.proc.stdin.flush()
        msg = q.get(timeout=timeout)
        if not msg.get("ok", False):
            self._failed = True
            raise OracleError(msg.get("error", "unknown oracle error") + f" (log: {self._log_path})")
        return msg

    def submit(self, req: dict) -> "queue.Queue":
        """Pipelined request; returns a queue that will receive the response."""
        rid = req.get("id") or f"r{next(self._ids)}"
        req = dict(req, id=rid)
        q: queue.Queue = queue.Queue()
        self._pending[rid] = q
        with self._lock:
            self.proc.stdin.write(json.dumps(req, separators=(",", ":")) + "\n")
            self.proc.stdin.flush()
        return q

    def run(self, blocks, size, ticks, actions=(), block_entities=(), offset=None, timeout=600) -> dict:
        req = {"op": "run", "size": list(size), "ticks": ticks, "blocks": [list(b) for b in blocks],
               "actions": list(actions), "block_entities": list(block_entities)}
        if offset is not None:
            req["offset"] = list(offset)
        return self.request(req, timeout=timeout)

    day_time = 0  # the oracle world's overworld clock (new worlds start at 0; time does not advance)

    def set_day_time(self, t: int) -> None:
        """Set the overworld day clock (shared by all slots: only between batches)."""
        if t != self.day_time:
            self.request({"op": "set_day_time", "time": int(t)})
            self.day_time = int(t)

    def extract(self, out_path: str) -> None:
        self.request({"op": "extract", "out": str(out_path)})

    def close(self):
        try:
            if self.proc.poll() is None:
                self.proc.stdin.write('{"op":"quit"}\n')
                self.proc.stdin.flush()
                self.proc.wait(20)
        except Exception:
            self.proc.kill()
            self._failed = True
        self._log.close()
        # remove the scratch world; keep the log only when something went wrong
        import shutil
        if not self._keep:
            shutil.rmtree(self._universe, ignore_errors=True)
            if not self._failed:
                try:
                    os.unlink(self._log_path)
                except OSError:
                    pass

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()
