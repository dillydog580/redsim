"""Minimal, dependency-free NBT reader/writer (Java Edition, big-endian, optional gzip).

Values map to Python as: Byte/Short/Int/Long -> Int (with tag kept), Float/Double -> float,
String -> str, List -> list, Compound -> dict, arrays -> lists. Tag types are kept in
`Tagged` wrappers only where they matter for writing (numbers and arrays).
"""

from __future__ import annotations

import gzip
import io
import struct
from dataclasses import dataclass

END, BYTE, SHORT, INT, LONG, FLOAT, DOUBLE, BYTE_ARRAY, STRING, LIST, COMPOUND, INT_ARRAY, LONG_ARRAY = range(13)


@dataclass
class Tagged:
    """A number or array with an explicit NBT tag type (for faithful writing)."""
    tag: int
    value: object

    def __int__(self):
        return int(self.value)

    def __index__(self):
        return int(self.value)

    def __eq__(self, other):
        return self.value == (other.value if isinstance(other, Tagged) else other)

    def __hash__(self):
        return hash(self.value)


def Byte(v): return Tagged(BYTE, int(v))
def Short(v): return Tagged(SHORT, int(v))
def Int(v): return Tagged(INT, int(v))
def Long(v): return Tagged(LONG, int(v))
def Float(v): return Tagged(FLOAT, float(v))
def Double(v): return Tagged(DOUBLE, float(v))
def ByteArray(v): return Tagged(BYTE_ARRAY, list(v))
def IntArray(v): return Tagged(INT_ARRAY, list(v))
def LongArray(v): return Tagged(LONG_ARRAY, list(v))


def unwrap(v):
    """Strip Tagged wrappers recursively."""
    if isinstance(v, Tagged):
        return v.value
    if isinstance(v, dict):
        return {k: unwrap(x) for k, x in v.items()}
    if isinstance(v, list):
        return [unwrap(x) for x in v]
    return v


class _Reader:
    def __init__(self, data: bytes):
        self.b = data
        self.i = 0

    def take(self, n):
        v = self.b[self.i:self.i + n]
        self.i += n
        return v

    def u(self, fmt):
        n = struct.calcsize(fmt)
        return struct.unpack(">" + fmt, self.take(n))[0]

    def string(self):
        n = self.u("H")
        return self.take(n).decode("utf-8", errors="surrogatepass")

    def payload(self, tag):
        if tag == BYTE:
            return Tagged(BYTE, self.u("b"))
        if tag == SHORT:
            return Tagged(SHORT, self.u("h"))
        if tag == INT:
            return Tagged(INT, self.u("i"))
        if tag == LONG:
            return Tagged(LONG, self.u("q"))
        if tag == FLOAT:
            return Tagged(FLOAT, self.u("f"))
        if tag == DOUBLE:
            return Tagged(DOUBLE, self.u("d"))
        if tag == BYTE_ARRAY:
            n = self.u("i")
            return Tagged(BYTE_ARRAY, list(struct.unpack(f">{n}b", self.take(n))))
        if tag == STRING:
            return self.string()
        if tag == LIST:
            t = self.u("b")
            n = self.u("i")
            return [self.payload(t) for _ in range(n)]
        if tag == COMPOUND:
            out = {}
            while True:
                t = self.u("b")
                if t == END:
                    return out
                k = self.string()
                out[k] = self.payload(t)
        if tag == INT_ARRAY:
            n = self.u("i")
            return Tagged(INT_ARRAY, list(struct.unpack(f">{n}i", self.take(4 * n))))
        if tag == LONG_ARRAY:
            n = self.u("i")
            return Tagged(LONG_ARRAY, list(struct.unpack(f">{n}q", self.take(8 * n))))
        raise ValueError(f"bad NBT tag {tag}")


def loads(data: bytes) -> tuple[str, dict]:
    """Parse NBT bytes (gzip detected automatically). Returns (root name, root compound)."""
    if data[:2] == b"\x1f\x8b":
        data = gzip.decompress(data)
    r = _Reader(data)
    t = r.u("b")
    if t != COMPOUND:
        raise ValueError("NBT root is not a compound")
    name = r.string()
    return name, r.payload(COMPOUND)


def load(path) -> tuple[str, dict]:
    with open(path, "rb") as f:
        return loads(f.read())


def _tag_of(v) -> int:
    if isinstance(v, Tagged):
        return v.tag
    if isinstance(v, bool):
        return BYTE
    if isinstance(v, int):
        return INT
    if isinstance(v, float):
        return DOUBLE
    if isinstance(v, str):
        return STRING
    if isinstance(v, list):
        return LIST
    if isinstance(v, dict):
        return COMPOUND
    raise TypeError(f"cannot encode {type(v)} as NBT")


def _write(out: io.BytesIO, tag: int, v):
    if isinstance(v, Tagged):
        v = v.value
    if tag == BYTE:
        out.write(struct.pack(">b", int(v)))
    elif tag == SHORT:
        out.write(struct.pack(">h", int(v)))
    elif tag == INT:
        out.write(struct.pack(">i", int(v)))
    elif tag == LONG:
        out.write(struct.pack(">q", int(v)))
    elif tag == FLOAT:
        out.write(struct.pack(">f", float(v)))
    elif tag == DOUBLE:
        out.write(struct.pack(">d", float(v)))
    elif tag == BYTE_ARRAY:
        out.write(struct.pack(">i", len(v)))
        out.write(struct.pack(f">{len(v)}b", *v))
    elif tag == STRING:
        b = v.encode("utf-8")
        out.write(struct.pack(">H", len(b)))
        out.write(b)
    elif tag == LIST:
        et = _tag_of(v[0]) if v else END
        out.write(struct.pack(">bi", et, len(v)))
        for x in v:
            _write(out, et, x)
    elif tag == COMPOUND:
        for k, x in v.items():
            t = _tag_of(x)
            out.write(struct.pack(">b", t))
            _write(out, STRING, k)
            _write(out, t, x)
        out.write(b"\x00")
    elif tag == INT_ARRAY:
        out.write(struct.pack(">i", len(v)))
        out.write(struct.pack(f">{len(v)}i", *v))
    elif tag == LONG_ARRAY:
        out.write(struct.pack(">i", len(v)))
        out.write(struct.pack(f">{len(v)}q", *v))
    else:
        raise ValueError(tag)


def dumps(root: dict, name: str = "", compress: bool = True) -> bytes:
    out = io.BytesIO()
    out.write(b"\x0a")
    _write(out, STRING, name)
    _write(out, COMPOUND, root)
    data = out.getvalue()
    return gzip.compress(data, mtime=0) if compress else data


def dump(root: dict, path, name: str = "", compress: bool = True) -> None:
    with open(path, "wb") as f:
        f.write(dumps(root, name, compress))
