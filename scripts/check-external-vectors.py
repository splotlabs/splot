# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

"""Check complete external streams against recorded oracle hashes (CONF-PUBLIC-VECTORS)."""

import argparse
import hashlib
import pathlib
import re
import subprocess
import tempfile
import tomllib


def check(corpus, root, splot):
    if corpus == "dav2d":
        root = root / "avm"
        entries = re.findall(
            r"\['([^']+)',\s*files\('([^']+)'\),\s*'([0-9a-f]{32})'\]",
            (root / "meson.build").read_text(),
        )
        streams = [dict(path=path, digest=digest) for _, path, digest in entries]
        names = [name for name, _, _ in entries]
        if len(names) != len(set(names)):
            raise SystemExit("The upstream AVM manifest contains duplicate test names.")
        algorithm, threads, extension = "md5", [1], "*.obu"
    else:
        manifest_path = pathlib.Path(__file__).resolve().parents[1] / "tests/conformance/ewouth-av2.toml"
        with manifest_path.open("rb") as file:
            manifest = tomllib.load(file)
        if manifest["schema_version"] != 1:
            raise SystemExit("The sample manifest schema is unsupported.")
        if manifest["threads"] != [1, 8]:
            raise SystemExit("The sample manifest must specify thread counts 1 and 8.")
        revision = subprocess.check_output(["git", "-C", root, "rev-parse", "HEAD"], text=True).strip()
        if revision != manifest["revision"]:
            raise SystemExit(f"The sample revision differs: expected {manifest['revision']}, got {revision}.")
        root = root / "samples"
        streams = [dict(path=stream["path"], digest=stream["avm_raw_sha256"],
                        input_sha256=stream["input_sha256"], raw_bytes=stream["raw_bytes"])
                   for stream in manifest["stream"]]
        algorithm, threads, extension = "sha256", manifest["threads"], "*.ivf"

    paths = [stream["path"] for stream in streams]
    if not paths or len(paths) != len(set(paths)):
        raise SystemExit("The corpus manifest is empty or contains duplicate paths.")
    present = {path.relative_to(root).as_posix() for path in root.rglob(extension)}
    missing, unlisted = sorted(set(paths) - present), sorted(present - set(paths))
    if missing or unlisted:
        raise SystemExit(f"The corpus files differ from the manifest: missing={missing}, unlisted={unlisted}.")

    with tempfile.TemporaryDirectory() as temp:
        output = pathlib.Path(temp) / "decoded.raw"
        for stream in sorted(streams, key=lambda stream: stream["path"]):
            source = root / stream["path"]
            if "input_sha256" in stream:
                with source.open("rb") as file:
                    actual = hashlib.file_digest(file, "sha256").hexdigest()
                if actual != stream["input_sha256"]:
                    raise SystemExit(f"{source.name}: input SHA-256 differs: {actual}.")
            for width in threads:
                output.unlink(missing_ok=True)
                subprocess.run(
                    [str(splot), "decode", "--quiet", f"--threads={width}",
                     "--output-format=raw", f"--output={output}", source],
                    check=True,
                    timeout=600,
                )
                with output.open("rb") as file:
                    actual = hashlib.file_digest(
                        file, lambda: hashlib.new(algorithm, usedforsecurity=False)
                    ).hexdigest()
                if actual != stream["digest"]:
                    raise SystemExit(f"{source.name}, {width} threads: expected {stream['digest']}, got {actual}.")
                if "raw_bytes" in stream and output.stat().st_size != stream["raw_bytes"]:
                    raise SystemExit(f"{source.name}: decoded output size differs.")
                print(f"{source.name}, {width} threads: {actual}", flush=True)
    print(f"Verified all {len(streams)} {corpus} streams.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("corpus", choices=["dav2d", "ewouth-av2"])
    parser.add_argument("root", type=pathlib.Path)
    parser.add_argument("--splot", type=pathlib.Path, default=pathlib.Path("target/release/splot"))
    args = parser.parse_args()
    check(args.corpus, args.root, args.splot.resolve())
