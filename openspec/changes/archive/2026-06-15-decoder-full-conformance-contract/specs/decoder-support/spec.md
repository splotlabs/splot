## ADDED Requirements

### Requirement: Full decoder conformance contract

The repository SHALL provide `docs/DECODER-FULL-CONFORMANCE.md` as the public
contract for the future AV2 v1.0.0 full decoder conformance claim. The document
SHALL state the current decoder status without overclaiming, define the final
conditions for claiming full conformance, distinguish raw intermediate output
from post-film-grain output, describe deterministic diagnostics and output-file
safety requirements, and preserve the local-only AVM/dav2d evidence boundary.
Tracked by `DOC-DECODER-FULL-CONFORMANCE-CONTRACT`.

#### Scenario: Reader checks current decoder status

- **WHEN** a reader opens `docs/DECODER-FULL-CONFORMANCE.md`
- **THEN** the document says that `splot decode` is not yet a full AV2 decoder
- **AND** it points readers to the generated decoder support and decoder spec
  coverage documents for current status

#### Scenario: Reader checks the future conformance claim

- **WHEN** a reader checks the future definition of full decoder conformance
- **THEN** the document requires support for every normative AV2 v1.0.0
  decode-relevant section within configured resource limits
- **AND** it requires zero temporary `decode/unsupported-feature` diagnostics for
  conforming streams before any full-conformance claim is allowed

#### Scenario: Reader checks reference evidence boundaries

- **WHEN** a reader checks how AVM or dav2d may be used for decoder evidence
- **THEN** the document states that reference evidence is committed as
  non-executable metadata only
- **AND** repository code, tests, `xtask`, CI, setup scripts, wrappers, and
  dependencies SHALL NOT locate, build, invoke, cache, or require AVM or dav2d

### Requirement: Generated decoder spec coverage document

The repository SHALL provide a generated `docs/DECODER-SPEC-COVERAGE.md`
document that maps AV2 v1.0.0 decoder-relevant section families to current
implementation ownership and evidence. Each row SHALL include `spec_sections`,
`spec_title`, `normative_status`, `implementation_owner`,
`decoder_support_rows`, `feature_ids`, `status`, `tests`, `fuzz_targets`,
`local_reference_evidence`, `diagnostics`, and `notes`. The allowed
`normative_status` values SHALL be `normative`, `informative`, and `mixed`; rows
with `normative_status = "mixed"` SHALL include notes explaining which portion is
normative for decoder conformance. The allowed row statuses SHALL be
`unsupported`, `partial`, `supported`, `blocked`, and
`out-of-scope-nonnormative`. Tracked by
`XTASK-DECODER-CONFORMANCE-COVERAGE`.

#### Scenario: Coverage document is generated

- **WHEN** `cargo xtask decoder-conformance-coverage --format markdown --output docs/DECODER-SPEC-COVERAGE.md` runs
- **THEN** the command writes a deterministic Markdown render of the decoder
  conformance coverage rows
- **AND** the output includes every section family needed by the full decoder
  conformance contract

#### Scenario: Normative status is explicit

- **WHEN** a generated row has `normative_status = "mixed"`
- **THEN** the row notes which cited sections are normative for decoder
  conformance and which cited sections are informative context

#### Scenario: Unsupported runtime sections remain visible

- **WHEN** a decode-relevant AV2 section family has no runtime decoder owner
- **THEN** the generated coverage document records `status = "unsupported"` or
  `status = "partial"` rather than omitting the section family
- **AND** the notes explain the missing runtime owner or remaining evidence gap

#### Scenario: Supported coverage row requires proof

- **WHEN** a decoder conformance coverage row has `status = "supported"`
- **THEN** the row records at least one self-contained test or proof reference
- **AND** runtime decode support SHALL NOT be marked supported from parser-only,
  docs-only, or raw reference-output evidence alone

#### Scenario: Non-normative exclusions are explicit

- **WHEN** a row has `status = "out-of-scope-nonnormative"`
- **THEN** the row includes a note explaining why the section family is not
  required for AV2 decoder conformance

### Requirement: Decoder conformance coverage drift gate

The repository SHALL provide `cargo xtask check-decoder-conformance-coverage` as
a self-contained drift and honesty gate for `docs/DECODER-SPEC-COVERAGE.md`. The
gate SHALL be part of `cargo xtask ci`, SHALL run without AVM or dav2d, and SHALL
fail when generated coverage output or cross-links are inconsistent with
committed repository files. Tracked by `XTASK-DECODER-CONFORMANCE-COVERAGE`.

#### Scenario: Coverage document drifts

- **WHEN** the coverage rows change without regenerating
  `docs/DECODER-SPEC-COVERAGE.md`
- **THEN** `cargo xtask check-decoder-conformance-coverage` fails
- **AND** the failure names the regeneration command

#### Scenario: Coverage row has invalid status

- **WHEN** a decoder conformance coverage row uses a status outside the allowed
  status set
- **THEN** `cargo xtask check-decoder-conformance-coverage` fails and names the
  offending row

#### Scenario: Coverage row references missing evidence

- **WHEN** a decoder conformance coverage row names a decoder support row,
  Feature ID, diagnostic, or local reference evidence id that is absent from the
  committed support, implementation matrix, diagnostics, or evidence files
- **THEN** `cargo xtask check-decoder-conformance-coverage` fails and names the
  missing reference

#### Scenario: Normative owner lacks a Feature ID

- **WHEN** a normative or mixed decoder conformance coverage row names a decoder
  support row as an implementation owner
- **THEN** that support row SHALL have a non-empty Feature ID

#### Scenario: CI remains self-contained

- **WHEN** `cargo xtask ci` runs on a machine without AVM or dav2d installed
- **THEN** decoder conformance coverage checks pass or fail solely from committed
  repository files
- **AND** no external reference decoder is located or invoked
