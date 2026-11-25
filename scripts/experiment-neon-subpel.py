# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

"""Compare same-host NEON first-picture latency with an unchanged base."""

import csv
import hashlib
from itertools import permutations
import os
from pathlib import Path
import random
import statistics
import subprocess
import sys
import time


def command(binary, source, raw):
    args = [binary, "decode", "--threads", "1", "--frame-delay", "1", source]
    if raw:
        return [*args, "--output-format", "raw", "-o", "-"]
    return [*args, "--output-format", "null", "--limit", "1"]


def measure(binary, source):
    before = os.getloadavg()[0]
    start = time.perf_counter_ns()
    subprocess.run(command(binary, source, False), stdout=subprocess.DEVNULL, check=True)
    return time.perf_counter_ns() - start, before, os.getloadavg()[0]


def verify(binary, source, expected, expected_size):
    digest = hashlib.sha256()
    size = 0
    with subprocess.Popen(command(binary, source, True), stdout=subprocess.PIPE) as decode:
        while chunk := decode.stdout.read(1 << 20):
            digest.update(chunk)
            size += len(chunk)
        code = decode.wait()
    actual = digest.hexdigest()
    assert (code, size, actual) == (0, expected_size, expected), (binary, source, code, size, actual)
    return actual


def main():
    out, corpus = (Path(value).resolve() for value in sys.argv[1:])
    variants = ("base", "startup", "shared")
    orders = list(permutations(variants))
    binaries = {variant: out / variant / "splot" for variant in variants}
    revisions = {
        variant: (out / variant / "revision.txt").read_text().strip()
        for variant in variants
    }
    inputs = [
        line.split("\t") for line in (corpus / "simd-cases.tsv").read_text().splitlines()
        if line.startswith(("BlueSky_", "RitualDance_"))
    ]
    assert len(inputs) == 2
    print("Hosted timing uses the symbol-preserved release build and is descriptive.")
    with (out / "timings.csv").open("w", newline="") as file:
        writer = csv.writer(file)
        writer.writerow(
            ("stream", "round", "order", "variant", "revision", "wall_ns", "load_before", "load_after")
        )
        for name, _, _, _ in inputs:
            source = corpus / name
            for binary in binaries.values():
                measure(binary, source)
            pairs = []
            for index, order in enumerate(orders * 2, 1):
                times = {}
                for position, variant in enumerate(order, 1):
                    ns, before, after = measure(binaries[variant], source)
                    times[variant] = ns
                    writer.writerow((name, index, position, variant, revisions[variant], ns, before, after))
                    file.flush()
                pairs.append(times)
            for variant, reference in (("startup", "base"), ("shared", "startup"), ("shared", "base")):
                ratios = [100 * (pair[variant] / pair[reference] - 1) for pair in pairs]
                rng = random.Random(1993)
                distribution = sorted(
                    statistics.median(rng.choices(ratios, k=len(ratios))) for _ in range(10_000)
                )
                center = statistics.median(ratios)
                print(f"{name}: {variant}/{reference} {center:+.2f}% [{distribution[250]:+.2f}, {distribution[-251]:+.2f}]")
    for name, expected, frames, size in inputs:
        for variant, binary in binaries.items():
            actual = verify(binary, corpus / name, expected, int(size))
            print(f"PASS {variant} {name} {frames} {size} {actual}")


if __name__ == "__main__":
    main()
