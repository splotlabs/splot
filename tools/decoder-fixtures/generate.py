#!/usr/bin/env python3
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
"""Local-only regeneration for the AVM decode-output oracle (CONF-AVM-DECODE-ORACLE).

AVM is a LOCAL oracle only — never committed, never run in CI. Needs a local AVM
build (avmenc/avmdec) + ffmpeg. See docs/decoder/AVM-FIXTURE-CORPUS.md.

  python3 tools/decoder-fixtures/generate.py find
  python3 tools/decoder-fixtures/generate.py hashes [--out oracle.json]
  python3 tools/decoder-fixtures/generate.py coverage-fixtures --stage <dir>

`hashes` recomputes, for every committed tests/conformance/vectors/valid/*.ivf, the
.ivf sha256 and the AVM oracle sha256 (avmdec --i420 --rawvideo) and classifies
splot decode (must_pass / xfail_splot). `coverage-fixtures` re-encodes the small
capability-coverage fixtures (chroma formats, and one isolated intra tool each) so
the corpus is reproducible; move vetted `.ivf` into vectors/valid/ by hand.
"""
import argparse, hashlib, json, os, shutil, subprocess, sys

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
VALID = os.path.join(REPO, "tests/conformance/vectors/valid")


def find(name):
    for base in filter(None, [os.environ.get("AVM_BUILD"), os.environ.get("AVM_ROOT"),
                              os.path.expanduser("~/Devel/avm")]):
        for sub in ("", "build", "build-splot-fixtures", "build_inspect"):
            p = os.path.join(base, sub, name)
            if os.path.isfile(p):
                return p
    p = shutil.which(name)
    if p:
        return p
    sys.exit(f"error: {name} not found (set $AVM_ROOT to your AVM checkout)")


def sha(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for c in iter(lambda: f.read(1 << 16), b""):
            h.update(c)
    return h.hexdigest()


def run(cmd):
    return subprocess.run(cmd, capture_output=True, timeout=180)


def run_checked(cmd):
    r = run(cmd)
    if r.returncode != 0:
        raise SystemExit(f"error: {cmd[0]} failed ({r.returncode}): "
                         f"{r.stderr.decode(errors='replace')[:200]}")
    return r


# Disable-broad intra base: with these off, a lone enabled tool is what appears in
# (and is rejected from) the stream. `cpu-used=2` (full-enough RD) is required — a
# fast encode collapses most tools back to a byte-identical baseline.
BASE = ["--enable-cfl-intra=0", "--enable-intra-dip=0", "--enable-ibp=0", "--enable-mrls=0",
        "--enable-intra-edge-filter=0", "--enable-sdp=0", "--enable-fsc=0", "--enable-cctx=0",
        "--enable-idtx-intra=0", "--enable-ist=0", "--enable-inter-ist=0", "--enable-mhccp=0",
        "--enable-parity-hiding=0", "--enable-tcq=0", "--enable-restoration=0",
        "--enable-wiener-nonsep=0", "--enable-pc-wiener=0", "--enable-gdf=0"]

# The COMPLETE recipe for every committed coverage fixture, deterministic via `-D`.
# Each row: (id, lavfi_src, pix_fmt, input_flag, avmenc_flags, cpu, qp). Only tool
# fixtures byte-distinct from a same-content baseline are committed (see the
# byte-distinctness guard in `cargo xtask decoder-fixtures verify`).
COVERAGE = [
    ("syn-444-intra-64x64", "testsrc2=size=64x64:rate=1:duration=1", "yuv444p", "--i444", ["--i444"], "8", "120"),
    ("syn-422-intra-64x64", "testsrc2=size=64x64:rate=1:duration=1", "yuv422p", "--i422", ["--i422"], "8", "120"),
    ("syn-mono-intra-64x64", "testsrc2=size=64x64:rate=1:duration=1", "yuv420p", "--i420", ["--monochrome"], "8", "120"),
    ("syn-2tile-intra-128x64", "testsrc2=size=128x64:rate=1:duration=1", "yuv420p", "--i420", ["--tile-columns=1"], "8", "120"),
    ("syn-filmgrain-intra-64x64", "color=c=gray:size=64x64:rate=1:duration=1", "yuv420p", "--i420", ["--film-grain-test=1"], "8", "120"),
    ("syn-fsc-intra-128x128", "testsrc2=size=128x128:rate=1:duration=1", "yuv420p", "--i420", BASE + ["--enable-fsc=1"], "2", "90"),
    ("syn-ist-intra-128x128", "testsrc2=size=128x128:rate=1:duration=1", "yuv420p", "--i420", BASE + ["--enable-ist=1"], "2", "90"),
    ("syn-mrl-intra-128x128", "testsrc2=size=128x128:rate=1:duration=1", "yuv420p", "--i420", BASE + ["--enable-mrls=1"], "2", "90"),
    ("syn-mhccp-intra-128x128", "testsrc2=size=128x128:rate=1:duration=1", "yuv420p", "--i420", BASE + ["--enable-mhccp=1"], "2", "90"),
    ("syn-parity-intra-128x128", "testsrc2=size=128x128:rate=1:duration=1", "yuv420p", "--i420", BASE + ["--enable-parity-hiding=1"], "2", "90"),
    ("syn-wienerns-intra-128x128", "testsrc2=size=128x128:rate=1:duration=1", "yuv420p", "--i420", BASE + ["--enable-restoration=1", "--enable-wiener-nonsep=1"], "2", "90"),
    ("syn-pcwiener-intra-128x128", "testsrc2=size=128x128:rate=1:duration=1", "yuv420p", "--i420", BASE + ["--enable-restoration=1", "--enable-pc-wiener=1"], "2", "90"),
    ("syn-gdf-intra-128x128", "testsrc2=size=128x128:rate=1:duration=1", "yuv420p", "--i420", BASE + ["--enable-gdf=1"], "2", "90"),
    ("syn-warp-inter-128x128", "testsrc2=size=128x128:rate=1:duration=4", "yuv420p", "--i420", BASE + ["--enable-warped-motion=1", "--enable-global-motion=1"], "2", "90"),
]


def cmd_find(_):
    for n in ("avmenc", "avmdec"):
        print(f"{n}: {find(n)}")


def cmd_hashes(args):
    avmdec, splot = find("avmdec"), os.path.join(REPO, "target/release/splot")
    rows = []
    for name in sorted(f for f in os.listdir(VALID) if f.endswith(".ivf")):
        ivf = os.path.join(VALID, name)
        raw = "/tmp/_o.raw"
        avm_ok = run([avmdec, "--i420", "--rawvideo", "-o", raw, ivf]).returncode == 0
        avm = sha(raw) if avm_ok and os.path.exists(raw) else None
        sp = run([splot, "decode", "--output-format", "raw", "-o", "/tmp/_s.raw", ivf])
        cls = ("must_pass" if sp.returncode == 0 and sha("/tmp/_s.raw") == avm
               else "xfail_splot" if sp.returncode != 0 else "MISMATCH")
        rows.append({"id": name[:-4], "ivf_sha256": sha(ivf), "avm_raw_sha256": avm, "status": cls})
        print(f"  {cls:12s} {name}")
    out = args.out or "/tmp/oracle.json"
    json.dump(rows, open(out, "w"), indent=2)
    print(f"wrote {out} ({len(rows)} fixtures)")


def cmd_coverage_fixtures(args):
    avmenc = find("avmenc")
    stage = args.stage
    os.makedirs(stage, exist_ok=True)
    for fid, src, pix, inflag, flags, cpu, qp in COVERAGE:
        y4m = os.path.join(stage, fid + ".y4m")
        run_checked(["ffmpeg", "-y", "-f", "lavfi", "-i", src, "-pix_fmt", pix, y4m])
        run_checked([avmenc, "--codec=av2", "--ivf", "-D", "--cpu-used=" + cpu,
                     "--end-usage=q", "--qp=" + qp, "--kf-max-dist=0", "-t", "1",
                     inflag, *flags, "-o", os.path.join(stage, fid + ".ivf"), y4m])
    print(f"staged {len(COVERAGE)} coverage fixtures in {stage} "
          f"(move vetted `.ivf` into {os.path.relpath(VALID, REPO)}/ and refresh hashes)")


def main():
    p = argparse.ArgumentParser(description=__doc__)
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("find").set_defaults(fn=cmd_find)
    h = sub.add_parser("hashes"); h.add_argument("--out"); h.set_defaults(fn=cmd_hashes)
    c = sub.add_parser("coverage-fixtures"); c.add_argument("--stage", required=True); c.set_defaults(fn=cmd_coverage_fixtures)
    args = p.parse_args()
    args.fn(args)


if __name__ == "__main__":
    main()
