# Validator next phase: sequence + HLS coverage

`status: proposed`
`owner: validator`
`primary change: openspec/changes/sequence-hls-validator-coverage`

## Goal

Move `splot` from a header/envelope validator with a partial sequence-header parser to a validator that can:

1. parse the remainder of `sequence_header_obu()` in safe, bounded Rust;
2. validate locally decidable §6.4 sequence-header semantics;
3. remember and compare sequence headers in validator state;
4. enforce more of §6.2.2 through activated sequence-header state;
5. enforce the sequence/HLS subset of §7.3.7 and §7.3.8 without pretending to validate frame/tile payloads.

This is still a validator-first phase. It is not an encoder phase and not a decoder phase.

## Implementation slices

### Slice 0 — OpenSpec and matrix alignment

Before Rust edits:

```bash
git status --short
cargo xtask feature-status --format table
cargo xtask spec-coverage
```

Then:

- add or update `openspec/changes/sequence-hls-validator-coverage/`;
- confirm every touched Feature ID exists in `docs/IMPLEMENTATION-MATRIX.toml`;
- add no bare `TODO(spec)` markers; use `TODO(spec: FEATURE-ID): ...` only;
- regenerate `docs/FEATURE-STATUS.md` only after code/tests/proof change.

### Slice 1 — Sequence parser completion, shallow children first

Implement child config parsers that do not require deep table generation or frame/tile decoding:

- `AV2-5.4.3-SEQUENCE-PARTITION-CONFIG`
- `AV2-5.4.5-SEQUENCE-INTRA-CONFIG`
- `AV2-5.4.7-SEQUENCE-SCC-CONFIG`
- `AV2-5.4.12-TIMING-INFO`
- `AV2-5.4.13-SEQUENCE-DECODER-MODEL-INFO`

Target module:

```text
crates/splot-core/src/headers/sequence.rs
```

Parser rules:

- preserve inferred values explicitly in typed structs;
- return typed parse errors on EOF or invalid descriptors;
- do not allocate large tables;
- do not read past the OBU payload boundary;
- add EOF tests at field boundaries.

### Slice 2 — Sequence inter/TQ/filter children

Implement more complex but still sequence-local tool-flag parsers:

- `AV2-5.4.6-SEQUENCE-INTER-CONFIG`
- `AV2-5.4.8-SEQUENCE-TQ-ENTROPY-CONFIG`
- `AV2-5.4.10-SEQUENCE-FILTER-CONFIG`

Keep these parsers field-level and syntax-exact. They should not implement motion estimation, transforms, loop filtering, entropy coding, or a decoder. They only read sequence-level feature flags and derived variables needed by later validators.

### Slice 3 — Table-dependent children behind explicit boundaries

Handle the rows that depend on shared structures or spec tables:

- `AV2-5.4.2-SEQUENCE-TILE-CONFIG` calls `tile_params(...)`. **Done** (OpenSpec
  `segmentation-tile-params-foundation`): the shared `tile_params()` helper
  (`AV2-5.18.7.3-TILE-PARAMS`) is implemented and wired in; only a reserved
  `seq_level_idx` stays bounded.
- `AV2-5.4.4-SEQUENCE-SEGMENT-CONFIG` may call `seg_info(MaxSegments)`. **Done**: wired
  to `AV2-5.4.9-SEGMENT-INFO`.
- `AV2-5.4.9-SEGMENT-INFO` uses segmentation feature tables. **Done**: reusable
  `seg_info(numSegments)` with the §5.4.9 feature tables and the `su(n)` descriptor
  (`AV2-4.11.7-SU`); also wired into the multi-frame header (`AV2-5.7-MULTI-FRAME-HEADER`).
- `AV2-5.4.11-USER-QM` uses transform-size/scan/QM tables. (Still future.)

Preferred behavior:

- if the required table/helper is already available, implement the parser and record proof;
- otherwise add a bounded `PayloadStatus::Unimplemented { feature, payload }` or typed `Error::Unimplemented { feature }` at the exact child feature boundary;
- never hand-transcribe large tables unless the matrix explicitly allows it;
- if a helper deserves its own matrix row, add it before code.

### Slice 4 — Sequence semantics and state

Strengthen `splot-validate`:

- compare repeated activated sequence headers for bit-identical payloads within a coded video sequence;
- preserve one active sequence header per extended layer until a CLK/reset condition can be modeled;
- enforce `max_tlayer_id` / `max_mlayer_id` checks for OBUs that have an active sequence;
- validate local range rules for new child fields;
- track timing values across embedded layers where the spec requires consistency and the relevant payloads are parseable;
- keep state local to `ValidatorContext`, no global mutable state.

Target modules:

```text
crates/splot-validate/src/context.rs
crates/splot-validate/src/checks/mod.rs
crates/splot-validate/src/validator.rs
```

### Slice 5 — HLS availability foundation

Implement the minimum HLS payload/state needed to make §7.3.8 less partial:

- `AV2-5.5-TEMPORAL-DELIMITER`: empty payload/state reset semantics.
- `AV2-5.6-MSDO`: parse fields, enforce base/global layer ids and `num_streams_minus_2 <= 2`.
- `AV2-5.7-MULTI-FRAME-HEADER`: parse the header fields needed to store `mfh_seq_header_id`, `mfh_id`, layer ids, optional frame size, and future frame-header references.

Do not implement full LCR, OPS, atlas, frame, or tile parsing in this slice unless the implementation naturally stays small and fully tested. Leave those as next-phase rows if they grow.

### Slice 6 — Inspector and fixtures

The validator is also an inspector. Update inspect output so developers can see sequence/HLS state without decoding frames:

- `inspect --json` should include parsed sequence-header child fields where implemented;
- unimplemented child payload sections should report a stable feature ID;
- add fixtures for minimal still-picture, non-still, timing, and HLS ordering cases;
- add snapshot tests only if the repo already has the chosen snapshot dependency and generated output policy.

## Acceptance commands

Run all commands from repo root:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test -p splot-core sequence
cargo test -p splot-validate sequence_header
cargo test -p splot-validate hls
cargo test -p splot-cli inspect
cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md
cargo xtask check-feature-status
cargo xtask spec-coverage
cargo xtask ci
```

## Done criteria

This phase is done when:

- all implemented sequence child rows have `parse = done`, `tests = done`, and proof entries;
- unimplemented sequence child rows are explicitly bounded and still honest in the matrix;
- sequence semantics have stronger `validate = partial` or `validate = done` proof, not just new types;
- activated sequence limit checks no longer depend only on the general parser;
- temporal/HLS ordering diagnostics are stable and covered by tests;
- malformed sequence/HLS payloads never panic under unit tests and proptests.
