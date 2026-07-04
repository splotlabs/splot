#!/usr/bin/env python3
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
"""Generate deterministic ffmpeg `lavfi` Y4M sources for new decoder fixtures.

LOCAL ONLY. Writes small (<=64x64, <=4 frame), 8/10/12-bit 4:2:0 synthetic Y4M
sources to `target/decoder-fixtures/source-y4m/` (gitignored, never
committed). These sources are raw material for a human to later feed through
`encode_fixture.py` and hand-pick a vetted `.ivf` to add to the committed
corpus under `tests/conformance/vectors/valid/`; this script does not encode
or write into `tests/`.

Patterns (each name-tagged with the feature pressure it is intended to probe):

- `flat`         solid mid-gray: minimal/all-zero residual pressure.
- `testsrc2`     ffmpeg's built-in test pattern: general intra/AC-residual
                 pressure across many block structures at once.
- `gradient`     `lavfi` `gradients` source: smooth low-frequency directional
                 intra-prediction pressure.
- `checkerboard` high-frequency alternating blocks via `geq`: worst-case AC
                 residual / transform-size pressure.
- `movingsquare` a translating filled box across frames: inter/motion-search
                 pressure (only meaningful for `--frames >= 2`).

Bit depth is realized via `-pix_fmt yuv420p{,10le,12le}` (`-strict -1` is
required by ffmpeg's y4m muxer for the 10/12-bit formats).
"""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_OUT_DIR = REPO_ROOT / "target/decoder-fixtures/source-y4m"
DEFAULT_SIZES = ("32x32", "64x64")
DEFAULT_BIT_DEPTHS = (8, 10, 12)
DEFAULT_FRAME_COUNTS = (1, 4)

PIX_FMT_BY_DEPTH = {8: "yuv420p", 10: "yuv420p10le", 12: "yuv420p12le"}

FEATURE_PRESSURE = {
    "flat": "minimal/all-zero residual (skip-branch pressure)",
    "testsrc2": "general intra + AC-residual pressure across many block structures",
    "gradient": "smooth low-frequency directional intra-prediction pressure",
    "checkerboard": "high-frequency AC residual / transform-size pressure",
    "movingsquare": "inter/motion-search pressure (needs >=2 frames)",
}


def lavfi_filter(pattern: str, size: str, frames: int) -> str | None:
    """Return the `-f lavfi` filter graph for `pattern`, or None if inapplicable."""
    if pattern == "flat":
        return f"color=c=gray:s={size}:d={frames}:r=1"
    if pattern == "testsrc2":
        return f"testsrc2=s={size}:d={frames}:r=1"
    if pattern == "gradient":
        return f"gradients=s={size}:d={frames}:r=1:n=4"
    if pattern == "checkerboard":
        return (
            f"color=c=black:s={size}:d={frames}:r=1,"
            "geq=lum='if(mod(floor(X/8)+floor(Y/8)\\,2)\\,235\\,16)':cb=128:cr=128"
        )
    if pattern == "movingsquare":
        if frames < 2:
            return None
        return (
            f"color=c=black:s={size}:d={frames}:r=1,"
            f"drawbox=x='8*mod(t\\,4)':y=8:w=8:h=8:color=white:t=fill"
        )
    return None


def generate_one(
    ffmpeg_bin: str, pattern: str, size: str, bit_depth: int, frames: int, out_dir: Path
) -> tuple[Path, str] | None:
    """Generate one Y4M source; return `(path, feature_pressure)` or None if skipped."""
    filt = lavfi_filter(pattern, size, frames)
    if filt is None:
        return None
    pix_fmt = PIX_FMT_BY_DEPTH[bit_depth]
    out_path = out_dir / f"{pattern}-{size}-{frames}f-{bit_depth}bit.y4m"
    cmd = [
        ffmpeg_bin,
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "lavfi",
        "-i",
        filt,
        "-pix_fmt",
        pix_fmt,
        "-strict",
        "-1",
        "-y",
        str(out_path),
    ]
    result = subprocess.run(cmd, capture_output=True, timeout=60)
    if result.returncode != 0:
        print(
            f"error: ffmpeg failed for {out_path.name}: {result.stderr.decode(errors='replace').strip()}",
            file=sys.stderr,
        )
        return None
    return (out_path, FEATURE_PRESSURE[pattern])


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--ffmpeg", default="ffmpeg", help="ffmpeg executable (default: %(default)s).")
    parser.add_argument(
        "--out-dir", type=Path, default=DEFAULT_OUT_DIR, help="Output dir for generated Y4M (default: %(default)s)."
    )
    parser.add_argument(
        "--patterns",
        nargs="+",
        default=sorted(FEATURE_PRESSURE),
        choices=sorted(FEATURE_PRESSURE),
        help="Which patterns to generate (default: all).",
    )
    parser.add_argument("--sizes", nargs="+", default=list(DEFAULT_SIZES), help="WxH sizes, each side <= 64.")
    parser.add_argument(
        "--bit-depths", nargs="+", type=int, default=list(DEFAULT_BIT_DEPTHS), choices=[8, 10, 12], help="Bit depths."
    )
    parser.add_argument(
        "--frame-counts", nargs="+", type=int, default=list(DEFAULT_FRAME_COUNTS), help="Frame counts, each <= 4."
    )
    args = parser.parse_args(argv)

    for size in args.sizes:
        width, _, height = size.partition("x")
        if not (width.isdigit() and height.isdigit() and int(width) <= 64 and int(height) <= 64):
            print(f"error: size {size!r} must be WxH with both sides <= 64", file=sys.stderr)
            return 1
    for frames in args.frame_counts:
        if frames < 1 or frames > 4:
            print(f"error: frame count {frames} must be in [1, 4]", file=sys.stderr)
            return 1

    args.out_dir.mkdir(parents=True, exist_ok=True)

    manifest: list[dict] = []
    for pattern in args.patterns:
        for size in args.sizes:
            for bit_depth in args.bit_depths:
                for frames in args.frame_counts:
                    generated = generate_one(args.ffmpeg, pattern, size, bit_depth, frames, args.out_dir)
                    if generated is None:
                        continue
                    out_path, pressure = generated
                    manifest.append(
                        {
                            "path": str(out_path),
                            "pattern": pattern,
                            "size": size,
                            "bit_depth": bit_depth,
                            "frames": frames,
                            "feature_pressure": pressure,
                        }
                    )

    print(f"Generated {len(manifest)} source(s) under {args.out_dir}:\n")
    for entry in manifest:
        print(
            f"  {Path(entry['path']).name:40s} pattern={entry['pattern']:14s} "
            f"{entry['size']:8s} {entry['bit_depth']:2d}bit {entry['frames']}f  -- {entry['feature_pressure']}"
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
