# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

"""Check complete external streams against recorded oracle hashes (CONF-PUBLIC-VECTORS)."""

import argparse
import concurrent.futures
import hashlib
import json
import pathlib
import re
import subprocess
import tempfile
import tomllib
import urllib.request

TWELVE_BIT_NAME_RE = re.compile(r"(?:^|[._-])12bit(?:[._-]|$)")


def verify_input(source, expected):
    with source.open("rb") as file:
        actual = hashlib.file_digest(file, "sha256").hexdigest()
    if actual != expected:
        raise SystemExit(f"{source.name}: input SHA-256 differs: {actual}.")


def rejection_diagnostic(splot, source, width, output):
    result = subprocess.run(
        [str(splot), "decode", "--json", f"--threads={width}",
         "--output-format=raw", f"--output={output}", source],
        capture_output=True,
        text=True,
        timeout=600,
    )
    if result.returncode == 0:
        raise SystemExit(
            f"{source.name}, {width} threads: expected decode to reject the stream."
        )
    try:
        diagnostic = json.loads(result.stdout)
    except json.JSONDecodeError as exc:
        raise SystemExit(
            f"{source.name}, {width} threads: failed to parse rejection JSON (exit {result.returncode}): "
            f"{exc}; stdout={result.stdout!r}, stderr={result.stderr!r}"
        ) from exc
    if output.exists() or not isinstance(diagnostic, dict):
        raise SystemExit(f"{source.name}, {width} threads: expected a diagnostic and no output file.")
    return diagnostic


def verify_non_normative_12bit_rejection(splot, source, width, output):
    diagnostic = rejection_diagnostic(splot, source, width, output)
    if (
        diagnostic.get("rule_id") != "decode/malformed-source"
        or diagnostic.get("source_issue_kind") != "sequence_header_parse_error"
        or "bit_depth_idc must be less than or equal to 1" not in diagnostic.get("parser_message", "")
    ):
        raise SystemExit(
            f"{source.name}, {width} threads: expected bit_depth_idc rejection, got {diagnostic!r}."
        )
    print(f"{source.name}, {width} threads: rejected (non-normative 12-bit)", flush=True)


def verify_timing_info2(splot, source, threads, output):
    data = source.read_bytes()
    if hashlib.sha256(data).hexdigest() != "712aff94509ad7c79a8afa7424b6da523418d5ebd4edd0b11b7bbc5696f4d9f1":
        return source
    for width in threads:
        output.unlink(missing_ok=True)
        diagnostic = rejection_diagnostic(splot, source, width, output)
        if (diagnostic.get("rule_id"), diagnostic.get("unsupported_reason"), diagnostic.get("byte_offset")) != (
            "decode/unsupported-feature", "unexpected_leading_obu_after_key", 883
        ):
            raise SystemExit(f"{source.name}: unexpected metadata order diagnostic {diagnostic!r}.")

    # Set metadata_is_suffix on the five temporal point OBUs (AV2 § 7.3.3).
    corrected = bytearray(data)
    for offset in (884, 1094, 1152, 1165, 1179):
        corrected[offset] |= 0x80
    corrected_path = output.with_name("timing-info2-suffix.obu")
    corrected_path.write_bytes(corrected)
    return corrected_path


def check(corpus, root, splot, thread_override=None, fetch=None):
    if corpus == "dav2d":
        entries = [
            (name, f"{suite.parent.name}/{path}", digest)
            for suite in sorted(root.glob("avm*/meson.build"))
            for name, path, digest in re.findall(
                r"\['([^']+)',\s*files\('([^']+)'\),\s*'([0-9a-f]{32})'(?:,\s*\[[^\]]*\])?,?\s*\]",
                suite.read_text(),
            )
        ]
        streams = [
            dict(
                path=path,
                digest=digest,
                expect_12bit_rejection=bool(TWELVE_BIT_NAME_RE.search(pathlib.Path(path).name)),
            )
            for _, path, digest in entries
        ]
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
        if manifest["threads"] != [1, 7]:
            raise SystemExit("The sample manifest must specify thread counts 1 and 7.")
        if (root / ".git").exists():
            revision = subprocess.check_output(["git", "-C", root, "rev-parse", "HEAD"], text=True).strip()
            if revision != manifest["revision"]:
                raise SystemExit(f"The sample revision differs: expected {manifest['revision']}, got {revision}.")
        if (root / "samples").is_dir():
            root = root / "samples"
        if fetch:
            by_path = {s["path"]: s for s in manifest["stream"] if "release" in s}
            if unknown := sorted(set(fetch) - by_path.keys()):
                raise SystemExit(f"No release stream in the manifest for {unknown}.")
            manifest["stream"] = [by_path[path] for path in fetch]
        missing_releases = [
            (f"{manifest['source']}/releases/download/{s['release']}/{s['path']}", root / s["path"])
            for s in manifest["stream"]
            if "release" in s and not (root / s["path"]).exists()
        ]
        if missing_releases:
            with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
                list(pool.map(lambda item: urllib.request.urlretrieve(*item), missing_releases))
        if fetch:
            for stream in manifest["stream"]:
                verify_input(root / stream["path"], stream["input_sha256"])
            print(f"Verified {len(fetch)} fetched streams.")
            return
        streams = [dict(path=stream["path"], digest=stream["avm_raw_sha256"],
                        input_sha256=stream["input_sha256"], raw_bytes=stream["raw_bytes"])
                   for stream in manifest["stream"]]
        algorithm, threads, extension = "sha256", manifest["threads"], "*.ivf"
    if thread_override is not None:
        if thread_override not in threads:
            raise SystemExit(f"Unsupported thread override {thread_override}; expected one of {threads}.")
        threads = [thread_override]

    paths = [stream["path"] for stream in streams]
    if not paths or len(paths) != len(set(paths)):
        raise SystemExit("The corpus manifest is empty or contains duplicate paths.")
    present = {path.relative_to(root).as_posix() for path in root.rglob(extension)}
    missing, unlisted = sorted(set(paths) - present), sorted(present - set(paths))
    if missing or unlisted:
        raise SystemExit(f"The corpus files differ from the manifest: missing={missing}, unlisted={unlisted}.")

    verified = 0
    rejected = 0
    corrected = 0
    with tempfile.TemporaryDirectory() as temp:
        output = pathlib.Path(temp) / "decoded.raw"
        for stream in sorted(streams, key=lambda stream: stream["path"]):
            source = root / stream["path"]
            if "input_sha256" in stream:
                verify_input(source, stream["input_sha256"])
            if stream.get("expect_12bit_rejection"):
                for width in threads:
                    output.unlink(missing_ok=True)
                    verify_non_normative_12bit_rejection(splot, source, width, output)
                rejected += 1
                continue
            if corpus == "dav2d" and source.name == "avm.bus.64x64.l5.timing-info2.obu":
                checked_source = verify_timing_info2(splot, source, threads, output)
                if checked_source != source:
                    corrected += 1
                    source = checked_source
            for width in threads:
                output.unlink(missing_ok=True)
                subprocess.run(
                    [str(splot), "decode", f"--threads={width}",
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
            verified += 1
    if rejected or corrected:
        print(f"{corpus} hash matches: {verified - corrected} original, {corrected} corrected. "
              f"Expected rejections: {rejected} non-normative 12-bit, {corrected} misplaced metadata.")
    else:
        print(f"Verified all {verified} {corpus} streams.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("corpus", choices=["dav2d", "ewouth-av2"])
    parser.add_argument("root", type=pathlib.Path)
    parser.add_argument("--splot", type=pathlib.Path, default=pathlib.Path("target/release/splot"))
    parser.add_argument("--threads", type=int, default=None)
    parser.add_argument("--fetch", action="append", metavar="PATH",
                        help="download and verify only this release stream; no decode")
    args = parser.parse_args()
    check(args.corpus, args.root, args.splot.resolve(), args.threads, args.fetch)
