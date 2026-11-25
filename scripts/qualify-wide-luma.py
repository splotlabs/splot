# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

"""Qualify the native wide luma release against an exact control."""

import csv
import gzip
import hashlib
import json
import math
import os
from pathlib import Path
import random
import statistics
import subprocess
import sys
import time
import tomllib

ROUNDS = 24
GRAIN = "Noise_Ocean_1920x1080_60fps_filmgrain_qp135_130f.ivf"
TEN_BIT = "RitualDance_1920x1080_60fps_10bit_420_qp135_600f.ivf"
# AVM a1ec4594d97028ebc34083ec8346616d697c2fc9 with --i420 --threads=1 --skip-film-grain.
GRAIN_OFF_SHA256 = "d9c7bcd31ad76c5a20b3dd8601f26d9d289ba56c3b2838f67b4a3a591aa5ded6"


def decode(binary, source, threads, *options):
    delay = ("--frame-delay", "1") if threads == 1 else ()
    return (binary, "decode", source, "--threads", str(threads), *delay, *options)


def raw_digest(command, env=None):
    digest = hashlib.sha256()
    size = 0
    with subprocess.Popen(
        command, stdout=subprocess.PIPE, env=env,
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


def check_hashes(out, data, cases, binaries):
    grain_off = (GRAIN_OFF_SHA256, cases[GRAIN][2])
    for case, (checksum, frames, size) in cases.items():
        reference = None
        for variant, binary in binaries.items():
            for threads in (1, 12):
                result = json.loads(subprocess.check_output(
                    decode(binary, data / case, threads, "--output-format", "hash", "--json")
                ))
                del result["selected_thread_policy"]
                with gzip.open(out / variant / f"{case}.{threads}.json.gz", "wt") as file:
                    json.dump(result, file)
                if len(result["frames"]) != frames:
                    raise SystemExit(f"{variant} {case}: The decoded frame count differs.")
                if reference is None:
                    reference = result
                if result != reference:
                    raise SystemExit(f"{variant} {case} {threads} workers: The CLI hashes differ.")
                if case in (GRAIN, TEN_BIT):
                    actual = raw_digest(decode(
                        binary, data / case, threads, "--output-format", "raw", "-o", "-"
                    ))
                    check_raw(actual, (checksum, size), f"{variant} CLI {case} {threads} workers")
            if case == GRAIN:
                actual = raw_digest(decode(
                    binary, data / case, 1, "--output-format", "raw", "-o", "-", "--skip-film-grain"
                ))
                check_raw(actual, grain_off, f"{variant} CLI grain-off")
            if case in (GRAIN, TEN_BIT):
                for link in ("shared", "static"):
                    command = (out / variant / link, data / case, "/dev/stdout")
                    check_raw(raw_digest(command), (checksum, size), f"{variant} {link} {case}")
                    if case == GRAIN:
                        skip_grain = dict(os.environ, SPLOT_C_SKIP_GRAIN="1")
                        check_raw(raw_digest(command, skip_grain), grain_off, f"{variant} {link} grain-off")
        print(f"CLI frame hashes match for {case} at 1 and 12 workers ({frames} frames).", flush=True)


def check_raw(actual, expected, label):
    print(f"{label} {actual[0]} {actual[1]}", flush=True)
    if actual != expected:
        raise SystemExit(f"{label}: The raw output SHA-256 or byte count differs from {expected}.")


def check_sizes(out, binaries):
    with (out / "binary-size.toml").open("rb") as file:
        budgets = tomllib.load(file)
    limits = {
        "splot": budgets["budget"]["x86_64-linux"],
        "libsplot_capi.so": budgets["capi_budget"]["x86_64-linux"],
        "libsplot_capi.a": None,
    }
    failures = []
    sizes = {}
    for variant in binaries:
        sizes[variant] = {}
        for line in (out / variant / "sizes.txt").read_text().splitlines():
            size, name = line.split(maxsplit=1)
            sizes[variant][Path(name).name] = int(size)
    for name, budget in limits.items():
        control = sizes["control"][name]
        candidate = sizes["candidate"][name]
        print(f"Size {name}: {control:,} -> {candidate:,} bytes ({candidate - control:+,}).", flush=True)
        if budget is not None and candidate > budget:
            failures.append(f"{name} exceeds its {budget:,}-byte release budget.")
    return failures


def verify_playback(path):
    series = {}
    with path.open(newline="") as file:
        for row in csv.DictReader(file):
            case = row["stream"]
            threads, attempt, pair = (int(row[key]) for key in ("threads", "run", "pair"))
            key = (case, threads, attempt)
            values = series.setdefault(key, [])
            control, candidate = (float(row[f"{variant}_seconds"]) for variant in ("control", "candidate"))
            order = "control/candidate" if (len(values) + attempt) % 2 else "candidate/control"
            if pair != len(values) + 1 or row["order"] != order:
                raise SystemExit(f"{key}: Pair numbers or run order differ.")
            if not all(math.isfinite(value) and value > 0 for value in (control, candidate)):
                raise SystemExit(f"{key}: The recorded time is invalid.")
            values.append((control, candidate))

    def check(case, threads, attempt):
        key = (case, threads, attempt)
        values = series.pop(key, [])
        if len(values) != ROUNDS:
            raise SystemExit(f"{key}: The required {ROUNDS} pairs are missing.")
        bases, tests = zip(*values)
        point, low, high = interval([test / base for base, test in values])
        name = "RitualDance first 130 frames" if case == TEN_BIT else "Noise Ocean full stream"
        print(f"{name}, {threads} workers, run {attempt}, {ROUNDS} pairs: "
              f"{statistics.median(bases):.5f} s -> {statistics.median(tests):.5f} s, "
              f"{point:+.2f}% [{low:+.2f}%, {high:+.2f}%].", flush=True)
        return point, low, high

    for threads in (1, 7):
        _, _, high = check(GRAIN, threads, 1)
        if high >= 0:
            raise SystemExit(f"Noise Ocean at {threads} workers has no established playback gain.")
        point, low, _ = check(TEN_BIT, threads, 1)
        if point > 0.5:
            repeat, repeat_low, _ = check(TEN_BIT, threads, 2)
            if repeat > 0.5 or low > 0.5 or repeat_low > 0.5:
                raise SystemExit(f"RitualDance at {threads} workers exceeds the 0.5% regression limit.")
    if series:
        raise SystemExit("The playback evidence contains unexpected samples.")


def main(out, data):
    cases = {}
    for line in (data / "simd-cases.tsv").read_text().splitlines():
        name, checksum, frames, size = line.split("\t")
        cases[name] = checksum, int(frames), int(size)
    if len(cases) != 7 or not {GRAIN, TEN_BIT} <= cases.keys():
        raise SystemExit("The seven pinned cases are required.")
    binaries = {name: out / name / "splot" for name in ("control", "candidate")}
    failures = check_sizes(out, binaries)
    check_hashes(out, data, cases, binaries)
    for failure in failures:
        print(failure, flush=True)
    if failures:
        raise SystemExit("The qualification checks failed.")


if __name__ == "__main__":
    if sys.argv[1] == "--verify-playback":
        verify_playback(Path(sys.argv[2]))
    else:
        main(Path(sys.argv[1]).resolve(), Path(sys.argv[2]).resolve())
