# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

"""Matched native playback timing for the isolated wide luma experiment."""

import csv
import hashlib
import os
from pathlib import Path
import random
import statistics
import subprocess
import sys
import time

ROUNDS = 24
GRAIN = "Noise_Ocean_1920x1080_60fps_filmgrain_qp135_130f.ivf"
TEN_BIT = "RitualDance_1920x1080_60fps_10bit_420_qp135_600f.ivf"


def decode(binary, source, threads, *options):
    delay = ("--frame-delay", "1") if threads == 1 else ()
    return (binary, "decode", source, "--threads", str(threads), *delay, *options)


def raw_digest(binary, source, threads, *options):
    digest = hashlib.sha256()
    size = 0
    with subprocess.Popen(
        decode(binary, source, threads, "--output-format", "raw", "-o", "-", *options),
        stdout=subprocess.PIPE,
    ) as proc:
        for chunk in iter(lambda: proc.stdout.read(1 << 20), b""):
            digest.update(chunk)
            size += len(chunk)
        if proc.wait():
            raise subprocess.CalledProcessError(proc.returncode, proc.args)
    return digest.hexdigest(), size


def interval(ratios):
    rng = random.Random(1891)
    resample = sorted(statistics.median(rng.choices(ratios, k=len(ratios))) for _ in range(20_000))
    return [(value - 1) * 100 for value in (statistics.median(ratios), resample[500], resample[19_499])]


def main(out, data):
    cases = {}
    for line in (data / "simd-cases.tsv").read_text().splitlines():
        name, checksum, frames, size = line.split("\t")
        cases[name] = checksum, int(frames), int(size)
    binaries = {name: out / name for name in ("control", "candidate")}
    for case in (GRAIN, TEN_BIT):
        checksum, _, size = cases[case]
        for variant, binary in binaries.items():
            actual = raw_digest(binary, data / case, 1)
            assert actual == (checksum, size), (case, variant, actual)
            print(f"AVM {case} {variant} {size} {checksum}", flush=True)
    grain_off = [raw_digest(binary, data / GRAIN, 1, "--skip-film-grain") for binary in binaries.values()]
    assert grain_off[0] == grain_off[1] and grain_off[0][0] != cases[GRAIN][0]
    print("Grain-off control", *grain_off[0], flush=True)

    allowed = os.sched_getaffinity(0)
    with (out / "paired.csv").open("w", newline="") as stream:
        writer = csv.writer(stream)
        writer.writerow(("stream", "threads", "round", "order", "control_seconds", "candidate_seconds"))
        for threads in (1, 7):
            if threads == 1:
                os.sched_setaffinity(0, {min(allowed)})
            else:
                os.sched_setaffinity(0, allowed)
            commands = {
                name: decode(binary, data / GRAIN, threads, "--output-format", "null")
                for name, binary in binaries.items()
            }
            for command in commands.values():
                subprocess.run(command, check=True, stdout=subprocess.DEVNULL)
            ratios = []
            for round_ in range(ROUNDS):
                order = ("control", "candidate") if round_ % 2 == 0 else ("candidate", "control")
                elapsed = {}
                for name in order:
                    start = time.perf_counter_ns()
                    subprocess.run(commands[name], check=True, stdout=subprocess.DEVNULL)
                    elapsed[name] = (time.perf_counter_ns() - start) * 1e-9
                writer.writerow((GRAIN, threads, round_ + 1, "/".join(order),
                                 elapsed["control"], elapsed["candidate"]))
                stream.flush()
                ratios.append(elapsed["candidate"] / elapsed["control"])
            point, low, high = interval(ratios)
            print(f"{threads} workers, {ROUNDS} pairs: {point:+.2f}% [{low:+.2f}%, {high:+.2f}%]", flush=True)


if __name__ == "__main__":
    main(*(Path(arg).resolve() for arg in sys.argv[1:]))
