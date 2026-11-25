#!/usr/bin/env python3
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
"""Compile only the experiment's concrete CDEF units for their proven x86 capability."""
import os
from pathlib import Path
import platform
import sys

rustc, *args = sys.argv[1:]
def value(option, default=None):
    return args[args.index(option)+1] if option in args else default

# The target is explicit when cross-compiling; default Cargo uses the native target.
target = value('--target', platform.machine())
unit = value('--crate-name')
flags = []
if target.startswith('x86_64') and unit in ('splot_cdef_base', 'splot_cdef_avx2'):
    flags = ['-Ctarget-cpu=x86-64']
    if unit == 'splot_cdef_avx2' and '--test' not in args:
        # The complete feature contract of the type carried by the selected backend.
        flags += ['-Ctarget-feature=+avx2,+bmi1,+bmi2,+cmpxchg16b,+f16c,+fma,+fxsr,+lzcnt,+movbe,+popcnt,+xsave']
log = os.environ.get('SPLOT_CDEF_COMPILE_LOG')
if log and unit and '--emit' in ' '.join(args):
    with Path(log).open('a') as output:
        print(unit,target,*flags,sep='\t',file=output)
os.execvp(rustc,[rustc,*args,*flags])
