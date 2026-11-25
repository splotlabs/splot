#!/usr/bin/env python3
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
"""Build only the isolated luma Wiener run unit with crate-wide AVX2 features."""

import os
import platform
import sys
from pathlib import Path


def value(args, option, default=None):
    return args[args.index(option) + 1] if option in args else default


rustc, *args = sys.argv[1:]
target = value(args, "--target", platform.machine())
unit = value(args, "--crate-name")
flags = []
if target.startswith("x86_64") and unit == "splot_lr_avx2" and "--test" not in args:
    # This unit's sole entry requires a Fearless AVX2 token.
    flags = [
        "-Ctarget-cpu=x86-64",
        "-Ctarget-feature=+avx2,+bmi1,+bmi2,+cmpxchg16b,+f16c,+fma,+fxsr,+lzcnt,+movbe,+popcnt,+xsave",
    ]

log = os.environ.get("SPLOT_LR_COMPILE_LOG")
if log and unit and any(arg.startswith("--emit") for arg in args):
    with Path(log).open("a", encoding="utf-8") as output:
        print(unit, target, *flags, sep="\t", file=output)

os.execvp(rustc, [rustc, *args, *flags])
