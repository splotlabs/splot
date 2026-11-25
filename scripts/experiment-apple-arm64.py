# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

"""Measure a same-host macOS comparison and verify the frozen AVM output."""

import csv
import hashlib
import os
from pathlib import Path
import random
import statistics
import subprocess
import sys
import time


def command(binary, source, threads, output_format, limit=None):
    args = [
        binary, "decode", "--threads", str(threads), "--frame-delay", "1",
        "--output-format", output_format, source,
    ]
    return [*args, "-o", "-"] if limit is None else [*args, "--limit", str(limit)]


def timing_pairs(out, corpus, binaries):
    cases = [
        ("first-picture", "BlueSky_360p25_v2_qp110_130f.ivf", 1),
        ("first-picture", "RitualDance_1920x1080_60fps_10bit_420_qp135_600f.ivf", 1),
        ("playback-prefix", "Noise_Ocean_1920x1080_60fps_filmgrain_qp135_130f.ivf", 64),
        ("playback-prefix", "RitualDance_1920x1080_60fps_10bit_420_qp135_600f.ivf", 64),
    ]

    def measure(variant, source, limit):
        before = os.getloadavg()[0]
        start = time.perf_counter_ns()
        subprocess.run(
            command(binaries[variant], source, 1, "null", limit),
            check=True, stdout=subprocess.DEVNULL,
        )
        elapsed = time.perf_counter_ns() - start
        return elapsed, before, os.getloadavg()[0]

    with (out / "timings.csv").open("w", newline="") as file:
        writer = csv.writer(file)
        writer.writerow(
            ("mode", "stream", "limit", "round", "order", "variant", "revision",
             "wall_ns", "load_before", "load_after")
        )
        with (out / "timing-summary.txt").open("w") as summary:
            def record(line):
                print(line, flush=True)
                print(line, file=summary, flush=True)

            record("Hosted-runner timing is descriptive. No pass/fail threshold is applied.")
            for name, stream, limit in cases:
                source = corpus / stream
                revisions = {
                    mode: (binary.parent / "revision.txt").read_text().strip()
                    for mode, binary in binaries.items()
                }
                for variant in binaries:
                    measure(variant, source, limit)
                differences = []
                observations = {"base": [], "candidate": []}
                for pair in range(8):
                    order = tuple(binaries) if pair % 2 == 0 else tuple(reversed(binaries))
                    times = {}
                    for position, variant in enumerate(order, 1):
                        elapsed, before, after = measure(variant, source, limit)
                        writer.writerow(
                            (name, stream, limit, pair + 1, position, variant,
                             revisions[variant], elapsed, f"{before:.2f}", f"{after:.2f}")
                        )
                        file.flush()
                        observations[variant].append(elapsed)
                        times[variant] = elapsed
                    differences.append(100 * (times["candidate"] / times["base"] - 1))
                rng = random.Random(2006)
                bootstrap = sorted(
                    statistics.median(rng.choices(differences, k=len(differences)))
                    for _ in range(10_000)
                )
                base, candidate = (
                    statistics.median(observations[variant]) / 1e6 for variant in binaries
                )
                record(
                    f"{name} {stream} limit={limit} base={base:.2f}ms "
                    f"candidate={candidate:.2f}ms paired={statistics.median(differences):+.2f}% "
                    f"95% [{bootstrap[250]:+.2f}, {bootstrap[-251]:+.2f}]"
                )


def verify_all(out, corpus, binaries):
    lines = (corpus / "simd-cases.tsv").read_text().splitlines()
    assert len(lines) == 7, lines
    failures = []
    with (out / "avm-hashes.tsv").open("w", newline="") as file:
        writer = csv.writer(file, delimiter="\t")
        writer.writerow(("status", "variant", "stream", "workers", "frames", "bytes", "sha256"))
        for line in lines:
            filename, expected, frames, expected_bytes = line.split("\t")
            for workers in (1, 7, 12):
                for variant, binary in binaries.items():
                    digest = hashlib.sha256()
                    size = 0
                    args = command(binary, corpus / filename, workers, "raw")
                    with subprocess.Popen(args, stdout=subprocess.PIPE) as decode:
                        while chunk := decode.stdout.read(1 << 20):
                            digest.update(chunk)
                            size += len(chunk)
                        code = decode.wait()
                    actual = digest.hexdigest()
                    good = (code, size, actual) == (0, int(expected_bytes), expected)
                    result = ("PASS" if good else "FAIL", variant, filename, workers, frames, size, actual)
                    writer.writerow(result)
                    file.flush()
                    print(*result, flush=True)
                    if not good:
                        failures.append(result)
    if failures:
        raise RuntimeError(failures)


def main():
    out, corpus = (Path(argument).resolve() for argument in sys.argv[1:])
    binaries = {variant: out / variant / "splot" for variant in ("base", "candidate")}
    timing_pairs(out, corpus, binaries)
    verify_all(out, corpus, binaries)


if __name__ == "__main__":
    main()
