#!/usr/bin/env python3
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
"""Local-only regeneration for the AVM decode-output oracle (CONF-AVM-DECODE-ORACLE).

AVM is a LOCAL oracle only — never committed, never run in CI. Needs a local AVM
build (avmenc/avmdec) + ffmpeg. See docs/CONFORMANCE.md.

  python3 tools/decoder-fixtures/generate.py find
  python3 tools/decoder-fixtures/generate.py hashes [--out oracle.json]
  python3 tools/decoder-fixtures/generate.py coverage-fixtures --stage <dir>

`hashes` recomputes, for every committed tests/conformance/vectors/valid/*.ivf, the
.ivf sha256 and the AVM oracle sha256 (avmdec --i420 --rawvideo) and classifies
splot decode (must_pass / xfail_splot). `coverage-fixtures` re-encodes the small
capability-coverage fixtures (profiles, chroma formats, and one isolated intra tool
each) so the corpus is reproducible; move vetted `.ivf` into vectors/valid/ by hand.
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
    ("syn-profile31-mono-intra-16x16", "color=c=gray:size=16x16:rate=1:duration=1", "yuv420p", "--i420",
     ["--disable-warning-prompt", "--quiet", "--limit=1", "--passes=1", "--threads=1",
      "--lag-in-frames=0", "--monochrome", "--input-bit-depth=8", "--bit-depth=8",
      "--profile=31", "--enable-deblocking=0", "--enable-cdef=0", "--enable-restoration=0",
      "--enable-gdf=0", "--enable-ccso=0", "--enable-pc-wiener=0",
      "--enable-wiener-nonsep=0", "--enable-keyframe-filtering=0"], "0", "180"),
    ("syn-2frame-intra-only-mono-16x16-q255",
     "color=c=gray:size=16x16:rate=1:duration=2", "yuv420p", "--i420",
     ["--monochrome"], "8", "255"),
    ("syn-output-multi-brt-16x16",
     "color=c=gray:size=16x16:rate=30:duration=0.0333333333333333", "yuv420p", "--i420",
     ["--monochrome", "--timing-info=unspecified"], "8", "240"),
    ("syn-444-intra-64x64", "testsrc2=size=64x64:rate=1:duration=1", "yuv444p", "--i444", ["--i444"], "8", "120"),
    ("syn-422-intra-64x64", "testsrc2=size=64x64:rate=1:duration=1", "yuv422p", "--i422", ["--i422"], "8", "120"),
    ("syn-mono-intra-64x64", "testsrc2=size=64x64:rate=1:duration=1", "yuv420p", "--i420", ["--monochrome"], "8", "120"),
    ("syn-2tile-intra-128x64", "testsrc2=size=128x64:rate=1:duration=1", "yuv420p", "--i420", ["--tile-columns=1"], "8", "120"),
    ("syn-filmgrain-intra-64x64", "color=c=gray:size=64x64:rate=1:duration=1", "yuv420p", "--i420", ["--film-grain-test=1"], "8", "120"),
    ("syn-fsc-intra-128x128", "testsrc2=size=128x128:rate=1:duration=1", "yuv420p", "--i420", BASE + ["--enable-fsc=1"], "2", "90"),
    ("syn-cctx-444-intra-128x128", "gradients=size=128x128:rate=1:duration=1:c0=red:c1=blue:c2=green:c3=yellow:c4=magenta:c5=cyan:nb_colors=6:x0=10:y0=10:x1=118:y1=118", "yuv444p", "--i444", BASE + ["--enable-intrabc=0", "--enable-cctx=1"], "2", "60"),
    ("syn-ist-intra-128x128", "testsrc2=size=128x128:rate=1:duration=1", "yuv420p", "--i420", BASE + ["--enable-ist=1"], "2", "90"),
    ("syn-mrl-intra-128x128", "testsrc2=size=128x128:rate=1:duration=1", "yuv420p", "--i420", BASE + ["--enable-mrls=1"], "2", "90"),
    ("syn-mhccp-intra-128x128", "testsrc2=size=128x128:rate=1:duration=1", "yuv420p", "--i420", BASE + ["--enable-mhccp=1"], "2", "90"),
    ("syn-parity-intra-128x128", "testsrc2=size=128x128:rate=1:duration=1", "yuv420p", "--i420", BASE + ["--enable-parity-hiding=1"], "2", "90"),
    ("syn-wienerns-intra-128x128", "testsrc2=size=128x128:rate=1:duration=1", "yuv420p", "--i420", BASE + ["--enable-restoration=1", "--enable-wiener-nonsep=1"], "2", "90"),
    ("syn-wienerns-422-intra-128x128", "testsrc2=size=128x128:rate=1:duration=1", "yuv422p", "--i422", BASE + ["--enable-intrabc=0", "--enable-restoration=1", "--enable-wiener-nonsep=1"], "2", "90"),
    ("syn-wienerns-444-intra-128x128", "testsrc2=size=128x128:rate=1:duration=1", "yuv444p", "--i444", BASE + ["--enable-restoration=1", "--enable-wiener-nonsep=1"], "2", "90"),
    ("syn-wienerns-tilerows-intra-256x128", "testsrc2=size=256x128:rate=1:duration=1", "yuv420p", "--i420", BASE + ["--enable-restoration=1", "--enable-wiener-nonsep=1", "--tile-rows=1"], "2", "120"),
    ("syn-cfl-444-tilerows-intra-256x128", "testsrc2=size=256x128:rate=1:duration=1", "yuv444p", "--i444", ["--i444", "--tile-rows=1"], "2", "180"),
    ("syn-pcwiener-intra-128x128", "testsrc2=size=128x128:rate=1:duration=1", "yuv420p", "--i420", BASE + ["--enable-restoration=1", "--enable-pc-wiener=1"], "2", "90"),
    ("syn-gdf-intra-128x128", "testsrc2=size=128x128:rate=1:duration=1", "yuv420p", "--i420", BASE + ["--enable-gdf=1"], "2", "90"),
    ("syn-warp-inter-128x128", "testsrc2=size=128x128:rate=1:duration=4", "yuv420p", "--i420", BASE + ["--enable-warped-motion=1", "--enable-global-motion=1"], "2", "90"),
    ("syn-dirneigh-reorder-128x64-q196",
     "nullsrc=size=128x64:rate=1:duration=1,format=yuv420p,geq=lum='if(lt(X,64),if(lt(X+Y,64),40,210),30+180*Y/(64-1))':cb=120:cr=130",
     "yuv420p", "--i420",
     ["--disable-warning-prompt", "--quiet", "--limit=1", "--passes=1", "--threads=1",
      "--lag-in-frames=0", "--sb-size=64", "--min-partition-size=64",
      "--max-partition-size=64", "--enable-gdf=0", "--enable-cdef=0",
      "--enable-deblocking=0", "--enable-restoration=0", "--enable-cfl-intra=0",
      "--enable-mhccp=0", "--enable-cctx=0", "--enable-ibp=0", "--enable-angle-delta=0",
      "--enable-intra-edge-filter=0", "--enable-ist=0", "--enable-intra-dip=0",
      "--enable-idtx-intra=0", "--enable-mrls=0", "--enable-palette=0",
      "--enable-bawp=0", "--enable-rect-partitions=0", "--enable-ext-partitions=0",
      "--enable-uneven-4way-partitions=0", "--enable-sdp=0", "--enable-fsc=0",
      "--enable-tx-partition=0", "--enable-smooth-intra=0", "--enable-paeth-intra=0",
      "--enable-ccso=0", "--enable-pc-wiener=0", "--enable-wiener-nonsep=0",
      "--enable-keyframe-filtering=0", "--enable-intrabc=0", "--enable-parity-hiding=0",
      "--enable-tcq=0"], "2", "196"),
]

PINNED_RECIPE_HASHES = {
    "syn-profile31-mono-intra-16x16": {
        "avm_revision": "457cd58681a747465661baccb1f32095bc5b7774",
        "source_sha256": "83dc7abaa81f46324b7a47fa89b127c1f8891ff2b3d97e4736ac25e45aadb1c6",
        "ivf_sha256": "5cda9a0c51c31721036a23c2601b88770989e9e872c66d32fc5d0a1875b53501",
    },
    "syn-output-multi-brt-16x16": {
        "avm_revision": "457cd58681a747465661baccb1f32095bc5b7774",
        "source_sha256": "1a862fd46eb2b14b9772b8cba7f1ef4a97d833d76e3ad9b4149ed56504366220",
        "ivf_sha256": "7dd5e609570d7d8be941e684f8e7bf7be669f6e9d39f7c03167b09e9fec4764c",
        "avm_native_raw_sha256": "5a5f307aa9ce504d9235634f15cf382e8914c49fbd8dd4d4c47136c917886f7b",
        "avm_i420_raw_sha256": "f83545d43c6939ec393b6b8310959b6174fd764b08a12fc22d908408a7e6a43e",
        "instrumentation_sha256": "6ec529e93ff9ec09ab211e6bc29034937302eb7c686d7fe6c872f88984a41164",
        "recipe_sha256": "95d423a982a8b0995a4a21d2feae525447f3a69b22dd57c4440602960643bb64",
    },
    "syn-2frame-intra-only-mono-16x16-q255": {
        "avm_revision": "457cd58681a747465661baccb1f32095bc5b7774",
        "ffmpeg_version": "8.1.2",
        "source_sha256": "2e8c4d3b8780635c7b7eb83887a21ca35e280d4ac900c62f158201dac89c7186",
        "ivf_sha256": "74acd49de73519e0d80a4508e5dcaf7548f22669e0aafe2fa62c9f9610289776",
        "avm_i420_raw_sha256": "0dc50ed6e41c0a4d3eaa8c5a1e850607fd4bf2042596439f6112369e90f58364",
        "instrumentation_sha256": "b99a7a36c7b143d3af07cc0418a590f461344698dc1d181352bfc17dbab00dcc",
        "instrumentation_source": "av2/encoder/encode_strategy.c",
        "avmenc_sha256": "132ff0f7ddd74bfd35c59cb4f50413ead008cc541d8e50f1cd34f905adeb9d68",
        "avmdec_sha256": "cd465be567e971105695fd3fbff0e277969de5c497ce3bc18c3a7b8185131247",
        "reproducibility_runs": 2,
    },
    "syn-dirneigh-reorder-128x64-q196": {
        "avm_revision": "457cd58681a747465661baccb1f32095bc5b7774",
        "ffmpeg_version": "8.1.2",
        "source_sha256": "7dcfe06c50858a36f3227248a7c5da311d7dece274902d6e0a6bd4463f0e2dc8",
        "ivf_sha256": "c87506fdee597539962c49ad5ced2dfb1b36d7d50919158d73f368b4c6790b25",
        "obu_sha256": "84bf993e21f6edd790e9f7cc47dc4fe8fd1760a116c701e5aa892fba3c1fbb24",
        "avm_i420_raw_sha256": "a1500046842354466967cd766ae569ed61927ffcf4617cbc471a3c7644d18edd",
        "provenance_sha256": "a6850524c0725f28a9cf35605096caad8eea05ac8e71a4d2407853356319d6b0",
        "instrumentation_sha256": "da3df2afe8e8cedf670a67d551cfd4e95671a0d5d8b3a48d5f0f53233008fc49",
        "avmenc_sha256": "952af54f32109602ee9315dd36cc5ec3d01adcaae42f2a7d1cfdd11e92815b2e",
        "avmdec_sha256": "70342f860be8ccb277c6d719c631f3c054f0ec52b1bcf40c5db00941b8a56ab2",
        "reproducibility_runs": 2,
    },
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
    selected = [row for row in COVERAGE if args.only is None or row[0] == args.only]
    if not selected:
        sys.exit(f"error: unknown coverage fixture id: {args.only}")
    for fid, src, pix, inflag, flags, cpu, qp in selected:
        expected = PINNED_RECIPE_HASHES.get(fid)
        avmdec = None
        if expected and "avm_revision" in expected:
            avm_root = os.environ.get("AVM_ROOT") or os.path.dirname(os.path.dirname(avmenc))
            revision = run_checked(["git", "-C", avm_root, "rev-parse", "HEAD"])
            revision = revision.stdout.decode().strip()
            if revision != expected["avm_revision"]:
                sys.exit(f"error: {fid} requires AVM revision {expected['avm_revision']}: "
                         f"found {revision}")
        if expected and ("avmdec_sha256" in expected or "avm_i420_raw_sha256" in expected):
            avmdec = find("avmdec")
        if expected and "avmenc_sha256" in expected and sha(avmenc) != expected["avmenc_sha256"]:
            sys.exit(f"error: {fid} requires the recorded AVM encoder")
        if expected and "avmdec_sha256" in expected and sha(avmdec) != expected["avmdec_sha256"]:
            sys.exit(f"error: {fid} requires the recorded AVM decoder")
        if expected and "ffmpeg_version" in expected:
            version = run_checked(["ffmpeg", "-version"]).stdout.decode().splitlines()[0]
            if not version.startswith("ffmpeg version " + expected["ffmpeg_version"]):
                sys.exit(f"error: {fid} requires ffmpeg {expected['ffmpeg_version']}: {version}")
        if expected and "instrumentation_source" in expected:
            avm_root = os.environ.get("AVM_ROOT") or os.path.dirname(os.path.dirname(avmenc))
            revision = run_checked(["git", "-C", avm_root, "rev-parse", "HEAD"])
            revision = revision.stdout.decode().strip()
            instrumentation = run_checked([
                "git", "-C", avm_root, "diff", "--", expected["instrumentation_source"]
            ]).stdout
            instrumentation_sha256 = hashlib.sha256(instrumentation).hexdigest()
            if revision != expected["avm_revision"] or instrumentation_sha256 != expected["instrumentation_sha256"]:
                sys.exit(f"error: {fid} requires pinned instrumented AVM: "
                         f"revision={revision}, instrumentation={instrumentation_sha256}")
            if sha(avmenc) != expected["avmenc_sha256"] or sha(avmdec) != expected["avmdec_sha256"]:
                sys.exit(f"error: {fid} requires the recorded instrumented AVM producer binaries")
            version = run_checked(["ffmpeg", "-version"]).stdout.decode().splitlines()[0]
            if not version.startswith("ffmpeg version " + expected["ffmpeg_version"]):
                sys.exit(f"error: {fid} requires ffmpeg {expected['ffmpeg_version']}: {version}")
        y4m = os.path.join(stage, fid + ".y4m")
        run_checked(["ffmpeg", "-loglevel", "error", "-y", "-f", "lavfi", "-i", src,
                     "-pix_fmt", pix, y4m])
        ivf = os.path.join(stage, fid + ".ivf")
        encode = [avmenc, "--codec=av2", "--ivf", "-D", "--cpu-used=" + cpu,
                  "--end-usage=q", "--qp=" + qp, "--kf-max-dist=0", "-t", "1",
                  inflag, *flags]
        run_checked([*encode, "-o", ivf, y4m])
        if expected:
            actual_source = sha(y4m)
            actual_ivf = sha(ivf)
            if actual_source != expected["source_sha256"] or actual_ivf != expected["ivf_sha256"]:
                sys.exit(f"error: {fid} differs from pinned AVM {expected['avm_revision']}: "
                         f"source={actual_source}, ivf={actual_ivf}")
            if expected.get("reproducibility_runs") == 2:
                repeated_ivf = os.path.join(stage, fid + ".repeat.ivf")
                run_checked([*encode, "-o", repeated_ivf, y4m])
                if sha(repeated_ivf) != actual_ivf:
                    sys.exit(f"error: {fid} is not deterministic across two encodes")
                os.remove(repeated_ivf)
            if "obu_sha256" in expected:
                obu = os.path.join(stage, fid + ".obu")
                obu_encode = [avmenc, "--codec=av2", "--obu", "-D", "--cpu-used=" + cpu,
                              "--end-usage=q", "--qp=" + qp, "--kf-max-dist=0", "-t", "1",
                              inflag, *flags]
                run_checked([*obu_encode, "-o", obu, y4m])
                if sha(obu) != expected["obu_sha256"]:
                    sys.exit(f"error: {fid} differs from its pinned raw OBU")
            if "avm_i420_raw_sha256" in expected:
                raw = os.path.join(stage, fid + ".avm.raw")
                run_checked([avmdec, "--i420", "--rawvideo", "-o", raw, ivf])
                if sha(raw) != expected["avm_i420_raw_sha256"]:
                    sys.exit(f"error: {fid} differs from its pinned AVM raw output")
            if "instrumentation_sha256" in expected:
                print(f"  {fid}: isolated AVM instrumentation "
                      f"{expected['instrumentation_sha256']}")
                if "recipe_sha256" in expected:
                    print(f"  {fid}: recipe {expected['recipe_sha256']}")
                if "avm_native_raw_sha256" in expected:
                    print(f"  {fid}: AVM native raw {expected['avm_native_raw_sha256']}, "
                          f"forced I420 raw {expected['avm_i420_raw_sha256']}")
                else:
                    print(f"  {fid}: forced I420 raw {expected['avm_i420_raw_sha256']}")
    print(f"staged {len(selected)} coverage fixtures in {stage} "
          f"(move vetted `.ivf` into {os.path.relpath(VALID, REPO)}/ and refresh hashes)")


def main():
    p = argparse.ArgumentParser(description=__doc__)
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("find").set_defaults(fn=cmd_find)
    h = sub.add_parser("hashes"); h.add_argument("--out"); h.set_defaults(fn=cmd_hashes)
    c = sub.add_parser("coverage-fixtures"); c.add_argument("--stage", required=True)
    c.add_argument("--only", help="regenerate one fixture id")
    c.set_defaults(fn=cmd_coverage_fixtures)
    args = p.parse_args()
    args.fn(args)


if __name__ == "__main__":
    main()
