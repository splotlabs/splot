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
.ivf sha256 and the AVM oracle sha256 (avmdec --i420 --rawvideo). AVM decoder
failures are recorded as null hashes for local inventory; the committed manifest
remains strict. `coverage-fixtures` re-encodes the small capability-coverage
fixtures (profiles, chroma formats, and one isolated intra tool each) so the corpus
is reproducible; move vetted `.ivf` into vectors/valid/ by hand.
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


def run(cmd, env=None):
    return subprocess.run(cmd, capture_output=True, timeout=180, env=env)


def run_checked(cmd, env=None):
    r = run(cmd, env=env)
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

TX_PARTITION_INTRA = [
    "--limit=1", "--passes=1", "--threads=1", "--bit-depth=8", "--input-bit-depth=8",
    "--sb-size=64", "--min-partition-size=64", "--max-partition-size=64", "--enable-sdp=0",
    "--enable-extended-sdp=0", "--enable-cfl-intra=0", "--enable-mhccp=0", "--enable-cctx=0",
    "--enable-chroma-dctonly=0", "--enable-cdef=0", "--enable-deblocking=0",
    "--enable-restoration=0", "--enable-ccso=0", "--enable-gdf=0", "--enable-pc-wiener=0",
    "--enable-wiener-nonsep=0", "--enable-qm=0", "--enable-tcq=0",
    "--enable-parity-hiding=0", "--enable-trellis-quant=0", "--enable-fsc=0",
    "--enable-flip-idtx=0", "--enable-idtx-intra=0", "--enable-ist=0", "--enable-inter-ist=0",
    "--enable-intra-dip=0", "--enable-ibp=0", "--enable-mrls=0",
    "--enable-intra-edge-filter=0", "--enable-angle-delta=0", "--enable-palette=0",
    "--enable-intrabc=0", "--enable-intrabc-ext=0", "--enable-paeth-intra=0",
    "--enable-smooth-intra=0", "--enable-rect-partitions=0", "--enable-ext-partitions=0",
    "--enable-uneven-4way-partitions=0", "--enable-tx-partition=1",
    "--reduced-tx-part-set=1", "--deltaq-mode=0", "--aq-mode=0",
    "--enable-chroma-deltaq=0", "--enable-keyframe-filtering=0", "--lag-in-frames=0",
    "--force-video-mode=0", "--test-decode=fatal", "--quiet", "--disable-warning-prompt",
]

TX_PARTITION_INTER = [
    "--limit=2", "--passes=1", "--threads=1", "--bit-depth=8", "--input-bit-depth=8",
    "--sb-size=64", "--min-partition-size=64", "--max-partition-size=64", "--kf-min-dist=9999",
    "--kf-max-dist=9999", "--lag-in-frames=0", "--max-reference-frames=1",
    "--cdf-update-mode=0", "--aq-mode=0", "--deltaq-mode=0", "--enable-chroma-deltaq=0",
    "--enable-tpl-model=0", "--enable-sdp=0", "--enable-extended-sdp=0",
    "--enable-cfl-intra=0", "--enable-mhccp=0", "--enable-cctx=0",
    "--enable-chroma-dctonly=1", "--enable-qm=0", "--enable-deblocking=0",
    "--enable-cdef=0", "--enable-cdef-on-skip-txfm=0", "--enable-restoration=0",
    "--enable-wiener-nonsep=0", "--enable-pc-wiener=0", "--enable-gdf=0", "--enable-ccso=0",
    "--enable-ibp=0", "--enable-angle-delta=0", "--enable-intra-edge-filter=0",
    "--enable-ist=0", "--enable-inter-ist=0", "--enable-inter-ddt=0", "--enable-intra-dip=0",
    "--enable-idtx-intra=0", "--enable-flip-idtx=0", "--enable-mrls=0", "--enable-palette=0",
    "--enable-intrabc=0", "--enable-fsc=0", "--enable-tx-partition=1",
    "--reduced-tx-part-set=1", "--enable-rect-partitions=0", "--enable-ext-partitions=0",
    "--enable-uneven-4way-partitions=0", "--enable-parity-hiding=0", "--enable-tcq=0",
    "--enable-trellis-quant=0", "--enable-keyframe-filtering=0", "--enable-smooth-intra=0",
    "--enable-paeth-intra=0", "--enable-fwd-kf=0", "--enable-overlay=0", "--enable-bawp=0",
    "--enable-cwp=0", "--enable-masked-comp=0", "--enable-interinter-wedge=0",
    "--enable-diff-wtd-comp=0", "--enable-imp-msk-bld=0", "--enable-interintra-comp=0",
    "--enable-interintra-wedge=0", "--enable-smooth-interintra=0", "--enable-tip=0",
    "--enable-tip-refinemv=0", "--enable-refinemv=0", "--enable-opfl-refine=0",
    "--enable-global-motion=0", "--enable-warped-motion=0", "--enable-warp-causal=0",
    "--enable-warp-delta=0", "--enable-six-param-warp-delta=0", "--enable-warp-extend=0",
    "--enable-ref-frame-mvs=0", "--enable-refmvbank=0", "--enable-drl-reorder=0",
    "--enable-mv-traj=0", "--enable-high-motion=0", "--enable-adaptive-mvd=0",
    "--enable-flex-mvres=0", "--enable-joint-mvd=0", "--enable-mvd-sign-derive=0",
    "--enable-skip-mode=0", "--enable-bru=0", "--enable-onesided-comp=0",
    "--monotonic-output-order=1", "--test-decode=fatal", "--quiet", "--disable-warning-prompt",
]

SB256_INTRA = [
    "--limit=1", "--passes=1", "--threads=1", "--lag-in-frames=0",
    "--min-partition-size=16", "--max-partition-size=128", "--sb-size=256",
    "--enable-sdp=0", "--enable-extended-sdp=0", "--enable-cfl-intra=0",
    "--enable-mhccp=0", "--enable-cctx=0", "--enable-chroma-dctonly=0",
    "--enable-cdef=0", "--enable-deblocking=0", "--enable-restoration=0",
    "--enable-ccso=0", "--enable-gdf=0", "--enable-pc-wiener=0",
    "--enable-wiener-nonsep=0", "--enable-qm=0", "--enable-tcq=0",
    "--enable-parity-hiding=0", "--enable-trellis-quant=0", "--enable-fsc=0",
    "--enable-flip-idtx=0", "--enable-idtx-intra=0", "--enable-ist=0",
    "--enable-inter-ist=0", "--enable-intra-dip=0", "--enable-ibp=0",
    "--enable-mrls=0", "--enable-intra-edge-filter=0", "--enable-angle-delta=0",
    "--enable-palette=0", "--enable-intrabc=0", "--enable-intrabc-ext=0",
    "--enable-paeth-intra=0", "--enable-smooth-intra=0",
    "--enable-rect-partitions=0", "--enable-ext-partitions=0",
    "--enable-uneven-4way-partitions=0", "--enable-tx-partition=0",
    "--reduced-tx-part-set=1", "--deltaq-mode=0", "--aq-mode=0",
    "--enable-chroma-deltaq=0", "--enable-keyframe-filtering=0",
    "--force-video-mode=0", "--test-decode=fatal", "--quiet",
    "--disable-warning-prompt",
]

# The COMPLETE recipe for every committed coverage fixture, deterministic via `-D`.
# Each row: (id, lavfi_src, pix_fmt, input_flag, avmenc_flags, cpu, qp[, source]).
# `source` defaults to Y4M; the optional mapping selects deterministic raw input.
# Only tool
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
    ("syn-golomb-lossless-mono-16x16",
     "nullsrc=s=16x16:d=1,geq=lum=96:cb=128:cr=128", "yuv420p", "--i420",
     ["--disable-warning-prompt", "--quiet", "--limit=1", "--passes=1", "--threads=1",
      "--lag-in-frames=0", "--usage=0", "--monochrome", "--lossless=1", "--enable-qm=0",
      "--enable-deblocking=0", "--enable-gdf=0", "--enable-cdef=0",
      "--enable-restoration=0", "--enable-ccso=0", "--enable-cfl-intra=0",
      "--enable-mhccp=0", "--enable-palette=0", "--enable-intrabc=0", "--enable-fsc=0",
      "--enable-ist=0", "--enable-intra-dip=0", "--enable-mrls=0", "--enable-sdp=0",
      "--enable-extended-sdp=0", "--enable-tcq=0", "--enable-parity-hiding=0",
      "--enable-rect-partitions=0", "--enable-ext-partitions=0",
      "--enable-uneven-4way-partitions=0", "--enable-idtx-intra=0",
      "--enable-angle-delta=0", "--enable-smooth-intra=0", "--enable-paeth-intra=0",
      "--enable-tx-partition=0"], "0", "0",
     {"format": "rawvideo", "width": 16, "height": 16}),
    ("syn-wienerns-intra-128x128", "testsrc2=size=128x128:rate=1:duration=1", "yuv420p", "--i420", BASE + ["--enable-restoration=1", "--enable-wiener-nonsep=1"], "2", "90"),
    ("syn-wienerns-422-intra-128x128", "testsrc2=size=128x128:rate=1:duration=1", "yuv422p", "--i422", BASE + ["--enable-intrabc=0", "--enable-restoration=1", "--enable-wiener-nonsep=1"], "2", "90"),
    ("syn-wienerns-444-intra-128x128", "testsrc2=size=128x128:rate=1:duration=1", "yuv444p", "--i444", BASE + ["--enable-restoration=1", "--enable-wiener-nonsep=1"], "2", "90"),
    ("syn-wienerns-tilerows-intra-256x128", "testsrc2=size=256x128:rate=1:duration=1", "yuv420p", "--i420", BASE + ["--enable-restoration=1", "--enable-wiener-nonsep=1", "--tile-rows=1"], "2", "120"),
    ("syn-cfl-444-tilerows-intra-256x128", "testsrc2=size=256x128:rate=1:duration=1", "yuv444p", "--i444", ["--i444", "--tile-rows=1"], "2", "180"),
    ("syn-pcwiener-intra-128x128", "testsrc2=size=128x128:rate=1:duration=1", "yuv420p", "--i420", BASE + ["--enable-restoration=1", "--enable-pc-wiener=1"], "2", "90"),
    ("syn-gdf-intra-128x128", "testsrc2=size=128x128:rate=1:duration=1", "yuv420p", "--i420", BASE + ["--enable-gdf=1"], "2", "90"),
    ("syn-warp-inter-128x128", "testsrc2=size=128x128:rate=1:duration=4", "yuv420p", "--i420", BASE + ["--enable-warped-motion=1", "--enable-global-motion=1"], "2", "90"),
    ("syn-luma-palette2-row-mono-16x16-q0",
     "nullsrc=s=16x16:d=1,geq=lum='if(eq(Y,0),32,224)':cb=128:cr=128",
     "yuv420p", "--i420",
     ["--monochrome", "--tune-content=screen", "--limit=1", "--usage=0",
      "--enable-intrabc=0", "--enable-sdp=0", "--enable-extended-sdp=0",
      "--enable-intra-dip=0", "--enable-ibp=0", "--enable-mrls=0",
      "--enable-angle-delta=0", "--enable-fsc=0", "--enable-ist=0",
      "--enable-idtx-intra=0", "--enable-smooth-intra=0", "--enable-paeth-intra=0",
      "--enable-intra-edge-filter=0", "--enable-cfl-intra=0", "--enable-mhccp=0",
      "--enable-cctx=0", "--enable-tcq=0", "--enable-parity-hiding=0",
      "--enable-qm=0", "--enable-chroma-deltaq=0", "--enable-deblocking=0",
      "--enable-cdef=0", "--enable-restoration=0", "--enable-gdf=0",
      "--enable-ccso=0", "--enable-pc-wiener=0", "--enable-palette=1"],
     "8", "0", {"format": "rawvideo", "width": 16, "height": 16}),
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
    ("syn-2frame-extendwarp-global-48x48-q185", "", "yuv420p", "--i420",
     ["--limit=2", "--passes=1", "--lag-in-frames=0",
      "--kf-min-dist=16", "--kf-max-dist=16", "--enable-global-motion=1",
      "--enable-warped-motion=1", "--enable-warp-causal=0", "--enable-warp-delta=1",
      "--enable-six-param-warp-delta=0", "--enable-warp-extend=1", "--enable-tip=0",
      "--enable-skip-mode=0", "--enable-gdf=0", "--enable-restoration=0",
      "--enable-cdef=0", "--enable-deblocking=0", "--enable-ccso=0",
      "--enable-bawp=0", "--enable-interintra-comp=0", "--enable-ref-frame-mvs=0",
      "--enable-refmvbank=0", "--enable-tcq=0", "-y"], "2", "185",
     {"format": "y4m", "encode_env": {
         "SPLOT_DISABLE_SAME_REF_COMPOUND": "1",
         "SPLOT_DISABLE_WARP_DELTA_TRIAL": "1",
         "SPLOT_DISABLE_WARPMV_MODE": "1",
         "SPLOT_DISABLE_NEARMV_MODE": "1",
     }, "ffmpeg_args": [
         "-f", "lavfi", "-i", "testsrc2=size=48x48:rate=1:duration=1",
         "-filter_complex",
         "[0:v]split=2[f0][rotate_input];[rotate_input]rotate=angle=0.06:fillcolor=gray:bilinear=1,split=2[bg][pin];[pin]crop=w=12:h=12:x=27:y=18[patch];[bg][patch]overlay=x=24:y=18[f1];[f0][f1]concat=n=2:v=1:a=0,format=yuv420p[v]",
         "-map", "[v]", "-r", "1", "-pix_fmt", "yuv420p",
     ]}),
    ("syn-reduced-txpart-d135-intra-64x64-q160", "", "yuv420p", "--i420",
     TX_PARTITION_INTRA, "0", "160", {
         "format": "rawvideo", "width": 64, "height": 64,
         "decode_fixture": "syn-lossless-nondc-luma-d135-chroma-follow-intra-64x64.ivf",
         "decode_args": ["--codec=av2", "--rawvideo", "--i420", "--output-bit-depth=8",
                         "--limit=1"],
     }),
    ("syn-2frame-reduced-txpart-inter-64x64-q160", "", "yuv420p", "--i420",
     TX_PARTITION_INTER, "0", "160", {
         "format": "rawvideo", "width": 64, "height": 64,
         "decode_fixture": "syn-lossless-nondc-luma-d135-chroma-follow-intra-64x64.ivf",
         "decode_args": ["--codec=av2", "--rawvideo", "--i420", "--output-bit-depth=8",
                         "--limit=1"],
         "ffmpeg_args": [
             "-f", "rawvideo", "-pix_fmt", "yuv420p", "-s", "64x64", "-r", "1",
             "-i", "{decoded}", "-filter_complex",
             "[0:v]split=2[f0][f1in];[f1in]geq=lum='clip(p(X,Y)+if(lt(Y,32),4,0),0,255)':cb='p(X,Y)':cr='p(X,Y)'[f1];[f0][f1]concat=n=2:v=1:a=0[out]",
             "-map", "[out]", "-frames:v", "2", "-f", "rawvideo",
         ],
     }),
    ("syn-sb256-intra-129x16-q180", "color=c=gray:size=129x16:rate=1:duration=1",
     "yuv420p", "--i420", SB256_INTRA, "0", "180", {
         "format": "y4m",
         "control_replace": {"--sb-size=256": "--sb-size=128"},
     }),
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
    "syn-luma-palette2-row-mono-16x16-q0": {
        "avm_revision": "457cd58681a747465661baccb1f32095bc5b7774",
        "ffmpeg_version": "8.1.2",
        "source_sha256": "d004e017048deb20e7e68c13d75959fc007c0edd87b66336d59f51b441fd800d",
        "ivf_sha256": "15f7683df284453f314f169182e185590a892ca6e612cfdeb9a1b86949e0cc53",
        "obu_sha256": "f5cf30af52391c027f3b060ee9071ffbda4cb1f9454a6b1e55fd57200e1f3e06",
        "avm_native_raw_sha256": "418ecf79e4b2564419678cb25c28cbbf0259bf4602dd54f3e2868529d4df261d",
        "avmenc_sha256": "952af54f32109602ee9315dd36cc5ec3d01adcaae42f2a7d1cfdd11e92815b2e",
        "avmdec_sha256": "70342f860be8ccb277c6d719c631f3c054f0ec52b1bcf40c5db00941b8a56ab2",
        "reproducibility_runs": 2,
    },
    "syn-golomb-lossless-mono-16x16": {
        "avm_revision": "457cd58681a747465661baccb1f32095bc5b7774",
        "ffmpeg_version": "8.1.2",
        "source_sha256": "df7377a93c25c39d409e22fae2833c6754aad2654cc56e550751f9df27f21c3a",
        "ivf_sha256": "2e49ce31dc92b05747ff10ebed9b256f450cbac8ff69001f31a19c452de7716b",
        "obu_sha256": "c3007e484b13b81bfea7d7e2f148db22ece6d847aa12c7d6920adaf05e9484e3",
        "avm_i420_raw_sha256": "673f62a095962fe0413e3fa044af24b3f8ec6919a0b61132187e7257ad38afb1",
        "provenance_sha256": "8436f8216811c27854b52fa8837aa91f7d878301f571be7b02dd52d90a0e8721",
        "trace_sha256": "6a9933e1cbb196fface5ca35cd0dfed61a03ea40723ca6af62c8741eb59cd312",
        "avmenc_sha256": "952af54f32109602ee9315dd36cc5ec3d01adcaae42f2a7d1cfdd11e92815b2e",
        "avmdec_sha256": "70342f860be8ccb277c6d719c631f3c054f0ec52b1bcf40c5db00941b8a56ab2",
        "reproducibility_runs": 2,
    },
    "syn-2frame-extendwarp-global-48x48-q185": {
        "avm_revision": "457cd58681a747465661baccb1f32095bc5b7774",
        "ffmpeg_version": "8.1.2",
        "source_sha256": "3481905383458adb2131b710e9c6e5d5f78f4a76d525a495085d411bf6be4a29",
        "ivf_sha256": "48b7d643f7351344cb8cad1e3d3092bf29a9eea1075ffa71656111c7bffbce5c",
        "obu_sha256": "f4c510e520fa22c2a851dbc27198985c7c04970d0cc0a17dbe6f73663c387372",
        "avm_i420_raw_sha256": "9c432a75b318d6f811c11399e16fdd4f28367753a597de8858cd4b8a13efed36",
        "instrumentation_sha256": "45b59603751de012c9b5a29073fdd60da86bbd72906ba9ae55c56c3783c8811f",
        "reproducibility_runs": 2,
    },
    "syn-reduced-txpart-d135-intra-64x64-q160": {
        "avm_revision": "457cd58681a747465661baccb1f32095bc5b7774",
        "source_sha256": "5fffbdc79140da104a1721ed649130f0a2409fadeeb58632cdba54a1add778a1",
        "ivf_sha256": "fb5d3cd1abeb033e8de489f9daa90dbfbbc5c624ab92fb927c860d860420dfa7",
        "avm_i420_raw_sha256": "608619fe3b10c3464841b2269baa417c29d48d7e4793ca38913a68c107caa0a9",
        "trace_sha256": "cacde0da746b7fd66cb0326ca64a24f42631e843086b1b4c6017a70e749d33b3",
        "full_set_control_ivf_sha256": "01b26c7f9468a079ffb798398aad63b2224354498790189d7260ffccee64db59",
        "reproducibility_runs": 2,
    },
    "syn-2frame-reduced-txpart-inter-64x64-q160": {
        "avm_revision": "457cd58681a747465661baccb1f32095bc5b7774",
        "ffmpeg_version": "8.1.2",
        "source_sha256": "9e9707974cbbb48461ae6e9e4e512918ff5057e2d3ed068c862f67a5881074d3",
        "ivf_sha256": "06fbf0a02c027d3336e9c90e1b8f35c0ed21c367196129266b14504a34a0fe96",
        "avm_i420_raw_sha256": "9e9707974cbbb48461ae6e9e4e512918ff5057e2d3ed068c862f67a5881074d3",
        "trace_sha256": "32453fd5d10f17eec9f69551cb3408d2794e91bfb658965ac64761bb538ab2a3",
        "full_set_control_ivf_sha256": "176aa168708fd3afbe5b842ef364bafa19d1e07c4b58ddcdad6793cdec020ec6",
        "reproducibility_runs": 2,
    },
    "syn-sb256-intra-129x16-q180": {
        "avm_revision": "457cd58681a747465661baccb1f32095bc5b7774",
        "ffmpeg_version": "9.0.1",
        "source_sha256": "097e4a3a4394bbae09c548d6b1a44b2ea8a3ebe03cabafcb2ce59ff220698898",
        "ivf_sha256": "bfdf8c13d29f022dfbc1abd03f69efd51d6361bb51ae600af7812a2df11fedaf",
        "obu_sha256": "dca7cac0bd6e17a66e54986d8e80dc89331abeb12e07f0590c1635eb6f917db6",
        "avm_i420_raw_sha256": "6304c67c4e126342e56bc55b26ef1750444fc3e55cde4416f7d385aba4226cc6",
        "avm_trace_sha256": "c7a1948c4b0574e20bcf8ed854dafc27ca541a09ab65aa0c766ad8fe3d3b39f5",
        "control_ivf_sha256": "64b3c75cb0cab6cd00db8461f70ece148949321436cce81ee69198887f321bcc",
        "control_obu_sha256": "34577a0424f7da6e6185bee73a5ec9d1b8b6482bf9663604f975589b21fe07c3",
        "control_avm_i420_raw_sha256": "6304c67c4e126342e56bc55b26ef1750444fc3e55cde4416f7d385aba4226cc6",
        "control_avm_trace_sha256": "b97139cfb010355e097fa739755ee84aa5b92e98f42cac5c875b839c1141ac56",
        "instrumentation_sha256": "6edc38965cea57d33cd72b483d28c76672a4cf761d84798a49af13fdf820f3c1",
        "recipe_sha256": "4d46233bfbc84b44618f65abad233bdf3ee8733e4fd87c63987640b0015cf72d",
        "avmenc_sha256": "a152b44a783f37479500cbf468ed5fdc8209f8da700bbe8ce290315941f36a63",
        "avmdec_sha256": "e867227ee71a133aa0410f9075e604ae6aaef01c0e50f2dc74aa9b61a54547f5",
        "reproducibility_runs": 2,
    },
}


def cmd_find(_):
    for n in ("avmenc", "avmdec"):
        print(f"{n}: {find(n)}")


def cmd_hashes(args):
    avmdec = find("avmdec")
    rows = []
    for name in sorted(f for f in os.listdir(VALID) if f.endswith(".ivf")):
        ivf = os.path.join(VALID, name)
        raw = "/tmp/_o.raw"
        if os.path.exists(raw):
            os.remove(raw)
        avm_ok = (run([avmdec, "--i420", "--rawvideo", "-o", raw, ivf]).returncode == 0
                  and os.path.exists(raw))
        avm = sha(raw) if avm_ok else None
        rows.append({"id": name[:-4], "ivf_sha256": sha(ivf), "avm_raw_sha256": avm})
        print(f"  {'AVM_OK' if avm_ok else 'AVM_ERROR':9s} {name}")
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
    for row in selected:
        fid, src, pix, inflag, flags, cpu, qp = row[:7]
        source = row[7] if len(row) == 8 else {"format": "y4m"}
        expected = PINNED_RECIPE_HASHES.get(fid)
        avmdec = None
        if expected and "avm_revision" in expected:
            avm_root = os.environ.get("AVM_ROOT") or os.path.dirname(os.path.dirname(avmenc))
            revision = run_checked(["git", "-C", avm_root, "rev-parse", "HEAD"])
            revision = revision.stdout.decode().strip()
            if revision != expected["avm_revision"]:
                sys.exit(f"error: {fid} requires AVM revision {expected['avm_revision']}: "
                         f"found {revision}")
        if (expected and ("avmdec_sha256" in expected or "avm_i420_raw_sha256" in expected)
                or "decode_fixture" in source):
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
            instrumentation = run_checked([
                "git", "-C", avm_root, "diff", "--", expected["instrumentation_source"]
            ]).stdout
            instrumentation_sha256 = hashlib.sha256(instrumentation).hexdigest()
            if instrumentation_sha256 != expected["instrumentation_sha256"]:
                sys.exit(f"error: {fid} requires pinned instrumented AVM: "
                         f"instrumentation={instrumentation_sha256}")
        suffix = ".yuv" if source["format"] == "rawvideo" else ".y4m"
        y4m = os.path.join(stage, fid + suffix)
        source_args = ["-frames:v", "1", "-f", "rawvideo"] if source["format"] == "rawvideo" else []
        decoded = None
        if "decode_fixture" in source:
            decoded = y4m if "ffmpeg_args" not in source else os.path.join(stage, fid + ".decoded.yuv")
            run_checked([avmdec, *source.get("decode_args", []), "-o", decoded,
                         os.path.join(VALID, source["decode_fixture"])])
        if "ffmpeg_args" in source:
            ffmpeg_args = [decoded if arg == "{decoded}" else arg for arg in source["ffmpeg_args"]]
            run_checked(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y",
                         *ffmpeg_args, y4m])
        elif decoded is None:
            run_checked(["ffmpeg", "-loglevel", "error", "-y", "-f", "lavfi", "-i", src,
                         "-pix_fmt", pix, *source_args, y4m])
        ivf = os.path.join(stage, fid + ".ivf")
        size_args = (["-w", str(source["width"]), "-h", str(source["height"])]
                     if source["format"] == "rawvideo" else [])
        encode = [avmenc, "--codec=av2", "--ivf", "-D", "--cpu-used=" + cpu,
                  "--end-usage=q", "--qp=" + qp, "--kf-max-dist=0", "-t", "1",
                  inflag, *size_args, *flags]
        encode_env = os.environ.copy()
        encode_env.update(source.get("encode_env", {}))
        run_checked([*encode, "-o", ivf, y4m], env=encode_env)
        if expected:
            actual_source = sha(y4m)
            actual_ivf = sha(ivf)
            if actual_source != expected["source_sha256"] or actual_ivf != expected["ivf_sha256"]:
                sys.exit(f"error: {fid} differs from pinned AVM {expected['avm_revision']}: "
                         f"source={actual_source}, ivf={actual_ivf}")
            if expected.get("reproducibility_runs") == 2:
                repeated_ivf = os.path.join(stage, fid + ".repeat.ivf")
                run_checked([*encode, "-o", repeated_ivf, y4m], env=encode_env)
                if sha(repeated_ivf) != actual_ivf:
                    sys.exit(f"error: {fid} is not deterministic across two encodes")
                os.remove(repeated_ivf)
            if "obu_sha256" in expected:
                obu = os.path.join(stage, fid + ".obu")
                obu_encode = [avmenc, "--codec=av2", "--obu", "-D", "--cpu-used=" + cpu,
                              "--end-usage=q", "--qp=" + qp, "--kf-max-dist=0", "-t", "1",
                              inflag, *size_args, *flags]
                run_checked([*obu_encode, "-o", obu, y4m], env=encode_env)
                if sha(obu) != expected["obu_sha256"]:
                    sys.exit(f"error: {fid} differs from its pinned raw OBU")
            if "avm_i420_raw_sha256" in expected:
                raw = os.path.join(stage, fid + ".avm.raw")
                decode_flags = (["--codec=av2", "--threads=1"]
                                if "avm_trace_sha256" in expected else [])
                decoded = run_checked([avmdec, *decode_flags, "--i420", "--rawvideo",
                                       "-o", raw, ivf])
                if sha(raw) != expected["avm_i420_raw_sha256"]:
                    sys.exit(f"error: {fid} differs from its pinned AVM raw output")
                if "avm_trace_sha256" in expected:
                    actual_trace = hashlib.sha256(decoded.stderr).hexdigest()
                    if actual_trace != expected["avm_trace_sha256"]:
                        sys.exit(f"error: {fid} differs from its pinned AVM trace")
            if "avm_native_raw_sha256" in expected:
                raw = os.path.join(stage, fid + ".avm-native.raw")
                run_checked([avmdec, "--rawvideo", "-o", raw, ivf])
                if sha(raw) != expected["avm_native_raw_sha256"]:
                    sys.exit(f"error: {fid} differs from its pinned AVM native raw output")
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
            if "trace_sha256" in expected:
                print(f"  {fid}: isolated AVM trace {expected['trace_sha256']}")
            if "full_set_control_ivf_sha256" in expected:
                print(f"  {fid}: full-set control IVF "
                      f"{expected['full_set_control_ivf_sha256']}")
            if "control_replace" in source:
                replacements = source["control_replace"]
                if not all(old in encode for old in replacements):
                    sys.exit(f"error: {fid} control replacement source is absent")
                control_encode = [replacements.get(arg, arg) for arg in encode]
                control_ivf = os.path.join(stage, fid + ".control.ivf")
                run_checked([*control_encode, "-o", control_ivf, y4m], env=encode_env)
                if sha(control_ivf) != expected["control_ivf_sha256"]:
                    sys.exit(f"error: {fid} differs from its pinned control IVF")
                if expected.get("reproducibility_runs") == 2:
                    repeated_control = os.path.join(stage, fid + ".control.repeat.ivf")
                    run_checked([*control_encode, "-o", repeated_control, y4m], env=encode_env)
                    if sha(repeated_control) != expected["control_ivf_sha256"]:
                        sys.exit(f"error: {fid} control is not deterministic")
                    os.remove(repeated_control)
                if "control_obu_sha256" in expected:
                    control_obu_encode = [replacements.get(arg, arg) for arg in obu_encode]
                    control_obu = os.path.join(stage, fid + ".control.obu")
                    run_checked([*control_obu_encode, "-o", control_obu, y4m], env=encode_env)
                    if sha(control_obu) != expected["control_obu_sha256"]:
                        sys.exit(f"error: {fid} differs from its pinned control OBU")
                if "control_avm_i420_raw_sha256" in expected:
                    control_raw = os.path.join(stage, fid + ".control.avm.raw")
                    control_decoded = run_checked([
                        avmdec, "--codec=av2", "--threads=1", "--i420", "--rawvideo",
                        "-o", control_raw, control_ivf,
                    ])
                    if sha(control_raw) != expected["control_avm_i420_raw_sha256"]:
                        sys.exit(f"error: {fid} differs from its pinned control AVM output")
                    if "control_avm_trace_sha256" in expected:
                        actual_trace = hashlib.sha256(control_decoded.stderr).hexdigest()
                        if actual_trace != expected["control_avm_trace_sha256"]:
                            sys.exit(f"error: {fid} differs from its pinned control AVM trace")
                print(f"  {fid}: sequence-SB control IVF {expected['control_ivf_sha256']}")
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
