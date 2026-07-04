#!/usr/bin/env python3
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
"""Locate the local AVM `avmenc`/`avmdec` binaries.

LOCAL ONLY. AVM is never vendored, built, or invoked by CI; this script only
discovers a developer's existing local AVM checkout/build so the other
`tools/decoder-fixtures/` scripts can shell out to it.

Resolution order for each binary:

1. `$AVM_BUILD/<name>` if `$AVM_BUILD` is set (a specific build output dir).
2. `$AVM_ROOT/build/<name>`, `$AVM_ROOT/build-splot-fixtures/<name>`,
   `$AVM_ROOT/build_inspect/<name>` if `$AVM_ROOT` is set.
3. The same three build dirs under the hard-coded default AVM checkout
   (`/Users/bartosztomczyk/Devel/avm`).
4. `<name>` on `$PATH`.

Importable as `find_avmenc()` / `find_avmdec()` (each returns a `Path` or
raises `AvmNotFoundError`); also runnable as a CLI that prints the resolved
paths and a sanitized version summary for each.
"""

from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
from pathlib import Path

DEFAULT_AVM_ROOT = Path("/Users/bartosztomczyk/Devel/avm")
BUILD_SUBDIRS = ("build", "build-splot-fixtures", "build_inspect")


class AvmNotFoundError(RuntimeError):
    """Raised when an AVM binary cannot be resolved by any known strategy."""


def _candidate_dirs(env: dict) -> list[Path]:
    dirs: list[Path] = []
    avm_build = env.get("AVM_BUILD")
    if avm_build:
        dirs.append(Path(avm_build))
    for root_str in (env.get("AVM_ROOT"), str(DEFAULT_AVM_ROOT)):
        if not root_str:
            continue
        root = Path(root_str)
        dirs.extend(root / sub for sub in BUILD_SUBDIRS)
    return dirs


def _find_binary(name: str, env: dict) -> Path:
    for directory in _candidate_dirs(env):
        candidate = directory / name
        if candidate.is_file() and __import__("os").access(candidate, __import__("os").X_OK):
            return candidate
    on_path = shutil.which(name)
    if on_path:
        return Path(on_path)
    raise AvmNotFoundError(
        f"could not find '{name}'. Set $AVM_ROOT to an AVM checkout with a "
        f"built {BUILD_SUBDIRS[0]}/ dir, $AVM_BUILD to the exact build output "
        f"dir, or put '{name}' on $PATH."
    )


def find_avmenc(env: dict | None = None) -> Path:
    """Return the resolved path to the `avmenc` encoder binary."""
    import os

    return _find_binary("avmenc", env if env is not None else os.environ)


def find_avmdec(env: dict | None = None) -> Path:
    """Return the resolved path to the `avmdec` decoder binary."""
    import os

    return _find_binary("avmdec", env if env is not None else os.environ)


def _sanitized_version(binary: Path) -> str:
    """Run `<binary> --help` and return a version line with no local paths."""
    try:
        result = subprocess.run(
            [str(binary), "--help"], capture_output=True, timeout=10, text=True
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        return f"<version probe failed: {exc}>"
    for line in (result.stdout + result.stderr).splitlines():
        line = line.strip()
        # Lines look like "av2    - AOMedia Project AV2 Decoder 1.0.0-33-g457cd5868";
        # this never contains a filesystem path, only the codec name + version.
        if ("AV2 Decoder" in line or "AV2 Encoder" in line) and "/" not in line:
            return line
    return "<version unknown>"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.parse_args(argv)

    try:
        avmenc = find_avmenc()
        avmdec = find_avmdec()
    except AvmNotFoundError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1

    print(f"avmenc: {avmenc}")
    print(f"  {_sanitized_version(avmenc)}")
    print(f"avmdec: {avmdec}")
    print(f"  {_sanitized_version(avmdec)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
