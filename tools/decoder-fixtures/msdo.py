#!/usr/bin/env python3
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
"""Reproduce the MSDO proof streams with independently encoded I420 components."""

import argparse
import hashlib
import json
from pathlib import Path

from extended_layers import AVM_REVISION, framed, temporal_units
from generate import _write_i420_y4m, find, run_checked


# Each case gives the bit depth, frame count, and (xlayer, width, height, seed, key interval).
CASES = (
    ("two-streams-8bit", 8, 4, ((3, 64, 64, 0, 2), (7, 80, 64, 37, 2))),
    ("two-streams-10bit", 10, 5, ((2, 64, 64, 0, 0), (9, 80, 64, 37, 0))),
    ("four-streams-obu-order", 8, 3,
     ((1, 64, 64, 11, 0), (5, 80, 64, 55, 0), (12, 64, 80, 132, 0), (20, 96, 64, 220, 0))),
    ("ops-random-access", 8, 6, ((3, 64, 64, 0, 4), (7, 80, 64, 37, 2))),
)

OPS_CASE = "ops-random-access"
AV2_SEQUENCE, AV2_LCR, AV2_OPS, AV2_MSDO = 1, 16, 18, 20


def obu_type(obu):
    return (obu[0] >> 2) & 31


def obu_xlayer(obu):
    if obu[0] & 128:
        return obu[1] & 31
    return 31 if obu_type(obu) == AV2_MSDO else 0


def encode(encoder, scalable, path, width, height, seed, depth, count, key, use_ops):
    source = path.with_suffix(".y4m")

    def sample(t, x, y, plane):
        return (16 + (3 * x + 5 * y + 13 * t + seed + 47 * plane) % 192) << (depth - 8)

    _write_i420_y4m(source, width, height, depth, sample, count)
    print(f"{source.name} source={hashlib.sha256(source.read_bytes()).hexdigest()}")
    if use_ops:
        # The pinned scalable_encoder example accepts raw I420.
        frame_size = width * height * 3 // 2
        y4m = source.read_bytes().split(b"\n", 1)[1]
        marker_size = len(b"FRAME\n")
        stride = frame_size + marker_size
        raw = b"".join(y4m[start + marker_size:start + stride]
                       for start in range(0, len(y4m), stride))
        if len(raw) != count * frame_size:
            raise ValueError(f"Unexpected raw source size for {source.name}.")
        source = source.with_suffix(".i420")
        source.write_bytes(raw)
        run_checked([scalable, str(width), str(height), source, path, str(count),
                     "2", "1", "0", "0", "0", str(key)])
        return

    flags = ["--lag-in-frames=0"]
    if key:
        flags += [f"--kf-min-dist={key}", f"--kf-max-dist={key}"]
    if depth == 10:
        flags = ["--lag-in-frames=5", "--auto-alt-ref=1", "--monotonic-output-order=0",
                 "--enable-keyframe-filtering=0"]
    run_checked([
        encoder, source, "--obu", "--i420", f"--profile={2 if depth == 10 else 0}",
        f"--bit-depth={depth}", f"--input-bit-depth={depth}", "--cpu-used=6",
        "--threads=1", "--qp=100", "--end-usage=q", "--passes=1", "--quiet", "-D",
        "--disable-warning-prompt", f"--limit={count}", "-o", path, *flags,
    ])


def retag(obu, xlayer, depth, use_ops):
    header_size = 2 if obu[0] & 128 else 1
    mlayer = obu[1] & 224 if header_size == 2 else 0
    payload = bytearray(obu[header_size:])

    if obu_type(obu) == AV2_SEQUENCE and (depth == 10 or use_ops):
        # AV2 § 5.4.1. These profile-2 headers have ID 0 and no tier bit.
        # The 10-bit component uses global LCR 1. OPS uses no LCR and sets
        # monotonic output order. The lag-free frames use immediate output.
        expected_prefix = b"\x88\x0c" if depth == 10 else b"\x88\x0a"
        if payload[:2] != expected_prefix:
            raise ValueError("Unexpected sequence header in the AVM component.")
        if use_ops:
            if payload[2] & 0xE0 != 0x20 or payload[3] & 0x40:
                raise ValueError("Unexpected sequence LCR ID or output order.")
            payload[2] &= 0x1F
            payload[3] |= 0x40
        else:
            if payload[2] & 0x80:
                raise ValueError("The 10-bit AVM component already references an LCR.")
            payload[2] |= 0x80
    return framed(bytes((obu[0] | 128, mlayer | xlayer)) + payload)


def mux(template, components, depth, use_ops):
    # Reuse only the existing descriptors and their resend locations. All
    # frame OBUs and sequence headers come from the independent encodes.
    template_units = temporal_units(template.read_bytes())
    streams = {xlayer: temporal_units(path.read_bytes()) for xlayer, path in components.items()}
    if any(len(units) != len(template_units) for units in streams.values()):
        raise ValueError(f"Component temporal unit counts differ for {template.name}.")
    if any(not units[0] for units in streams.values()):
        raise ValueError(f"An AVM component has an empty first temporal unit for {template.name}.")
    output = bytearray()
    for index, layout in enumerate(template_units):
        output += framed(b"\x08")
        for obu in layout:
            if obu_type(obu) in (AV2_MSDO, AV2_LCR, AV2_OPS) and obu_xlayer(obu) == 31:
                output += framed(obu)
        for xlayer, units in streams.items():
            for obu in layout:
                if obu_type(obu) == AV2_OPS and obu_xlayer(obu) == xlayer:
                    output += framed(obu)
            if (any(obu_type(obu) == AV2_SEQUENCE and obu_xlayer(obu) == xlayer for obu in layout)
                    and not any(obu_type(obu) == AV2_SEQUENCE for obu in units[index])):
                initial = next(obu for obu in units[0] if obu_type(obu) == AV2_SEQUENCE)
                output += retag(initial, xlayer, depth, use_ops)
            for obu in units[index]:
                if obu_type(obu) not in (AV2_LCR, AV2_OPS):
                    output += retag(obu, xlayer, depth, use_ops)
    return output


def verify(path, components, depth, count, specs, decoder, splot, use_ops):
    run_checked([splot, "validate", "--strict", path])
    inputs = {}
    decode = [decoder, "--rawvideo", "--i420", f"--output-bit-depth={depth}", "-o", "-"]
    for xlayer, width, height, _, _ in specs:
        raw = run_checked([*decode, "--all-layers", components[xlayer]]).stdout
        size = width * height * 3 // 2 * (2 if depth == 10 else 1)
        if len(raw) != count * size:
            raise ValueError(f"Unexpected decoded size for layer {xlayer}.")
        inputs[xlayer] = [raw[start:start + size] for start in range(0, len(raw), size)]

    selections = [("--all-layers", (), inputs.keys(), ())]
    selections += [(None, ("--extended-layer", str(layer)), (layer,), ()) for layer in inputs]
    if use_ops:
        selections += [
            ("--select-ops=0,0", ("--operating-point", "global:0:0"), (3, 7), (3, 7)),
            ("--select-ops=0,1", ("--operating-point", "global:0:1"), (3,), ()),
            ("--select-ops=1,0", ("--operating-point", "global:1:0"), (3, 7), (7,)),
            ("--select-local-ops=3,2,0", ("--operating-point", "local:3:2:0"), (3, 7), (3,)),
        ]
    for mode, options, selected, base_only in selections:
        timeline = [(layer, i) for i in range(count) for layer in selected
                    if layer not in base_only or i % 2 == 0]
        if depth == 10 and mode == "--all-layers":
            # The coded DOH sequence is 0, 3, 1, 2, 4. The decoder
            # releases display frames 2 and 3 from one layer at a time.
            timeline[4:8] = [(2, 2), (2, 3), (9, 2), (9, 3)]
        frames = [inputs[layer][i] for layer, i in timeline]
        expected = b"".join(frames)
        label = mode or " ".join(options)
        if mode and run_checked([*decode, mode, path]).stdout != expected:
            raise ValueError(f"AVM differs from independent outputs for {path.name} {label}.")
        if len(selected) == 1:
            actual = run_checked([splot, "decode", "--output-format", "raw",
                                  *options, "-o", "-", path]).stdout
            if actual != expected:
                raise ValueError(f"Splot raw bytes differ for {path.name} {label}.")
        expected_digests = [hashlib.sha256(frame).hexdigest() for frame in frames]
        for threads in (1, 3, 12):
            report = run_checked([
                splot, "decode", "--output-format", "hash", "--json",
                "--threads", str(threads), *options, path,
            ])
            digests = [frame["hashes"][0]["digest_hex"] for frame in json.loads(report.stdout)["frames"]]
            if digests != expected_digests:
                raise ValueError(f"Splot output differs for {path.name} {label} at {threads} threads.")
        print(f"{path.name} {label} frames={len(frames)} bytes={len(expected)} "
              f"raw={hashlib.sha256(expected).hexdigest()}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--stage", type=Path, required=True)
    parser.add_argument("--splot", type=Path, required=True)
    args = parser.parse_args()
    encoder, decoder = find("avmenc"), find("avmdec")
    scalable = Path(encoder).parent / "examples/scalable_encoder"
    if not scalable.is_file():
        raise SystemExit("Build the AVM scalable_encoder example alongside avmenc.")
    for program in (encoder, decoder):
        if f"-g{AVM_REVISION[:10]}".encode() not in run_checked([program, "--help"]).stdout:
            raise SystemExit(f"Build {Path(program).name} at AVM revision {AVM_REVISION}.")
    if args.stage.exists() and any(args.stage.iterdir()):
        raise SystemExit("Select an empty staging directory.")
    args.stage.mkdir(parents=True, exist_ok=True)
    valid = Path(__file__).resolve().parents[2] / "tests/conformance/vectors/valid"
    for name, depth, count, specs in CASES:
        use_ops = name == OPS_CASE
        components = {}
        for xlayer, width, height, seed, key in specs:
            path = args.stage / f"{name}-x{xlayer}.obu"
            encode(encoder, scalable, path, width, height, seed, depth, count, key, use_ops)
            components[xlayer] = path
        output = args.stage / f"syn-msdo-reproducible-{name}.obu"
        output.write_bytes(mux(valid / f"syn-msdo-{name}.obu", components, depth, use_ops))
        if output.read_bytes() != (valid / output.name).read_bytes():
            raise ValueError(f"Generated bytes differ from the committed fixture {output.name}.")
        print(f"{output.name} stream={hashlib.sha256(output.read_bytes()).hexdigest()}")
        verify(output, components, depth, count, specs, decoder, args.splot, use_ops)


if __name__ == "__main__":
    main()
