#!/usr/bin/env python3
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
"""Refresh AVM oracle hash data for `tests/conformance/decoder-oracle.toml`.

LOCAL ONLY. For every `.ivf` under `tests/conformance/vectors/valid/`, this
script:

1. Runs `avmdec --i420 --rawvideo` (via `find_avm.find_avmdec`) to a scratch
   file under `target/decoder-fixtures/` (gitignored, never committed).
2. Computes the whole-stream raw sha256, splits the output into per-shown-frame
   sha256 (frame size inferred from `w*h*3//2` for 8-bit or `w*h*3` for 16-bit
   I420 4:2:0, matched against the observed output size), and the `.ivf`
   sha256.
3. Probes `splot decode --output-format raw` (default
   `target/release/splot`, override with `$SPLOT_BIN`) and classifies the
   vector as `must_pass` (splot raw sha256 == AVM raw sha256), `xfail_splot`
   (splot fails closed with `decode/unsupported-feature`), `mismatch` (splot
   succeeds but disagrees with AVM — a real bug), or `splot_error` (any other
   non-zero exit).

This script does not write `tests/conformance/decoder-oracle.toml` itself; it
only produces/refreshes the AVM+splot hash data a human folds into that
manifest. Output is a JSON report on stdout, and also written to `--out` if
given.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import struct
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from find_avm import AvmNotFoundError, find_avmdec  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_VECTORS_DIR = REPO_ROOT / "tests/conformance/vectors/valid"
DEFAULT_SCRATCH_DIR = REPO_ROOT / "target/decoder-fixtures/oracle-scratch"
DEFAULT_SPLOT_BIN = REPO_ROOT / "target/release/splot"
RUN_TIMEOUT_SECONDS = 120


def sha256_file(path: Path) -> str:
    """Return the lowercase hex sha256 of a file's contents."""
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 16), b""):
            digest.update(chunk)
    return digest.hexdigest()


def sha256_bytes(data: bytes) -> str:
    """Return the lowercase hex sha256 of an in-memory byte string."""
    return hashlib.sha256(data).hexdigest()


def read_ivf_dims(path: Path) -> tuple[int, int, int] | None:
    """Return `(width, height, coded_frame_count)` from an IVF header, or None."""
    with path.open("rb") as handle:
        header = handle.read(32)
    if len(header) < 32 or header[:4] != b"DKIF":
        return None
    width = struct.unpack("<H", header[12:14])[0]
    height = struct.unpack("<H", header[14:16])[0]
    coded_frames = struct.unpack("<I", header[24:28])[0]
    return (width, height, coded_frames)


def infer_frame_layout(width: int, height: int, output_size: int) -> tuple[int, int] | None:
    """Infer `(bytes_per_sample, frame_bytes)` for I420 4:2:0 output.

    8-bit I420 is `w*h*3//2` bytes/frame; 16-bit-container 10/12-bit I420 is
    `w*h*3` bytes/frame (2 bytes/sample). Prefers the 16-bit layout when the
    output size is evenly divisible by both, since every 16-bit-frame size is
    also divisible by the 8-bit frame size.
    """
    frame_bytes_8bit = width * height * 3 // 2
    frame_bytes_16bit = width * height * 3
    if frame_bytes_8bit <= 0 or output_size <= 0:
        return None
    if output_size % frame_bytes_16bit == 0:
        return (2, frame_bytes_16bit)
    if output_size % frame_bytes_8bit == 0:
        return (1, frame_bytes_8bit)
    return None


def run(cmd: list[str], timeout: int = RUN_TIMEOUT_SECONDS) -> subprocess.CompletedProcess:
    """Run a subprocess, capturing output, tolerating timeouts as a failure."""
    try:
        return subprocess.run(cmd, capture_output=True, timeout=timeout)
    except subprocess.TimeoutExpired as exc:
        return subprocess.CompletedProcess(
            cmd, returncode=-99, stdout=b"", stderr=f"TIMEOUT: {exc}".encode()
        )


def decode_with_avm(avmdec: Path, ivf_path: Path, out_path: Path) -> bytes | None:
    """Decode `ivf_path` with AVM to raw I420 and return the output bytes."""
    result = run([str(avmdec), "--i420", "--rawvideo", "-o", str(out_path), str(ivf_path)])
    if result.returncode != 0 or not out_path.exists():
        return None
    return out_path.read_bytes()


def probe_splot(splot_bin: Path, ivf_path: Path, out_path: Path) -> dict:
    """Run `splot decode --output-format raw` and classify its outcome."""
    result = run([str(splot_bin), "decode", "--output-format", "raw", "-o", str(out_path), str(ivf_path)])
    info: dict = {"exit_code": result.returncode, "raw_sha256": None}
    if result.returncode == 0 and out_path.exists():
        info["raw_sha256"] = sha256_file(out_path)
        return info

    json_out_path = out_path.with_suffix(out_path.suffix + ".xfail")
    json_result = run(
        [str(splot_bin), "decode", "--json", "--output-format", "raw", "-o", str(json_out_path), str(ivf_path)]
    )
    diagnostic = None
    for candidate in (json_result.stdout, json_result.stderr):
        try:
            diagnostic = json.loads(candidate.decode("utf-8", errors="replace"))
            break
        except (json.JSONDecodeError, UnicodeDecodeError):
            continue
    if diagnostic:
        info["rule_id"] = diagnostic.get("rule_id")
        info["unsupported_reason"] = diagnostic.get("unsupported_reason")
        info["matrix_row"] = diagnostic.get("matrix_row")
        info["feature_id"] = diagnostic.get("feature_id")
    else:
        info["stderr_excerpt"] = result.stderr.decode("utf-8", errors="replace")[:200].strip()
    return info


def classify(avm_data: bytes | None, splot_info: dict) -> str:
    """Classify a vector given AVM output bytes and the splot probe result."""
    if avm_data is None:
        return "avm_error"
    avm_sha = sha256_bytes(avm_data)
    if splot_info.get("raw_sha256") == avm_sha:
        return "must_pass"
    if splot_info.get("raw_sha256") is not None:
        return "mismatch"
    if splot_info.get("rule_id") == "decode/unsupported-feature":
        return "xfail_splot"
    return "splot_error"


def build_report(vectors_dir: Path, scratch_dir: Path, splot_bin: Path, avmdec: Path) -> list[dict]:
    """Build the full oracle hash report for every `.ivf` in `vectors_dir`."""
    scratch_dir.mkdir(parents=True, exist_ok=True)
    rows = []
    for ivf_path in sorted(vectors_dir.glob("*.ivf")):
        dims = read_ivf_dims(ivf_path)
        width, height, coded_frames = dims if dims else (None, None, None)

        avm_out = scratch_dir / f"{ivf_path.name}.avm.raw"
        avm_data = decode_with_avm(avmdec, ivf_path, avm_out)

        splot_out = scratch_dir / f"{ivf_path.name}.splot.raw"
        splot_info = probe_splot(splot_bin, ivf_path, splot_out)

        row: dict = {
            "id": ivf_path.stem,
            "path": f"vectors/valid/{ivf_path.name}",
            "width": width,
            "height": height,
            "coded_frames": coded_frames,
            "ivf_sha256": sha256_file(ivf_path),
        }

        if avm_data is not None:
            layout = infer_frame_layout(width, height, len(avm_data)) if width and height else None
            row["avm_raw_i420_sha256"] = sha256_bytes(avm_data)
            if layout:
                bytes_per_sample, frame_bytes = layout
                row["bytes_per_sample"] = bytes_per_sample
                row["shown_frames"] = len(avm_data) // frame_bytes
                row["avm_raw_i420_frame_sha256"] = [
                    sha256_bytes(avm_data[i * frame_bytes : (i + 1) * frame_bytes])
                    for i in range(len(avm_data) // frame_bytes)
                ]
            else:
                row["frame_layout_error"] = (
                    f"output size {len(avm_data)} not divisible by inferred frame size "
                    f"for {width}x{height}"
                )
        else:
            row["avm_error"] = True

        row["splot"] = splot_info
        row["status"] = classify(avm_data, splot_info)
        rows.append(row)

        reason = splot_info.get("unsupported_reason") or splot_info.get("stderr_excerpt")
        suffix = f" reason={reason}" if reason else ""
        print(
            f"{row['status']:12s} {ivf_path.name:56s} {width}x{height} "
            f"coded={coded_frames} shown={row.get('shown_frames')}{suffix}",
            file=sys.stderr,
        )
    return rows


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument(
        "--vectors-dir", type=Path, default=DEFAULT_VECTORS_DIR, help="Directory of committed .ivf vectors to probe."
    )
    parser.add_argument(
        "--scratch-dir",
        type=Path,
        default=DEFAULT_SCRATCH_DIR,
        help="Scratch dir for decoded raw output (gitignored under target/).",
    )
    parser.add_argument("--out", type=Path, default=None, help="Also write the JSON report to this path.")
    args = parser.parse_args(argv)

    splot_bin = Path(os.environ.get("SPLOT_BIN", str(DEFAULT_SPLOT_BIN)))
    if not splot_bin.is_file():
        print(
            f"error: splot binary not found at {splot_bin}. Build it first "
            f"(`cargo build --release -p splot-cli`) or set $SPLOT_BIN.",
            file=sys.stderr,
        )
        return 1

    try:
        avmdec = find_avmdec()
    except AvmNotFoundError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1

    if not args.vectors_dir.is_dir():
        print(f"error: vectors dir not found: {args.vectors_dir}", file=sys.stderr)
        return 1

    rows = build_report(args.vectors_dir, args.scratch_dir, splot_bin, avmdec)

    counts: dict[str, int] = {}
    for row in rows:
        counts[row["status"]] = counts.get(row["status"], 0) + 1
    print(f"\nSUMMARY: {json.dumps(dict(sorted(counts.items())))}", file=sys.stderr)

    report = {"avmdec": str(avmdec), "splot_bin": str(splot_bin), "vectors": rows}
    output_json = json.dumps(report, indent=2)
    print(output_json)
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(output_json + "\n")
        print(f"wrote {args.out}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
