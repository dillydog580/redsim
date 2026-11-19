"""Set up the real-game oracle (only needed for `redsim verify`, fuzzing and `redsim world`).

Downloads into .cache/ (git-ignored, never committed):
  - the Minecraft server jar for the target version (from Mojang's piston-meta)
  - a JDK 25 (Eclipse Temurin) if `java` 25+ is not available
then compiles oracle/ against the jar and (optionally) regenerates block data.

    python tools/setup.py                 # download + build the oracle
    python tools/setup.py --decompile     # also decompile the game (for reading its code)
    python tools/setup.py --regen         # also re-extract block data and regenerate tables
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys
import tarfile
import urllib.request
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CACHE = ROOT / ".cache"
MANIFEST = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json"
VERSION = "26.3"
EXE = ".exe" if os.name == "nt" else ""


def fetch(url, dest: Path, sha1: str | None = None):
    if dest.exists() and (sha1 is None or hashlib.sha1(dest.read_bytes()).hexdigest() == sha1):
        return dest
    dest.parent.mkdir(parents=True, exist_ok=True)
    print(f"downloading {url}")
    with urllib.request.urlopen(url) as r, open(dest, "wb") as f:
        shutil.copyfileobj(r, f)
    if sha1 and hashlib.sha1(dest.read_bytes()).hexdigest() != sha1:
        raise SystemExit(f"checksum mismatch for {dest}")
    return dest


def server_jar(version: str) -> Path:
    mc = CACHE / "mc" / version
    manifest = json.load(urllib.request.urlopen(MANIFEST))
    print("latest release according to Mojang:", manifest["latest"]["release"])
    v = next(v for v in manifest["versions"] if v["id"] == version)
    meta = json.load(urllib.request.urlopen(v["url"]))
    (mc / f"{version}.json").parent.mkdir(parents=True, exist_ok=True)
    (mc / f"{version}.json").write_text(json.dumps(meta))
    dl = meta["downloads"]["server"]
    jar = fetch(dl["url"], mc / "server.jar", dl["sha1"])
    # the server jar is a bundler: unpack the real jar and its libraries
    with zipfile.ZipFile(jar) as z:
        for n in z.namelist():
            if n.startswith("META-INF/versions/") or n.startswith("META-INF/libraries/") or n.endswith(".list"):
                z.extract(n, mc / "bundle")
    inner = next((mc / "bundle" / "META-INF" / "versions").rglob("*.jar"))
    shutil.copy(inner, mc / f"server-{version}.jar")
    return mc


def java_home() -> Path:
    for home in sorted((CACHE / "jdk").glob("*")):
        for cand in (home / "Contents" / "Home", home):
            if (cand / "bin" / f"javac{EXE}").exists():
                return cand
    arch = "aarch64" if platform.machine() in ("arm64", "aarch64") else "x64"
    os_ = {"Darwin": "mac", "Linux": "linux", "Windows": "windows"}[platform.system()]
    url = f"https://api.adoptium.net/v3/binary/latest/25/ga/{os_}/{arch}/jdk/hotspot/normal/eclipse"
    archive = fetch(url, CACHE / "jdk" / ("jdk25.zip" if os_ == "windows" else "jdk25.tar.gz"))
    if archive.suffix == ".zip":
        zipfile.ZipFile(archive).extractall(CACHE / "jdk")
    else:
        tarfile.open(archive).extractall(CACHE / "jdk")
    archive.unlink()
    return java_home()


def build_oracle(jh: Path, version: str):
    """Compile oracle/ against the server jar (the same as oracle/build.sh, but also on Windows)."""
    mc = CACHE / "mc" / version
    jars = [mc / f"server-{version}.jar"] + sorted((mc / "bundle" / "META-INF" / "libraries").rglob("*.jar"))
    cp = os.pathsep.join(os.path.relpath(j, ROOT) for j in jars)  # the oracle runs with cwd = ROOT
    out = ROOT / "oracle" / "build"
    shutil.rmtree(out, ignore_errors=True)
    out.mkdir(parents=True)
    srcs = sorted(str(s.relative_to(ROOT)) for s in (ROOT / "oracle" / "src").rglob("*.java"))
    subprocess.run([str(jh / "bin" / f"javac{EXE}"), "-nowarn", "-encoding", "UTF-8", "--release", "25",
                    "-cp", cp, "-d", str(out.relative_to(ROOT)), *srcs], check=True, cwd=ROOT)
    (out / "classpath.txt").write_text(cp)
    print("oracle built")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--version", default=VERSION)
    ap.add_argument("--decompile", action="store_true")
    ap.add_argument("--regen", action="store_true")
    a = ap.parse_args()
    mc = server_jar(a.version)
    jh = java_home()
    print("JDK:", jh)
    build_oracle(jh, a.version)
    if a.decompile:
        vf = fetch("https://repo1.maven.org/maven2/org/vineflower/vineflower/1.12.0/vineflower-1.12.0.jar", CACHE / "tools" / "vineflower.jar")
        subprocess.run([str(jh / "bin" / f"java{EXE}"), "-Xmx6g", "-jar", str(vf), "-dgs=1", "-rsy=1", "-rbr=1", "-lit=1",
                        "--only=net/minecraft", str(mc / f"server-{a.version}.jar"), str(mc / "src")], check=True)
    if a.regen:
        sys.path.insert(0, str(ROOT / "python"))
        from redsim.oracle import Oracle
        with Oracle() as o:
            o.extract(str(ROOT / "data" / f"blocks-{a.version}.json"))
        subprocess.run([sys.executable, str(ROOT / "tools" / "gen_blockdata.py"), a.version], check=True)
    print("oracle ready. Try: redsim verify <build> or python -m redsim.fuzz --count 100")


if __name__ == "__main__":
    main()
