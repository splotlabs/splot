# Tasks: add-av2-spec-mirror

## 1. Matrix and tracking

- [x] 1.1 Add `DOC-AV2-SPEC-MIRROR` row to `docs/IMPLEMENTATION-MATRIX.toml`
  (category/kind `docs`, `openspec_change = "add-av2-spec-mirror"`, statuses with
  `mapped`/`tests` set and the rest `not-applicable`, proof recorded once gate exists).
- [x] 1.2 Confirm `cargo xtask check-feature-status` accepts the new row.

## 2. Regeneration script

- [x] 2.1 Write `scripts/spec/regenerate-av2-spec.sh` (POSIX): args `--version`,
  `--url`, `--sha256`, optional `--verify`; require `pdftotext` (poppler) and abort
  with an install hint if absent.
- [x] 2.2 Download the pinned PDF to a temp path and verify sha256; abort on mismatch.
- [x] 2.3 Run `pdftotext -layout` to a raw text file; record poppler version.
- [x] 2.4 Split the raw text by top-level chapter (§1–§9) and annex (A–G) plus a
  front-matter segment; sub-split §9 by its level-2 subsections.
- [x] 2.5 Emit per-file Markdown: inject `§ N.M Title` headings (depth = dotted
  level, capped at `######`, original `§` text preserved) and wrap every body
  segment verbatim in ` ```text ` fences.
- [x] 2.6 Generate `index.md` (every `§` heading → file#anchor + page number) and
  the `09-additional-tables/` directory layout.
- [x] 2.7 Write `provenance.toml` (spec version, pdf_url, pdf_sha256, source_html_url,
  poppler_version, pdftotext_args) and `CHECKSUMS` (sha256 per generated content file).
- [x] 2.8 Assert the round-trip invariant: stripping injected headings + fence
  lines and concatenating reproduces the raw `pdftotext -layout` output; fail otherwise.
- [x] 2.9 Implement `--verify`: regenerate into a temp dir and diff against the
  committed tree; non-zero exit on any difference.

## 3. Generate the committed mirror

- [x] 3.1 Run the script for v1.0.0 with the pinned URL and sha256
  `e9916f091e4e83446aad6b4601641c5b292e569c144c4163b26a4497573b533f`.
- [x] 3.2 Spot-check fidelity: `§ 5.2 OBU syntax`, `§ 5.16 Padding OBU syntax`,
  a §9 CDF table, and Annex B render with intact alignment; anchors resolve via `index.md`.
- [x] 3.3 Add `docs/spec/av2/1.0.0/README.md`: AOM copyright notice, "PDF is
  normative; this is a faithful mirror", how-to-cite convention, regenerate command,
  poppler-version reproducibility note. No PolyForm SPDX header on any mirror file.

## 4. Integrity gate (xtask)

- [x] 4.1 Add a `CheckSpecMirror` subcommand to `xtask/src/main.rs`: recompute each
  committed content file's sha256 and compare to `CHECKSUMS`; verify `provenance.toml`
  pins the expected PDF sha256 for v1.0.0; deterministic, no `pdftotext` call.
- [x] 4.2 Wire `check_spec_mirror` into `run_ci()` next to the other repo checks.
- [x] 4.3 Add an xtask unit test (tampered checksum → failure; clean → success).

## 5. Single-source-of-truth wire-up

- [x] 5.1 AGENTS.md: add a section making `docs/spec/av2/<version>/` the canonical
  offline source; require all code/docs/diagnostics/agents to cite the committed
  mirror (§ + anchor); cross-reference from §6 (spec honesty) and §9 (licensing).
- [x] 5.2 `docs/SPEC-MAPPING.md`: list the committed mirror as the primary offline
  reference; keep upstream PDF/HTML as source.
- [x] 5.3 `docs/references/THIRD-PARTY-NOTICES.md`: add the AOM AV2 spec exception entry.
- [x] 5.4 `_typos.toml`: add `docs/spec/**` to `[files].extend-exclude`.

## 6. Verification

- [x] 6.1 `cargo xtask check-spec-mirror` passes; flipping one mirror byte makes it fail.
- [x] 6.2 `openspec validate add-av2-spec-mirror --strict` passes.
- [x] 6.3 `cargo xtask feature-status` and `cargo xtask check-feature-status` pass.
- [x] 6.4 `cargo xtask ci` passes (fmt, clippy, build, test, typos, machete, deny,
  license headers, dependency direction, feature status, spec mirror).
- [x] 6.5 Record proof commands in the matrix row and mark applicable stages `done`.
