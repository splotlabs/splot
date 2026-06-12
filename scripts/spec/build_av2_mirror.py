#!/usr/bin/env python3
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
"""Build the committed AV2 specification mirror from a `pdftotext -layout` dump.

This script performs the *structuring* half of the mirror regeneration: it takes
the raw text produced by poppler's ``pdftotext -layout`` and splits it into one
Markdown file per chapter/annex, layering navigation headings + anchors over the
**verbatim** spec bytes (kept inside ```` ```text ```` fences). It also emits
``index.md``, ``provenance.toml``, ``CHECKSUMS``, and ``README.md``.

It is invoked by ``scripts/spec/regenerate-av2-spec.sh`` and uses only the Python
standard library (no third-party dependencies; markitdown is not used). The
fidelity guarantee is enforced by :func:`assert_roundtrip`, which re-reads the
generated files and checks that the concatenation of all fenced blocks is byte
(line) identical to the raw input.

This script and its shell wrapper are the only mechanical way to (re)generate the
mirror; the spec text itself is © Alliance for Open Media (see the mirror README).
"""

from __future__ import annotations

import argparse
import bisect
import hashlib
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

# A real section heading, as rendered by `pdftotext -layout`, is the section
# glyph followed by THREE spaces (inline cross-references use a single space and
# are excluded). A leading form-feed (page break) may precede the glyph. Headings
# come in four forms, matched in order:
#   - numbered:        "§   5.16. Padding OBU syntax"   (dotted number + ". " + title)
#   - annex top:       "§   Annex A: Profiles, ..."
#   - annex subsection:"§   A.1.General"  ("§   D.3.1.Background ...") — note the
#                      title runs straight after the trailing dot, no space.
#   - other/back matter:"§   References", "§   Index", ... (unnumbered)
# RE_OTHER is the catch-all and MUST be tried last; since every "§   " line in the
# spec is a heading, this guarantees no section is silently dropped from the index.
RE_NUM_HEADING = re.compile(r"^\x0c?§   (\d+(?:\.\d+)*)\.\s+(\S.*)$")
RE_ANNEX_HEADING = re.compile(r"^\x0c?§   Annex ([A-Z]):\s+(\S.*)$")
RE_ANNEX_SUB_HEADING = re.compile(r"^\x0c?§   ([A-Z](?:\.\d+)+)\.(\S.*)$")
RE_OTHER_HEADING = re.compile(r"^\x0c?§   (\S.*)$")
RE_FOOTER = re.compile(r"AV2 Specification\s+Page (\d+) of \d+")

FENCE_OPEN = "```text"
FENCE_CLOSE = "```"

# §9 is roughly half the document, so it is sub-split into per-subsection files.
SUBSPLIT_CHAPTERS = {9}
# Sub-split files for §9 live in this directory (relative to the mirror root).
CH9_DIRNAME = "09-additional-tables"


@dataclass
class Attachment:
    """A verbatim spec attachment committed under the mirror's ``attachments/``.

    Copied byte-for-byte (not structured) and pinned in ``provenance.toml`` +
    ``CHECKSUMS`` exactly like the PDF-derived files, so ``check-spec-mirror`` can
    detect drift.
    """

    src: Path  # local file fetched by the shell wrapper
    rel_path: str  # destination relative to the mirror root, e.g. attachments/all_tables.h
    url: str  # canonical AOM URL it was fetched from
    sha256: str  # expected sha256 (verified before copying)


@dataclass
class Heading:
    """A detected section heading and where it lives in the raw line list."""

    line_idx: int  # 0-based index into the raw line list
    kind: str  # "num" | "annex" (top or subsection) | "other" (back matter)
    number: str  # "5.16", "A", "A.1", "D.3.1", or "" for back matter
    title: str
    page: int

    @property
    def components(self) -> int:
        return self.number.count(".") + 1 if self.number else 1

    @property
    def anchor(self) -> str:
        if self.kind == "annex":
            return "s-annex-" + self.number.lower().replace(".", "-")
        if self.kind == "other":
            return "s-" + slugify(self.title)
        return "s-" + self.number.replace(".", "-")

    @property
    def md_level(self) -> int:
        # H1 is reserved for the file title; sections start at H2.
        return min(self.components + 1, 6)

    @property
    def heading_text(self) -> str:
        if self.kind == "annex":
            sep = " " if "." in self.number else ": "  # "Annex A: T" / "Annex A.1 T"
            return f"Annex {self.number}{sep}{self.title}"
        if self.kind == "other":
            return self.title
        return f"§ {self.number} {self.title}"

    @property
    def index_label(self) -> str:
        """Short citation label shown in the index 'Section' column."""
        if self.kind == "annex":
            return f"Annex {self.number}"
        if self.kind == "other":
            return "—"
        return f"§ {self.number}"


@dataclass
class Segment:
    """A top-level unit (front matter, chapter, annex) or a §9 subsection."""

    rel_path: str  # path of the output file, relative to the mirror root
    title: str  # human title for the file's H1
    start: int  # inclusive raw line index
    end: int  # exclusive raw line index
    headings: list[Heading] = field(default_factory=list)


def slugify(text: str) -> str:
    text = text.lower()
    text = re.sub(r"[^a-z0-9]+", "-", text)
    return text.strip("-")


def build_page_index(lines: list[str]) -> list[tuple[int, int]]:
    """Return sorted (line_idx, page) for each page footer."""
    footers: list[tuple[int, int]] = []
    for i, line in enumerate(lines):
        m = RE_FOOTER.search(line)
        if m:
            footers.append((i, int(m.group(1))))
    return footers


def page_for_line(footers: list[tuple[int, int]], idx: int) -> int:
    """Page number for a line: the first footer at or after the line."""
    keys = [f[0] for f in footers]
    pos = bisect.bisect_left(keys, idx)
    if pos < len(footers):
        return footers[pos][1]
    return footers[-1][1] if footers else 0


def detect_headings(lines: list[str], footers: list[tuple[int, int]]) -> list[Heading]:
    headings: list[Heading] = []
    for i, line in enumerate(lines):
        page = page_for_line(footers, i)
        m = RE_NUM_HEADING.match(line)
        if m:
            headings.append(Heading(i, "num", m.group(1), m.group(2).rstrip(), page))
            continue
        m = RE_ANNEX_HEADING.match(line)
        if m:
            headings.append(Heading(i, "annex", m.group(1), m.group(2).rstrip(), page))
            continue
        m = RE_ANNEX_SUB_HEADING.match(line)
        if m:
            headings.append(Heading(i, "annex", m.group(1), m.group(2).rstrip(), page))
            continue
        # Catch-all (tried last): every other "§   " line is a back-matter heading
        # (References, Index, ...). This keeps every section in the index.
        m = RE_OTHER_HEADING.match(line)
        if m:
            headings.append(Heading(i, "other", "", m.group(1).rstrip(), page))
    return headings


def plan_segments(lines: list[str], headings: list[Heading]) -> list[Segment]:
    """Partition the raw lines into output files, in canonical (raw) order.

    The top-level structure asserted below (chapters 1..9, annexes A..G) is
    **pinned to AV2 v1.0.0**. A future spec version with a different chapter/annex
    layout must update these expectations here, alongside the new ``--version`` and
    the ``SPEC_MIRRORS`` pin in ``xtask/src/main.rs``.
    """
    chapters = [h for h in headings if h.kind == "num" and h.components == 1]
    # Top-level annexes only (A..G); annex subsections (A.1, D.3.1, ...) are also
    # kind "annex" but live inside their annex file, not as split boundaries.
    annexes = [h for h in headings if h.kind == "annex" and h.components == 1]

    chapter_numbers = [int(h.number) for h in chapters]
    if chapter_numbers != list(range(1, 10)):
        raise SystemExit(
            "expected AV2 v1.0.0 chapters 1..9 in order (see plan_segments docstring "
            f"for new spec versions), found {chapter_numbers}"
        )
    annex_letters = [h.number for h in annexes]
    if annex_letters != ["A", "B", "C", "D", "E", "F", "G"]:
        raise SystemExit(
            f"expected annexes A..G in order, found {annex_letters}"
        )

    # Ordered list of top-level boundaries: each chapter heading, then each annex.
    tops = chapters + annexes
    boundaries = [h.line_idx for h in tops] + [len(lines)]

    segments: list[Segment] = []

    # Front matter: everything before the first chapter heading.
    if tops[0].line_idx > 0:
        segments.append(
            Segment("00-front-matter.md", "Front matter", 0, tops[0].line_idx)
        )

    for n, top in enumerate(tops):
        start = top.line_idx
        end = boundaries[n + 1]
        if top.kind == "num":
            num = int(top.number)
            if num in SUBSPLIT_CHAPTERS:
                segments.extend(_split_chapter_9(lines, top, start, end, headings))
                continue
            rel = f"{num:02d}-{slugify(top.title)}.md"
            title = f"§ {num}. {top.title}"
        else:
            rel = f"annex-{top.number.lower()}-{slugify(top.title)}.md"
            title = f"Annex {top.number}: {top.title}"
        segments.append(Segment(rel, title, start, end))

    # Attach the headings that fall inside each segment.
    h_by_line = sorted(headings, key=lambda h: h.line_idx)
    h_lines = [h.line_idx for h in h_by_line]
    for seg in segments:
        lo = bisect.bisect_left(h_lines, seg.start)
        hi = bisect.bisect_left(h_lines, seg.end)
        seg.headings = h_by_line[lo:hi]
    return segments


def _split_chapter_9(
    lines: list[str], top: Heading, start: int, end: int, headings: list[Heading]
) -> list[Segment]:
    """Sub-split §9 into one file per level-2 subsection (plus an overview)."""
    subs = [
        h
        for h in headings
        if h.kind == "num"
        and h.components == 2
        and h.number.startswith("9.")
        and start <= h.line_idx < end
    ]
    out: list[Segment] = []
    # Overview: chapter heading + any text before the first subsection.
    first_sub = subs[0].line_idx if subs else end
    if first_sub > start:
        out.append(
            Segment(
                f"{CH9_DIRNAME}/09-00-overview.md",
                f"§ 9. {top.title}",
                start,
                first_sub,
            )
        )
    sub_lines = [h.line_idx for h in subs] + [end]
    for i, sub in enumerate(subs):
        minor = sub.number.split(".")[1]
        rel = f"{CH9_DIRNAME}/09-{int(minor):02d}-{slugify(sub.title)}.md"
        out.append(
            Segment(rel, f"§ {sub.number} {sub.title}", sub.line_idx, sub_lines[i + 1])
        )
    return out


def render_segment(seg: Segment, lines: list[str]) -> str:
    """Render one output file: H1 + per-section (anchor + heading + fenced body)."""
    depth = seg.rel_path.count("/")
    root = "../" * depth if depth else "./"
    out: list[str] = []
    out.append(f"# AV2 v1.0.0 — {seg.title}")
    out.append("")
    out.append(
        "<!-- Verbatim mirror of the AOM AV2 v1.0.0 specification "
        "(© Alliance for Open Media). The PDF is normative; this is a faithful "
        f"`pdftotext -layout` copy. See [{root}README.md]({root}README.md) and "
        f"[{root}index.md]({root}index.md). Do not hand-edit: regenerate via "
        "scripts/spec/regenerate-av2-spec.sh. -->"
    )
    out.append("")

    hs = seg.headings
    cut = [h.line_idx for h in hs]

    def emit_fence(a: int, b: int) -> None:
        out.append(FENCE_OPEN)
        out.extend(lines[a:b])
        out.append(FENCE_CLOSE)
        out.append("")

    # Content before the first heading in this segment (front matter, or none).
    first = cut[0] if cut else seg.end
    if first > seg.start:
        emit_fence(seg.start, first)

    for i, h in enumerate(hs):
        nxt = cut[i + 1] if i + 1 < len(cut) else seg.end
        out.append(f'<a id="{h.anchor}"></a>')
        out.append("")
        out.append(f"{'#' * h.md_level} {h.heading_text}")
        out.append("")
        emit_fence(h.line_idx, nxt)

    text = "\n".join(out)
    if not text.endswith("\n"):
        text += "\n"
    return text


def extract_fenced(text: str) -> list[str]:
    """Return the verbatim lines inside ```text fences (round-trip helper)."""
    result: list[str] = []
    in_fence = False
    for line in text.split("\n"):
        if not in_fence and line == FENCE_OPEN:
            in_fence = True
            continue
        if in_fence and line == FENCE_CLOSE:
            in_fence = False
            continue
        if in_fence:
            result.append(line)
    return result


def assert_roundtrip(segments: list[Segment], rendered: dict[str, str], raw_lines: list[str]) -> None:
    """Re-read generated text and verify fenced content == raw, byte for byte."""
    recovered: list[str] = []
    for seg in segments:
        recovered.extend(extract_fenced(rendered[seg.rel_path]))
    if recovered != raw_lines:
        n = min(len(recovered), len(raw_lines))
        first_diff = next(
            (i for i in range(n) if recovered[i] != raw_lines[i]), n
        )
        raise SystemExit(
            "ROUND-TRIP FAILED: generated mirror is not byte-faithful.\n"
            f"  recovered {len(recovered)} lines, raw {len(raw_lines)} lines\n"
            f"  first difference at line {first_diff}:\n"
            f"    raw:       {raw_lines[first_diff] if first_diff < len(raw_lines) else '<eof>'!r}\n"
            f"    recovered: {recovered[first_diff] if first_diff < len(recovered) else '<eof>'!r}"
        )


def render_index(segments: list[Segment], headings: list[Heading], seg_of: dict[int, Segment]) -> str:
    out: list[str] = []
    out.append("# AV2 v1.0.0 specification — section index")
    out.append("")
    out.append(
        "Canonical map from every AV2 v1.0.0 `§` section to its file, anchor, and "
        "PDF page. This mirror is a faithful `pdftotext -layout` copy of the AOM "
        "specification (see [README.md](./README.md)). The PDF is normative."
    )
    out.append("")
    out.append("| Section | Title | File | Page |")
    out.append("| --- | --- | --- | --- |")
    for h in headings:
        seg = seg_of.get(h.line_idx)
        if seg is None:
            continue
        link = f"[{seg.rel_path}]({seg.rel_path}#{h.anchor})"
        title = h.title.replace("|", "\\|")
        out.append(f"| `{h.index_label}` | {title} | {link} | {h.page} |")
    out.append("")
    return "\n".join(out)


def render_readme(meta: dict[str, str]) -> str:
    return f"""# AV2 v1.0.0 specification mirror

> **Copyright {meta['copyright']}.**
> This directory is a faithful, verbatim mirror of the **AV2 Bitstream &
> Decoding Process Specification, version {meta['version']}**, produced by
> converting the official AOM PDF with poppler's `pdftotext -layout`.
>
> This material is **NOT** covered by the repository's PolyForm Noncommercial
> license. It is reproduced here, with attribution, as third-party reference
> material under the copyright of the Alliance for Open Media. See
> [../../../references/THIRD-PARTY-NOTICES.md](../../../references/THIRD-PARTY-NOTICES.md).

## Status of this copy

- The **PDF is the normative reference.** This Markdown is a navigation and
  citation aid: every byte of spec text is preserved verbatim inside
  ```` ```text ```` fences; only `§` navigation headings and anchors are added.
- Tables, math, and pseudocode are preserved as monospaced text (alignment from
  `-layout`), not reconstructed into Markdown tables.

## How to use it

- Start from [index.md](./index.md): it maps every `§` section to its file,
  anchor, and PDF page.
- Cite the section number (e.g. `§ 5.16`) plus the mirror path, e.g.
  `docs/spec/av2/{meta['version']}/05-syntax-structures.md#s-5-16`.
- `§ 9` (additional tables) is large and lives under
  [`{CH9_DIRNAME}/`](./{CH9_DIRNAME}/).

## Provenance

| Field | Value |
| --- | --- |
| Spec | {meta['spec']} |
| Version | {meta['version']} |
| PDF URL | {meta['pdf_url']} |
| PDF sha256 | `{meta['pdf_sha256']}` |
| HTML source | {meta['source_html']} |
| Converter | `pdftotext {meta['pdftotext_args']}` ({meta['poppler_version']}) |

Machine-readable provenance is in [provenance.toml](./provenance.toml).

## Regenerating

This mirror is generated, not hand-written. Do not edit these files directly —
the `cargo xtask check-spec-mirror` gate fails on any drift. To regenerate (e.g.
for a new spec version):

```sh
scripts/spec/regenerate-av2-spec.sh \\
  --version {meta['version']} \\
  --url {meta['pdf_url']} \\
  --sha256 {meta['pdf_sha256']}
```

The `{meta['poppler_version']}` line records the poppler build used; `-layout`
output can vary across poppler versions, so reproduce with a matching toolchain
or run the script's `--verify` mode.
"""


def render_provenance(meta: dict[str, str], attachment: "Attachment | None") -> str:
    base = f"""# Provenance of the committed AV2 v1.0.0 specification mirror.
# Generated by scripts/spec/regenerate-av2-spec.sh — do not hand-edit.
# This records exactly which PDF and converter produced the mirror. The
# `cargo xtask check-spec-mirror` gate checks pdf_sha256 against the pinned value.

spec = "{meta['spec']}"
version = "{meta['version']}"
pdf_url = "{meta['pdf_url']}"
pdf_sha256 = "{meta['pdf_sha256']}"
source_html = "{meta['source_html']}"
poppler_version = "{meta['poppler_version']}"
pdftotext_args = "{meta['pdftotext_args']}"
generated_by = "scripts/spec/regenerate-av2-spec.sh"
"""
    if attachment is None:
        return base
    # The § 9 "additional tables" attachment (all_tables.h) is fetched verbatim
    # from the spec website and committed under attachments/. `cargo xtask
    # gen-tables` generates the splot-core tables from it; check-spec-mirror pins
    # its sha256 like the PDF.
    return base + f"""
[attachments.all_tables_h]
path = "attachments/all_tables.h"
url = "{attachment.url}"
sha256 = "{attachment.sha256}"
"""


def sha256_text(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def main() -> int:
    ap = argparse.ArgumentParser(description="Build the AV2 spec mirror.")
    ap.add_argument("--raw", required=True, type=Path, help="pdftotext -layout output")
    ap.add_argument("--outdir", required=True, type=Path, help="mirror root directory")
    ap.add_argument("--version", required=True)
    ap.add_argument("--pdf-url", required=True)
    ap.add_argument("--pdf-sha256", required=True)
    ap.add_argument("--poppler-version", required=True)
    ap.add_argument("--source-html", default="")
    ap.add_argument("--spec-name", default="AV2 Bitstream & Decoding Process Specification")
    ap.add_argument("--copyright", default="2026, Alliance for Open Media")
    ap.add_argument(
        "--attachment",
        type=Path,
        default=None,
        help="local copy of the § 9 additional-tables attachment (all_tables.h)",
    )
    ap.add_argument("--attachment-url", default="")
    ap.add_argument("--attachment-sha256", default="")
    args = ap.parse_args()

    attachment: Attachment | None = None
    if args.attachment is not None:
        if not args.attachment_url or not args.attachment_sha256:
            raise SystemExit(
                "--attachment requires --attachment-url and --attachment-sha256"
            )
        att_bytes = args.attachment.read_bytes()
        got = hashlib.sha256(att_bytes).hexdigest()
        if got != args.attachment_sha256:
            raise SystemExit(
                f"attachment sha256 mismatch: expected {args.attachment_sha256}, got {got}"
            )
        attachment = Attachment(
            src=args.attachment,
            rel_path="attachments/all_tables.h",
            url=args.attachment_url,
            sha256=got,
        )

    raw_text = args.raw.read_text(encoding="utf-8", errors="strict")
    raw_lines = raw_text.split("\n")

    if any(line == FENCE_CLOSE for line in raw_lines):
        raise SystemExit("raw text contains a ``` fence delimiter; choose a longer fence")

    footers = build_page_index(raw_lines)
    headings = detect_headings(raw_lines, footers)
    segments = plan_segments(raw_lines, headings)

    seg_of: dict[int, Segment] = {}
    for seg in segments:
        for h in seg.headings:
            seg_of[h.line_idx] = seg

    rendered: dict[str, str] = {seg.rel_path: render_segment(seg, raw_lines) for seg in segments}
    assert_roundtrip(segments, rendered, raw_lines)

    meta = {
        "spec": args.spec_name,
        "version": args.version,
        "pdf_url": args.pdf_url,
        "pdf_sha256": args.pdf_sha256,
        "source_html": args.source_html or f"https://av2.aomedia.org/v{args.version}/index.html",
        "poppler_version": args.poppler_version,
        "pdftotext_args": "-layout",
        "copyright": args.copyright,
    }

    index_md = render_index(segments, headings, seg_of)
    readme_md = render_readme(meta)
    provenance_toml = render_provenance(meta, attachment)

    outdir: Path = args.outdir
    if outdir.exists():
        # Remove previously generated content so deletions are reflected, but
        # refuse to wipe a directory that is neither empty nor a prior mirror —
        # this guards against a typoed/wrong --outdir (e.g. "." or "$HOME").
        looks_like_mirror = (outdir / "provenance.toml").exists() or (
            outdir / "CHECKSUMS"
        ).exists()
        if any(outdir.iterdir()) and not looks_like_mirror:
            raise SystemExit(
                f"refusing to wipe non-mirror directory {outdir} "
                "(no provenance.toml/CHECKSUMS); pass an empty or previously generated --outdir"
            )
        import shutil

        shutil.rmtree(outdir)
    outdir.mkdir(parents=True, exist_ok=True)

    files: dict[str, str] = {}
    for rel, text in rendered.items():
        files[rel] = text
    files["index.md"] = index_md + "\n" if not index_md.endswith("\n") else index_md
    files["README.md"] = readme_md
    files["provenance.toml"] = provenance_toml

    for rel, text in files.items():
        path = outdir / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    # CHECKSUMS covers every generated file (except CHECKSUMS itself), sorted.
    # The verbatim attachment is hashed over its raw bytes (it is third-party C,
    # not UTF-8-normalized text), copied byte-for-byte, and listed alongside the
    # structured files so `check-spec-mirror` pins it too.
    checksums: dict[str, str] = {rel: sha256_text(files[rel]) for rel in files}
    if attachment is not None:
        att_bytes = attachment.src.read_bytes()
        att_path = outdir / attachment.rel_path
        att_path.parent.mkdir(parents=True, exist_ok=True)
        att_path.write_bytes(att_bytes)
        checksums[attachment.rel_path] = hashlib.sha256(att_bytes).hexdigest()

    checksum_lines = [f"{checksums[rel]}  {rel}" for rel in sorted(checksums)]
    (outdir / "CHECKSUMS").write_text("\n".join(checksum_lines) + "\n", encoding="utf-8")

    section_count = sum(1 for _ in headings)
    print(
        f"mirror: {len(files)} content files + CHECKSUMS, {section_count} sections, "
        f"{len(raw_lines)} raw lines — round-trip OK"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
