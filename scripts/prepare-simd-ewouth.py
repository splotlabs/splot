# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

"""Prepare the frozen CONF-PUBLIC-VECTORS cases for native SIMD qualification."""

import argparse
import hashlib
import pathlib
import subprocess
import tempfile
import tomllib

CASES = (
    "BlueSky_360p25_v2_qp110_130f.ivf",
    "DOTA2_1920x1080_60_8bit_420_qp135_130f.ivf",
    "Slides1_1920x1080_30fps_8bit_420_screen_qp135_130f.ivf",
    "Noise_Ocean_1920x1080_60fps_qp135_130f.ivf",
    "Noise_Ocean_1920x1080_60fps_filmgrain_qp135_130f.ivf",
    "old_town_cross_1080p50_tiles4x2_qp135_500f.ivf",
    "RitualDance_1920x1080_60fps_10bit_420_qp135_600f.ivf",
)


def prepare(root):
    repo = pathlib.Path(__file__).resolve().parent.parent
    root.mkdir(parents=True, exist_ok=True)
    with (repo / "tests/conformance/ewouth-av2.toml").open("rb") as manifest_file:
        manifest = tomllib.load(manifest_file)
    by_path = {stream["path"]: stream for stream in manifest["stream"]}
    releases = [argument for name in CASES if "release" in by_path[name]
                for argument in ("--fetch", name)]
    subprocess.run(["python3", repo / "scripts/check-external-vectors.py", "ewouth-av2", root,
                    *releases], check=True)
    lines = []
    for name in CASES:
        stream = by_path[name]
        path = root / name
        if "release" not in stream:
            if not path.exists():
                with tempfile.TemporaryDirectory() as clone:
                    subprocess.run(["git", "clone", "--filter=blob:none", "--no-checkout",
                                    manifest["source"], clone], check=True)
                    with path.open("wb") as file:
                        subprocess.run(["git", "-C", clone, "show",
                                        f"{manifest['revision']}:samples/{name}"],
                                       stdout=file, check=True)
            with path.open("rb") as file:
                actual = hashlib.file_digest(file, "sha256").hexdigest()
            if actual != stream["input_sha256"]:
                raise SystemExit(f"{path}: input SHA-256 differs: {actual}.")
        lines.append(f"{name}\t{stream['avm_raw_sha256']}\t{stream['output_frames']}\t{stream['raw_bytes']}\n")
    (root / "simd-cases.tsv").write_text("".join(lines))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=pathlib.Path)
    prepare(parser.parse_args().root)
