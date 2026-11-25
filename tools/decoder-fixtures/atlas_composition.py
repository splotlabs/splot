# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
"""Generate composition fixtures from the project-owned global atlas streams.

Run this script with --stage DIR and --avmdec PATH. The source streams come
from generate.py atlas-fixtures. The oracle used AVM commit
457cd58681a747465661baccb1f32095bc5b7774 (1.0.0-33-g457cd58681). Its
arm64 Release build used Ninja, CONFIG_AV2_ENCODER=0, ENABLE_TESTS=0,
and ENABLE_DOCS=0. AVM decodes native pictures. This script calculates
the composite goldens using limited-range BT.709 and nearest-neighbor.
"""
import argparse
from hashlib import sha256
from pathlib import Path
import struct
import subprocess
from typing import NamedTuple

from atlas import atlas, frame, obus


class Region(NamedTuple):
    x: int
    y: int
    width: int
    height: int


CANVAS = (144, 64)
LAYOUTS = {
    "multistream-side-by-side": (
        (Region(0, 0, 64, 64), Region(64, 0, 80, 64)),
        (Region(4, 2, 88, 64), Region(82, 8, 68, 64)),
    ),
    "alpha-paired": (
        (Region(20, 8, 64, 48), Region(12, 2, 100, 64)),
        (Region(66, 14, 48, 50), Region(48, 0, 108, 68)),
    ),
}
SEED_SHA256 = {
    8: "2e5c2565be54bb7a5c6d51082e4385ba7879c381dd4b4ff2dc8dc5e0d84f7544",
    10: "efb64e2d9bbab5cfa7c3f18a6cdce218350bf29c6b02dacd0ff3bb6faea1f40b",
}


def with_layouts(seed, layouts, alpha, layers):
    output = bytearray()
    replacements = iter(layouts)
    for obu in obus(seed):
        if obu[:2] == b"\xc4\x1f":
            obu = atlas(4 if alpha else 3, layers, rectangles=next(replacements))
        output += frame(obu)
    assert next(replacements, None) is None
    return output


def native_frames(decoder, path, depth, layers):
    command = [decoder, "--all-layers", "--rawvideo", "--i420", f"--output-bit-depth={depth}", "-o", "-", path]
    raw = subprocess.run(command, check=True, stdout=subprocess.PIPE).stdout
    # The 10-bit seed repeats its five presentation times at a closed key.
    # AVM emits two frames from each layer at a delayed output event.
    emission = [0, 1] * 4 if depth == 8 else [0, 1, 0, 1, 0, 0, 1, 1, 0, 1] * 2
    frames = [[], []]
    digest = [sha256(), sha256()]
    pos = 0
    for layer_index in emission:
        width = (64, 80)[layer_index]
        sizes = (width * 64, width * 16, width * 16)
        planes = []
        stride = 1 if depth == 8 else 2
        for count in sizes:
            data = raw[pos:pos + count * stride]
            digest[layer_index].update(data)
            planes.append(struct.unpack(f"<{count}{'B' if depth == 8 else 'H'}", data))
            pos += len(data)
        frames[layer_index].append(planes)
    assert pos == len(raw)
    print("  native SHA-256 per xlayer:", [(x, d.hexdigest()) for x, d in zip(layers, digest)])
    return zip(*frames, strict=True)


def sample(plane, native_width, rect, x, y, subsample):
    # Coordinates are relative to the whole segment, including any clipped part.
    sx = (x - rect.x // subsample) * (native_width // subsample) // (rect.width // subsample)
    sy = (y - rect.y // subsample) * (64 // subsample) // (rect.height // subsample)
    return plane[sy * (native_width // subsample) + sx]


def covers(rect, x, y):
    return rect.x <= x < rect.x + rect.width and rect.y <= y < rect.y + rect.height


def composite(frames, regions, alpha, depth):
    """Evaluate each canvas sample independently in its declared presentation format."""
    planes = []
    alpha_max = 1 << depth
    for plane in range(3):
        scale = 1 if plane == 0 else 2
        samples = []
        black = (16 if plane == 0 else 128) << (depth - 8)
        for y in range(CANVAS[1] // scale):
            for x in range(CANVAS[0] // scale):
                luma_x, luma_y = x * scale, y * scale
                value = black
                if not alpha and covers(regions[0], luma_x, luma_y):
                    value = sample(frames[0][plane], 64, regions[0], x, y, scale)
                if covers(regions[1], luma_x, luma_y):
                    texture = sample(frames[1][plane], 80, regions[1], x, y, scale)
                    opacity = alpha_max
                    if alpha and covers(regions[0], luma_x, luma_y):
                        opacity = sample(frames[0][0], 64, regions[0], luma_x, luma_y, 1)
                    value = (value * (alpha_max - opacity) + texture * opacity + alpha_max // 2) // alpha_max
                samples.append(value)
        planes.append(struct.pack(f"<{len(samples)}{'B' if depth == 8 else 'H'}", *samples))
    return planes


def generate(valid, stage, decoder):
    stage.mkdir(parents=True, exist_ok=True)
    for depth, layers in ((8, (3, 7)), (10, (2, 9))):
        seed = (valid / f"syn-atlas-multistream-{depth}bit.obu").read_bytes()
        assert sha256(seed).hexdigest() == SEED_SHA256[depth]
        for name, layouts in LAYOUTS.items():
            alpha = name == "alpha-paired"
            path = stage / f"syn-atlas-{name}-{depth}bit.obu"
            data = with_layouts(seed, layouts, alpha, layers)
            path.write_bytes(data)
            print(path.name, sha256(data).hexdigest(), flush=True)
            digest = sha256()
            per_plane = [sha256() for _ in range(3)]
            for index, pictures in enumerate(native_frames(decoder, path, depth, layers)):
                layout = layouts[int(index >= (2 if depth == 8 else 5))]
                planes = composite(pictures, layout, alpha, depth)
                for samples, plane_digest in zip(planes, per_plane):
                    plane_digest.update(samples)
                    digest.update(samples)
            print("  composed I420 SHA-256:", digest.hexdigest())
            print("  composed Y, U, V SHA-256:", [d.hexdigest() for d in per_plane])


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--avmdec", required=True, type=Path)
    parser.add_argument("--stage", required=True, type=Path)
    parser.add_argument("--valid", type=Path, default=Path(__file__).resolve().parents[2] / "tests/conformance/vectors/valid")
    args = parser.parse_args()
    generate(args.valid, args.stage, args.avmdec)
