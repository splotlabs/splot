"""Verify the original snapshot and write a sanitized, reproducible copy."""

from collections import Counter
import csv
import hashlib
import io
import os
from pathlib import Path, PurePosixPath
import re
import tarfile

HERE = Path(__file__).resolve().parent
SOURCE = HERE.parent / "qualification-1882"
OUTPUT = HERE / "evidence"
TEXT_SUFFIXES = {
    ".asm", ".cg", ".cglog", ".csv", ".dis", ".functions", ".hash", ".log", ".md",
    ".overview", ".patch", ".pcs", ".sha", ".sha256", ".text", ".tree", ".tsv", ".txt",
}
SCRIPTS = {".py", ".sh"}
EXCLUDED_DIRECTORIES = {".git", "__pycache__", "data", "frozen", "qemu", "ref-target", "shipped", "target"}
EXCLUDED_PATHS = {("dav2d-native", "src"), ("dav2d-native", "build-v3"), ("lr-check",)}
SECRET_SHAPES = [
    re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----"),
    re.compile(r"\b(?:github_pat_[A-Za-z0-9_]{20,}|gh[pousr]_[0-9A-Za-z]{20,})\b"),
    re.compile(r"\b(?:AKIA|ASIA)[A-Z0-9]{16}\b"),
    re.compile(r"\b(?:sk-proj-|sk-ant-|xox[bpars]-)[A-Za-z0-9_-]{16,}\b"),
    re.compile(r"https?://[^/\s:@]+:[^/\s@]+@"),
    re.compile(r"(?i)\b(?:authorization\s*[:=]\s*(?:bearer|basic|token)\s+|bearer\s+)[^\s,]+"),
]
LOG_TIMESTAMP = re.compile(r"^\d{4}-\d\d-\d\dT\S+ ")
ENVIRONMENT_LOG_LINE = re.compile(r"^[ \t]+[A-Z][A-Z0-9_]{2,}[ \t]*:")
SECRET_LINE = re.compile(
    r"(?i)\b(?:GITHUB_TOKEN|GH_TOKEN|ACTIONS_RUNTIME_TOKEN|AWS_SECRET_ACCESS_KEY|"
    r"SSH_AUTH_SOCK|OPENAI_API_KEY|ANTHROPIC_API_KEY|PASSWORD|ACCESS_TOKEN)\s*[:=]"
)
HOST_FIELD = re.compile(r"(?im)^([ \t]*(?:Host|Hostname|Machine Name)[ \t]*:)[^\r\n]*")
PATHS = (
    (r"/home/[^/\s]+/Dev/Git/SPLOT/workspace-worktrees(?=/|\b)", "<WORKTREES>"),
    (r"/home/runner/work/splot/splot(?=/|\b)", "<CI_WORKSPACE>"),
    (r"/home/runner/work/_temp(?=/|\b)", "<CI_TEMP>"),
    (r"/home/[^/\s]+/\.cargo/registry(?=/|\b)", "<CARGO_REGISTRY>"),
    (r"/(?:home|Users|root)/[^\s\x22'<>]+", "<LOCAL_PATH>"),
    (r"/(?:tmp|mnt|run/user|var/folders|Volumes)/[^\s\x22'<>]+", "<LOCAL_PATH>"),
)
stats = Counter()
entries = {}

def digest(data):
    return hashlib.sha256(data).hexdigest()

def safe_path(name):
    path = PurePosixPath(name)
    return bool(path.parts) and not path.is_absolute() and ".." not in path.parts

def capture_environment(name):
    leaf = PurePosixPath(name).name.lower()
    return leaf.startswith(("environment.", "env.")) or leaf.endswith((".env", "-host.txt", "-api.json"))

def sanitize(name, data):
    if capture_environment(name):
        stats["omitted environment files"] += 1
        return None
    contents = data.decode("utf-8")
    if PurePosixPath(name).suffix in SCRIPTS and re.search(
        r"(?im)\b(?:gh\s+(?:api|issue\s+comment|pr\s+comment)|git\s+push)\b", contents
    ):
        stats["omitted publishing scripts"] += 1
        return None
    if name.endswith((".log", ".cglog")):
        cleaned = []
        in_environment = False
        for line in contents.splitlines(keepends=True):
            value = LOG_TIMESTAMP.sub("", line).rstrip("\r\n")
            if value.strip() == "env:":
                in_environment = True
                stats["omitted log environment lines"] += 1
                continue
            if in_environment and ENVIRONMENT_LOG_LINE.match(value):
                stats["omitted log environment lines"] += 1
                continue
            in_environment = False
            if SECRET_LINE.search(value) or any(pattern.search(value) for pattern in SECRET_SHAPES):
                stats["omitted log authorization lines"] += 1
                continue
            cleaned.append(line)
        contents = "".join(cleaned)
    contents, count = HOST_FIELD.subn(r"\1 <HOST_OMITTED>", contents)
    stats["scrubbed host fields"] += count
    for pattern, replacement in PATHS:
        contents, count = re.subn(pattern, replacement, contents)
        stats["scrubbed filesystem paths"] += count
    contents, count = re.subn(r"(?i)\bVAY-[0-9]+\b", "<HOST_OMITTED>", contents)
    stats["scrubbed host identifiers"] += count
    contents, count = re.subn(r"(?i)\b[\w.+-]+@vay\.io\b", "<EMAIL_OMITTED>", contents)
    stats["scrubbed organization addresses"] += count
    for pattern in SECRET_SHAPES:
        if pattern.search(contents):
            raise ValueError(f"Credential-like content requires manual review in {name}")
    return contents.encode()

def store(name, data):
    assert safe_path(name)
    assert name not in entries, name
    entries[name] = data

def original():
    with (HERE / "files.tsv").open(newline="") as f:
        manifest = {r["path"]: (r["sha256"], int(r["bytes"]))
                    for r in csv.DictReader(f, delimiter="\t")}
    seen = set()
    with tarfile.open(HERE / "measurements.tar.xz", "r:xz") as archive:
        for member in archive:
            name = member.name
            assert member.isfile() and safe_path(name) and name not in seen
            data = archive.extractfile(member).read()
            assert manifest[name] == (digest(data), len(data)), name
            seen.add(name)
            if (sanitized := sanitize(name, data)) is not None:
                store(name, sanitized)
    assert seen == set(manifest)
    stats["verified original members"] = len(seen)

def supplement(root, prefix=""):
    for directory, subdirectories, files in os.walk(root):
        parent = Path(directory)
        subdirectories[:] = sorted(name for name in subdirectories if (
            name not in EXCLUDED_DIRECTORIES
            and not name.endswith((".er", "-target"))
            and (parent / name).relative_to(root).parts not in EXCLUDED_PATHS
        ))
        for name in sorted(files):
            path = parent / name
            if path.is_symlink() or path.suffix not in TEXT_SUFFIXES | SCRIPTS:
                continue
            relative = prefix + path.relative_to(root).as_posix()
            data = path.read_bytes()
            try:
                if b"\0" in data:
                    stats["omitted opaque binary records"] += 1
                    continue
                sanitized = sanitize(relative, data)
            except UnicodeDecodeError:
                stats["omitted opaque binary records"] += 1
                continue
            if sanitized is None:
                continue
            if relative in entries:
                if entries[relative] != sanitized:
                    store(f"updated/{relative}", sanitized)
                    stats["updated snapshots preserved"] += 1
            else:
                store(relative, sanitized)
                stats["missing records and scripts added"] += 1

def publish():
    OUTPUT.mkdir(exist_ok=True)
    with tarfile.open(OUTPUT / "measurements.tar.xz", "w:xz") as archive:
        for name, data in sorted(entries.items()):
            info = tarfile.TarInfo(name)
            info.mode, info.size = 0o644, len(data)
            archive.addfile(info, io.BytesIO(data))
    with (OUTPUT / "files.tsv").open("w", newline="") as f:
        writer = csv.writer(f, delimiter="\t", lineterminator="\n")
        writer.writerow(("path", "sha256", "bytes"))
        for name, data in sorted(entries.items()):
            writer.writerow((name, digest(data), len(data)))
    with (OUTPUT / "packaging.txt").open("w") as f:
        for key, count in sorted(stats.items()):
            print(f"{key}: {count}", file=f)
        print(f"published members: {len(entries)}", file=f)
    with (OUTPUT / "SHA256SUMS").open("w") as f:
        for name in ("measurements.tar.xz", "files.tsv", "packaging.txt", "xeon-vnni-pairs.csv"):
            print(f"{digest((OUTPUT / name).read_bytes())}  {name}", file=f)

if __name__ == "__main__":
    original()
    supplement(SOURCE)
    supplement(HERE.parent / "splot-1934-gdf-leaf/target/gdf-leaf-audit", "gdf-explicit-leaf/")
    publish()
