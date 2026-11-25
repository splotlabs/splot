# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

"""Verify frozen AVM output and measure alternating full-stream decodes."""

import csv
import hashlib
import os
from pathlib import Path
import re
import statistics
import subprocess
import sys
import time


def command(binary, source, threads):
    return [binary, "decode", "--threads", str(threads), "--frame-delay", "1", source]


def verify(binary, source, threads, expected, expected_size):
    digest = hashlib.sha256()
    size = 0
    args = [*command(binary, source, threads), "--output-format", "raw", "-o", "-"]
    with subprocess.Popen(args, stdout=subprocess.PIPE) as decode:
        while chunk := decode.stdout.read(1 << 20):
            digest.update(chunk)
            size += len(chunk)
        code = decode.wait()
    actual = digest.hexdigest()
    assert (code, size, actual) == (0, expected_size, expected), (
        binary, source, threads, code, size, actual
    )
    return actual


def measure(binary, source, threads):
    args = ["/usr/bin/time", "-l", *command(binary, source, threads), "--output-format", "null"]
    before = os.getloadavg()[0]
    start = time.perf_counter_ns()
    result = subprocess.run(args, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True, check=True)
    elapsed = time.perf_counter_ns() - start
    instructions = re.search(r"(\d+)\s+instructions retired", result.stderr)
    return elapsed, instructions[1] if instructions else "", before, os.getloadavg()[0]


def main():
    out, corpus = (Path(value).resolve() for value in sys.argv[1:])
    variants = ("base", "candidate")
    binaries = {variant: out / variant / "splot" for variant in variants}
    revisions = {
        variant: (out / variant / "revision.txt").read_text().strip()
        for variant in variants
    }
    cases = [line.split("\t") for line in (corpus / "simd-cases.tsv").read_text().splitlines()]
    assert len(cases) == 7
    for name, expected, frames, size in cases:
        for threads in (1, 7, 12):
            for variant, binary in binaries.items():
                actual = verify(binary, corpus / name, threads, expected, int(size))
                print("PASS", variant, name, threads, frames, size, actual, sep="\t")

    with (out / "timings.csv").open("w", newline="") as file:
        writer = csv.writer(file)
        writer.writerow(
            ("stream", "threads", "round", "order", "variant", "revision", "wall_ns",
             "instructions", "load_before", "load_after")
        )
        for name, _, _, _ in cases:
            source = corpus / name
            for threads in (1, 12):
                for binary in binaries.values():
                    measure(binary, source, threads)
                wall_deltas = []
                instruction_deltas = []
                for round_index in range(8):
                    order = variants if round_index % 2 == 0 else variants[::-1]
                    elapsed = {}
                    instructions = {}
                    for position, variant in enumerate(order, 1):
                        ns, retired, before, after = measure(binaries[variant], source, threads)
                        elapsed[variant] = ns
                        instructions[variant] = int(retired) if retired else 0
                        writer.writerow(
                            (name, threads, round_index + 1, position, variant, revisions[variant],
                             ns, retired, before, after)
                        )
                        file.flush()
                    wall_deltas.append(100 * (elapsed["candidate"] / elapsed["base"] - 1))
                    if all(instructions.values()):
                        instruction_deltas.append(100 * (instructions["candidate"] / instructions["base"] - 1))
                print(f"{name} threads={threads} wall median {statistics.median(wall_deltas):+.2f}% "
                      f"[{min(wall_deltas):+.2f}, {max(wall_deltas):+.2f}]")
                if instruction_deltas:
                    print(f"{name} threads={threads} instructions median "
                          f"{statistics.median(instruction_deltas):+.2f}%")
                elif threads == 1:
                    print(f"{name} instructions retired unavailable on this runner")


if __name__ == "__main__":
    main()
