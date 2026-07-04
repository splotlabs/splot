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


# capability -> avmenc flags over a disable-broad intra base (so a lone enabled tool
# is what splot rejects, not an earlier gate). Chroma/format jobs set the format.
BASE = ["--enable-cfl-intra=0", "--enable-intra-dip=0", "--enable-ibp=0", "--enable-mrls=0",
        "--enable-intra-edge-filter=0", "--enable-sdp=0", "--enable-fsc=0", "--enable-cctx=0",
        "--enable-idtx-intra=0", "--enable-ist=0", "--enable-inter-ist=0", "--enable-mhccp=0",
        "--enable-parity-hiding=0", "--enable-tcq=0", "--enable-restoration=0",
        "--enable-wiener-nonsep=0", "--enable-pc-wiener=0", "--enable-gdf=0"]
FMT = {  # id: (pix_fmt, size, input_flag, extra_flags, strict)
    "syn-444-intra-64x64": ("yuv444p", "64x64", "--i444", ["--i444"], False),
    "syn-422-intra-64x64": ("yuv422p", "64x64", "--i422", ["--i422"], False),
    "syn-mono-intra-64x64": ("yuv420p", "64x64", "--i420", ["--monochrome"], False),
    "syn-2tile-intra-128x64": ("yuv420p", "128x64", "--i420", ["--tile-columns=1"], False),
    "syn-filmgrain-intra-64x64": ("yuv420p", "64x64", "--i420", ["--film-grain-test=1"], False),
}
TOOLS = {  # id: enable-flags over BASE (on a shared testsrc2 64x64 source)
    "syn-fsc-intra-64x64": ["--enable-fsc=1"], "syn-ist-intra-64x64": ["--enable-ist=1"],
    "syn-mrl-intra-64x64": ["--enable-mrls=1"], "syn-mhccp-intra-64x64": ["--enable-mhccp=1"],
    "syn-dip-intra-64x64": ["--enable-intra-dip=1"], "syn-tcq-intra-64x64": ["--enable-tcq=1"],
    "syn-parity-intra-64x64": ["--enable-parity-hiding=1"],
    "syn-deltaq-intra-64x64": ["--deltaq-mode=1", "--enable-tpl-model=1"],
    "syn-lr-intra-64x64": ["--enable-restoration=1"],
    "syn-wienerns-intra-64x64": ["--enable-restoration=1", "--enable-wiener-nonsep=1"],
    "syn-pcwiener-intra-64x64": ["--enable-restoration=1", "--enable-pc-wiener=1"],
    "syn-gdf-intra-64x64": ["--enable-gdf=1"],
}


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
    testsrc = os.path.join(stage, "testsrc2-64.y4m")
    run(["ffmpeg", "-y", "-f", "lavfi", "-i", "testsrc2=size=64x64:rate=1:duration=1",
         "-pix_fmt", "yuv420p", testsrc])
    common = [avmenc, "--codec=av2", "--ivf", "-D", "--cpu-used=8", "--end-usage=q",
              "--qp=120", "--kf-max-dist=0", "-t", "1"]
    for fid, (pix, size, inflag, extra, _s) in FMT.items():
        s = os.path.join(stage, fid + ".y4m")
        run(["ffmpeg", "-y", "-f", "lavfi", "-i", f"testsrc2=size={size}:rate=1:duration=1",
             "-pix_fmt", pix, s])
        run(common + [inflag, *extra, "-o", os.path.join(stage, fid + ".ivf"), s])
    for fid, flags in TOOLS.items():
        run(common + ["--i420", *BASE, *flags, "-o", os.path.join(stage, fid + ".ivf"), testsrc])
    print(f"staged {len(FMT) + len(TOOLS)} coverage fixtures in {stage}")


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
