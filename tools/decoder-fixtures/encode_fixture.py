#!/usr/bin/env python3
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
"""Encode a Y4M source into a deterministic AV2 `.ivf` fixture candidate with AVM.

LOCAL ONLY. Wraps `avmenc` (via `find_avm.find_avmenc`) with the flag set the
committed corpus uses for reproducible fixtures: `-D` (debug/deterministic
mode), single-threaded, a fixed `--qp`, and a `--kf-max-dist` that lets the
caller pick "single keyframe" (0) or "allow inter frames" (>= frame count).

This script never writes into `tests/`. It prints the resolved dimensions,
frame count, output size, and a sanitized `avmenc` command summary, then tells
the developer where a human should place a vetted fixture
(`tests/conformance/vectors/valid/<name>.ivf`) plus the manifest entry they
still need to add by hand to `tests/conformance/manifest.toml` (validator
outcome) and `tests/conformance/decoder-oracle.toml` (decode-output oracle,
refreshed via `update_oracle_hashes.py`).

Determinism is verified by encoding twice and comparing sha256 of the two
`.ivf` outputs.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from find_avm import AvmNotFoundError, find_avmenc  # noqa: E402
from update_oracle_hashes import sha256_file  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_OUT_DIR = REPO_ROOT / "target/decoder-fixtures/encoded-ivf"
MAX_WIDTH_HEIGHT = 192
MAX_FRAMES = 4
MAX_BYTES = 64 * 1024

Y4M_HEADER_RE = re.compile(
    rb"^YUV4MPEG2\s+W(?P<w>\d+)\s+H(?P<h>\d+)\s+F(?P<fps>\S+)\s+I(?P<interlace>\S+)"
)


def parse_y4m_header(path: Path) -> tuple[int, int, int]:
    """Return `(width, height, bytes_per_sample)` parsed from a Y4M stream header.

    Frame payloads are raw binary, so counting frames must skip exactly
    `frame_bytes` between `FRAME` markers rather than using line-based reads
    (a naive `readline()` scan can desync inside binary pixel data that
    happens to contain `\\n` bytes).
    """
    with path.open("rb") as handle:
        header_line = handle.readline()
    match = re.match(rb"YUV4MPEG2\s+W(\d+)\s+H(\d+)", header_line)
    if not match:
        raise ValueError(f"could not parse Y4M header of {path}: {header_line!r}")
    width, height = int(match.group(1)), int(match.group(2))
    depth_match = re.search(rb"C420p(\d+)", header_line)
    bytes_per_sample = 2 if depth_match and int(depth_match.group(1)) > 8 else 1
    return width, height, bytes_per_sample


def count_y4m_frames(path: Path, width: int, height: int, bytes_per_sample: int) -> int:
    """Count frames in a 4:2:0 Y4M stream by skipping fixed-size frame payloads."""
    frame_bytes = width * height * 3 // 2 * bytes_per_sample
    count = 0
    with path.open("rb") as handle:
        handle.readline()  # stream header
        while True:
            frame_header = handle.readline()
            if not frame_header:
                break
            if not frame_header.startswith(b"FRAME"):
                raise ValueError(f"expected FRAME marker in {path}, got {frame_header!r}")
            payload = handle.read(frame_bytes)
            if len(payload) != frame_bytes:
                raise ValueError(f"truncated frame payload in {path}: got {len(payload)}, want {frame_bytes}")
            count += 1
    return count


def sanitize_path(path: Path) -> str:
    """Return only the filename, never an absolute local path, for summaries."""
    return path.name


def build_avmenc_cmd(
    avmenc: Path,
    source_y4m: Path,
    out_path: Path,
    width: int,
    height: int,
    frames: int,
    qp: int,
    kf_max_dist: int,
    bit_depth: int,
) -> list[str]:
    """Build the deterministic `avmenc` invocation used for fixture candidates."""
    return [
        str(avmenc),
        "-D",
        "--i420",
        "--passes=1",
        "-w",
        str(width),
        "-h",
        str(height),
        f"--limit={frames}",
        "--end-usage=q",
        f"--qp={qp}",
        f"--kf-max-dist={kf_max_dist}",
        "--threads=1",
        f"--bit-depth={bit_depth}",
        "-o",
        str(out_path),
        str(source_y4m),
    ]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("source_y4m", type=Path, help="Input Y4M source (e.g. from gen_sources.py).")
    parser.add_argument("--name", required=True, help="Fixture name stem (no extension); becomes <name>.ivf.")
    parser.add_argument("--qp", type=int, default=100, help="Constant quality level, --qp for avmenc (default: 100).")
    parser.add_argument(
        "--kf-max-dist",
        type=int,
        default=0,
        help="avmenc --kf-max-dist; 0 forces every coded frame to be a keyframe (default: 0).",
    )
    parser.add_argument("--bit-depth", type=int, default=8, choices=[8, 10, 12], help="Codec bit depth.")
    parser.add_argument(
        "--out-dir", type=Path, default=DEFAULT_OUT_DIR, help="Scratch dir for the candidate .ivf (not tests/)."
    )
    args = parser.parse_args(argv)

    if not args.source_y4m.is_file():
        print(f"error: source Y4M not found: {args.source_y4m}", file=sys.stderr)
        return 1

    try:
        avmenc = find_avmenc()
    except AvmNotFoundError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1

    width, height, bytes_per_sample = parse_y4m_header(args.source_y4m)
    frames = count_y4m_frames(args.source_y4m, width, height, bytes_per_sample)
    if width > MAX_WIDTH_HEIGHT or height > MAX_WIDTH_HEIGHT:
        print(f"error: {width}x{height} exceeds the {MAX_WIDTH_HEIGHT}px fixture limit per side", file=sys.stderr)
        return 1
    if frames < 1 or frames > MAX_FRAMES:
        print(f"error: source has {frames} frame(s); fixtures must have 1..{MAX_FRAMES}", file=sys.stderr)
        return 1

    args.out_dir.mkdir(parents=True, exist_ok=True)
    out_path = args.out_dir / f"{args.name}.ivf"
    out_path_2 = args.out_dir / f"{args.name}.determinism-check.ivf"

    cmd = build_avmenc_cmd(
        avmenc, args.source_y4m, out_path, width, height, frames, args.qp, args.kf_max_dist, args.bit_depth
    )
    cmd_2 = build_avmenc_cmd(
        avmenc, args.source_y4m, out_path_2, width, height, frames, args.qp, args.kf_max_dist, args.bit_depth
    )

    for run_cmd, dest in ((cmd, out_path), (cmd_2, out_path_2)):
        result = subprocess.run(run_cmd, capture_output=True, timeout=120)
        if result.returncode != 0 or not dest.exists():
            print(f"error: avmenc failed encoding {dest.name}:", file=sys.stderr)
            print(result.stderr.decode(errors="replace"), file=sys.stderr)
            return 1

    sha_1 = sha256_file(out_path)
    sha_2 = sha256_file(out_path_2)
    out_path_2.unlink()
    if sha_1 != sha_2:
        print(
            f"error: encode is NOT deterministic ({sha_1} != {sha_2}); do not use this fixture candidate",
            file=sys.stderr,
        )
        return 1

    path_basenames = {str(args.source_y4m): sanitize_path(args.source_y4m), str(out_path): sanitize_path(out_path)}
    sanitized_cmd = [path_basenames.get(part, part) for part in cmd]
    sanitized_cmd[0] = "avmenc"

    print(f"Encoded: {out_path}")
    print(f"  dims={width}x{height} frames={frames} bit_depth={args.bit_depth}")
    print(f"  size_bytes={out_path.stat().st_size} (limit {MAX_BYTES})")
    print(f"  sha256={sha_1}")
    print(f"  determinism: OK (two independent encodes match byte-for-byte)")
    print(f"  command: {' '.join(sanitized_cmd)}")
    if out_path.stat().st_size > MAX_BYTES:
        print(f"  WARNING: exceeds the {MAX_BYTES}-byte fixture guideline", file=sys.stderr)

    print()
    print("This candidate was NOT written into tests/. If it looks correct:")
    print(f"  1. cp {out_path} {REPO_ROOT}/tests/conformance/vectors/valid/{args.name}.ivf")
    print("  2. Add a [[vector]] entry to tests/conformance/manifest.toml (validator outcome).")
    print(
        "  3. Re-run tools/decoder-fixtures/update_oracle_hashes.py and fold the new "
        "row into tests/conformance/decoder-oracle.toml (decode-output oracle)."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
