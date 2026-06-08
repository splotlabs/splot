# Design: add-av2-spec-mirror

## Context

`splot` cites AV2 v1.0.0 `§` sections throughout its code and docs, but the
normative text lives only at `av2.aomedia.org` (PDF + HTML). There is no
committed, offline, greppable copy, so the "ground every claim in the spec"
principle relies on network access and human recall. AGENTS.md §6 already forbids
inventing syntax; this change supplies the artifact that makes that rule
enforceable from inside the repo.

The official PDF is `20260528_38f28e7_AV2_Spec_v1.0.0.pdf` (1169 pages,
sha256 `e9916f091e4e83446aad6b4601641c5b292e569c144c4163b26a4497573b533f`),
© Alliance for Open Media. AOM publishes the spec openly but grants no explicit
redistribution license; the maintainer has accepted the redistribution risk and
authorised committing a quarantined, attributed copy (AGENTS.md §9/§10).

## Goals / Non-Goals

**Goals:**

- A byte-faithful, versioned, offline copy of the AV2 v1.0.0 spec in the repo.
- Stable, referenceable anchors so agents/docs/code can deep-link `§` sections.
- Deterministic regeneration from a pinned PDF, producing clean diffs.
- A CI gate that guarantees the committed copy cannot silently drift or be
  hand-edited, and that records exactly which PDF it derives from.
- Clear license isolation so the AOM material never contaminates PolyForm code.

**Non-Goals:**

- Pretty prose Markdown, semantic table reconstruction, or math rendering.
- Any change to AV2 parsing/validation behavior or the crate dependency graph.
- Re-running `pdftotext` inside CI (output varies by poppler version).
- Mirroring AV2 versions other than v1.0.0.

## Decisions

### D1. Conversion engine: `pdftotext -layout` (poppler), not markitdown

Empirically trialed both on the real PDF. markitdown's pdfminer backend garbles
tabular content and TOC dot-leaders (interleaved `8. 5.` noise) and needs a heavy
`[pdf]` extra. `pdftotext -layout` preserves pseudocode indentation, the
`Descriptor` column, `§ N.M Title` headings, and `Page N of 1169` markers.
The user suggested markitdown as a *means*; fidelity is the goal, so poppler wins.
markitdown is never installed into or vendored by the repo.

_Alternative considered:_ markitdown (rejected: lower fidelity, heavier deps).
_Alternative considered:_ AVM/HTML scraping (rejected: HTML spec is a separate
artifact; PDF is the citable normative deliverable and pins cleanly by hash).

### D2. Location & granularity: `docs/spec/av2/1.0.0/`, one file per chapter/annex

Versioned path keeps future releases as independent frozen directories. Splitting
at the top level (§1–§9, Annex A–G + a front-matter file) keeps citation simple:
the chapter number maps to a file, and `index.md` resolves the exact anchor. §9
"Additional tables" is ~half the document (~1.5 MB), so it is sub-split into its
eight level-2 subsections (plus an overview file) under `09-additional-tables/`
for renderability.

_Alternative considered:_ single monolithic file (rejected: unwieldy, no
GitHub render, poor diffs). _Alternative considered:_ uniform level-2 split into
~88+ files (rejected: more files than navigation benefit; `index.md` already
provides fine-grained lookup).

### D3. Format: headings + verbatim fenced bodies

Each `§ N[.M[.K]]. Title` line becomes a Markdown heading at depth = dotted
level (capped at `######`), with the original `§ …` text preserved in the heading
so exact-citation grep still works and GitHub generates a stable `#anchor`. All
body content (pseudocode, tables, prose, page footers) is emitted **verbatim**
inside ` ```text ` fences, preserving column alignment and byte-exactness.

**Invariant (verifiable):** concatenating chapters and stripping the injected
headings and fence lines reproduces the raw `pdftotext -layout` output exactly.
The regeneration script asserts this round-trip before writing.

_Alternative considered:_ plain `.txt` slices + line-anchor index (rejected:
references would use line numbers, less stable than section anchors; content
would not be Markdown). _Alternative considered:_ full prose-to-Markdown
transform (rejected: distinguishing prose from pseudocode is error-prone and
risks corrupting normative wording).

### D4. Regeneration: standalone `scripts/spec/regenerate-av2-spec.sh`

A small POSIX script taking `--version`, `--url`, `--sha256`. Steps: download →
verify sha256 (abort on mismatch) → `pdftotext -layout` → split by chapter/annex
→ inject headings + fences → write `index.md`, `provenance.toml`, `CHECKSUMS` →
assert the D3 round-trip. A `--verify` mode regenerates into a temp dir and diffs
against the committed tree (full parity, for the pinned poppler version). No Rust
toolchain required; matches the maintainer's "simple, repeatable script" ask and
parameterises cleanly for a future AV2 1.1.0.

### D5. Enforcement: `cargo xtask check-spec-mirror`, deterministic, in CI

The CI gate must be stable across poppler versions, so it does **not** re-run
`pdftotext`. Instead it: (a) recomputes each committed content file's sha256
(using the `sha2` crate) and compares to `CHECKSUMS` (detects hand-edits/drift),
and (b) confirms `provenance.toml` pins the expected PDF sha256 for v1.0.0. Wired
into `run_ci()` alongside `check_license_headers` / `check_dependency_direction`.
Full re-derivation parity remains available locally via the script's `--verify`
mode. Hashing uses the well-vetted `sha2` crate (a maintainer-approved `xtask`
dependency) rather than a hand-rolled implementation — crypto primitives are not
reimplemented in-tree.

_Alternative considered:_ CI re-runs the conversion and diffs (rejected: flaky —
`-layout` output differs across poppler builds).

### D6. License quarantine

`docs/spec/av2/1.0.0/README.md` states the directory is AOM-copyright material,
not PolyForm; the PDF is normative and this is a faithful mirror. A
`THIRD-PARTY-NOTICES.md` entry records the exception (maintainer-approved per
AGENTS.md §9). The mirror files carry **no** PolyForm SPDX header (the
`check-license-headers` gate only inspects `.rs`, so this is consistent), and
`_typos.toml` excludes `docs/spec/**`.

## Risks / Trade-offs

- **Redistribution of AOM-copyright text** → Maintainer-accepted (AGENTS.md §10
  decision); quarantined, attributed, pinned by hash; isolated from PolyForm code.
- **poppler version drift changes `-layout` output** → CI gate uses the
  checksum manifest, not re-derivation; `provenance.toml` records the poppler
  version used so regenerations are reproducible against a known toolchain.
- **Conversion is lossy for complex tables/math** → Accepted and documented; the
  PDF remains the normative reference, the mirror is a faithful-but-textual
  navigation/citation aid. Byte-exactness inside fences keeps it honest.
- **Large committed footprint (~3.5 MB)** → One-time; text compresses well in
  git; split files keep individual diffs small on regeneration.
- **Heading-injection bug could mislabel a section** → The D3 round-trip
  assertion + `index.md` derived from the same pass + cross-check against the
  TOC catch mislabeling deterministically.

## Migration Plan

Additive only. New files + new opt-in xtask gate; nothing existing changes
behavior. Rollback = delete `docs/spec/av2/`, the script, the xtask subcommand +
CI line, and revert the doc/notice/typos edits. No data migration.

## Open Questions

None outstanding — location, granularity, format, regeneration home, and
enforcement strictness were resolved during brainstorming and approved.
