# av2-spec-mirror Specification

## Purpose
TBD - created by archiving change add-av2-spec-mirror. Update Purpose after archive.
## Requirements
### Requirement: committed faithful AV2 spec mirror

The repository SHALL contain a committed, versioned mirror of the AV2 v1.0.0
specification under `docs/spec/av2/1.0.0/`, derived from the official AOM PDF by a
`pdftotext -layout` conversion and split into one Markdown file per chapter and
annex. The normative spec text SHALL be preserved byte-for-byte inside fenced
code blocks; only navigation headings derived from `§` lines may be injected.
Tracked by `DOC-AV2-SPEC-MIRROR`.

#### Scenario: spec text is byte-faithful to the PDF conversion

- **WHEN** the injected `§` headings and code-fence delimiter lines are removed
  from the committed chapter files and the bodies are concatenated in chapter order
- **THEN** the result is identical to the raw `pdftotext -layout` output of the
  pinned PDF (the regeneration script asserts this round-trip before writing)

#### Scenario: a future spec version does not mutate v1.0.0

- **WHEN** a later AV2 version is mirrored
- **THEN** it is written to a new `docs/spec/av2/<version>/` directory and the
  `docs/spec/av2/1.0.0/` tree remains unchanged

### Requirement: stable section referencing

The mirror SHALL provide a committed `index.md` mapping every `§` heading to its
containing file, in-file anchor, and PDF page number, so that agents, docs, and
code comments can deep-link any AV2 section without network access.

#### Scenario: resolving a section reference

- **WHEN** a contributor needs the normative text for `§ 5.16 Padding OBU syntax`
- **THEN** `index.md` resolves it to a specific `file#anchor` in the mirror and
  the anchor exists in that file

### Requirement: provenance and reproducible regeneration

The mirror SHALL record its provenance in `provenance.toml` (spec version, PDF
URL, PDF sha256, source HTML URL, poppler version, and conversion arguments), and
the repository SHALL provide a standalone script `scripts/spec/regenerate-av2-spec.sh`
that regenerates the entire mirror from a pinned PDF. The script SHALL verify the
downloaded PDF's sha256 against the expected value before converting and SHALL
abort on mismatch. The conversion SHALL NOT depend on markitdown.

#### Scenario: regenerating from a tampered or wrong PDF

- **WHEN** the script downloads a PDF whose sha256 does not match the expected
  value passed for that version
- **THEN** the script aborts without writing any mirror files

#### Scenario: regeneration is deterministic

- **WHEN** the script is run twice against the same pinned PDF with the same
  poppler version
- **THEN** it produces identical mirror files, `index.md`, and `CHECKSUMS`

### Requirement: committed-mirror integrity gate

The repository SHALL provide a `cargo xtask check-spec-mirror` command, run as
part of `cargo xtask ci`, that fails when any committed mirror content file's
sha256 does not match the committed `CHECKSUMS` manifest, when the `CHECKSUMS`
manifest's own sha256 does not match the value pinned in source
(`SPEC_MIRRORS`), or when `provenance.toml` does not pin the expected PDF sha256
for the mirrored version. The gate SHALL be deterministic and SHALL NOT require
running `pdftotext`. Pinning the manifest hash in source (outside the mirror)
SHALL prevent a content edit from being laundered by also editing `CHECKSUMS`.

#### Scenario: hand-edited mirror file is rejected

- **WHEN** any file under `docs/spec/av2/1.0.0/` is modified without updating
  `CHECKSUMS` via the regeneration script
- **THEN** `cargo xtask check-spec-mirror` (and therefore `cargo xtask ci`) fails

#### Scenario: edit laundered through CHECKSUMS is still rejected

- **WHEN** a mirror file is modified **and** its `CHECKSUMS` line is updated to
  match, but the manifest hash pinned in `SPEC_MIRRORS` is not updated
- **THEN** `cargo xtask check-spec-mirror` fails because the `CHECKSUMS` sha256 no
  longer matches the pinned value

#### Scenario: clean mirror passes

- **WHEN** the committed mirror, `CHECKSUMS` (matching its pinned hash), and
  `provenance.toml` are consistent
- **THEN** `cargo xtask check-spec-mirror` succeeds without needing poppler

### Requirement: third-party license quarantine

The mirror SHALL be isolated as Alliance for Open Media copyright material,
distinct from the repository's PolyForm Noncommercial license. A `README.md` in
the mirror directory SHALL state the AOM copyright, that the PDF is normative and
the mirror is a faithful copy, and `docs/references/THIRD-PARTY-NOTICES.md` SHALL
record the exception. Mirror files SHALL NOT carry the PolyForm SPDX header, and
`docs/spec/**` SHALL be excluded from the `typos` spell-check.

#### Scenario: notices and isolation are present

- **WHEN** the mirror is committed
- **THEN** the mirror `README.md` carries the AOM copyright notice,
  `THIRD-PARTY-NOTICES.md` lists the AV2 spec exception, and no mirror file
  carries the PolyForm SPDX header

