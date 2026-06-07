 implement next validator coverage phase

You are working in the existing `splot` repository. This is a Rust AV2 toolkit, validator-first. Your task is to implement the next validator coverage phase: complete or honestly bound the remaining sequence-header child parsers, strengthen sequence/HLS validator state, and add the first HLS payload foundations. Do not implement the encoder, decoder, entropy/range coder, full frame-header parser, or tile-group payload parser in this task.

## 0. Mandatory first steps

Run these commands and read the files before editing:

```bash
git status --short
cargo xtask feature-status --format table
cargo xtask spec-coverage
```

Read:

```text
AGENTS.md
CLAUDE.md or .github/copilot-instructions.md if present
README.md
STATUS.md
docs/FEATURE-TRACKING.md
docs/IMPLEMENTATION-MATRIX.toml
docs/IMPLEMENTATION-MATRIX.schema.md
docs/FEATURE-STATUS.md
docs/VALIDATOR-ROADMAP.md
docs/CURRENT-VALIDATOR-STATE.md
docs/VALIDATOR-NEXT-PHASE.md
docs/VALIDATOR-SEQUENCE-HEADER-COVERAGE.md
docs/VALIDATOR-HLS-AVAILABILITY-STATE.md
docs/VALIDATOR-NEXT-DIAGNOSTICS.md
openspec/changes/sequence-hls-validator-coverage/proposal.md
openspec/changes/sequence-hls-validator-coverage/design.md
openspec/changes/sequence-hls-validator-coverage/tasks.md
```

Preserve all user work. Do not discard local changes. Do not push to a remote.

## 1. Non-negotiable project rules

- AV2 v1.0.0 is normative. Do not copy AV1 syntax, AV1 tables, AV1 OBU header assumptions, rav1e code, or SVT-AV1 code.
- `docs/IMPLEMENTATION-MATRIX.toml` is canonical. README, STATUS, GitHub Issues, and generated status are not canonical.
- Every new parser/check must reference a Feature ID from the matrix.
- Do not mark a matrix stage `done` without proof in `[feature.proof]`.
- No bare `TODO(spec)`. Use `TODO(spec: FEATURE-ID): ...`.
- No `unwrap`, `expect`, `panic!`, `todo!`, or `unimplemented!` in library code.
- Stubbed library features return typed `Error::Unimplemented { feature }`, `PayloadStatus::Unimplemented`, or a structured diagnostic.
- Keep `splot-cli` thin. Parser and validator logic belongs in library crates.
- All diagnostics must have stable rule IDs, severity, optional spec section, byte/bit offset, and message.
- Unsafe remains forbidden.

## 2. Implementation objective

Implement the next validator phase in PR-sized slices:

1. Complete shallow `sequence_header_obu()` child parsers.
2. Add or bound complex sequence children that need shared helpers/tables.
3. Strengthen §6.4 sequence-header semantic diagnostics.
4. Strengthen activated sequence state for §6.2.2 and §7.3.8.
5. Add temporal delimiter, MSDO, and multi-frame-header parser/state foundations.
6. Update inspector JSON, fixtures, matrix proof, and docs.

The final validator should be more capable, but still honest. If a section is not implemented, it must be visible in code, diagnostics/inspect output, and the matrix.

## 3. Code areas to inspect

Likely files:

```text
crates/splot-core/src/bitio.rs
crates/splot-core/src/obu.rs
crates/splot-core/src/annexb.rs
crates/splot-core/src/headers.rs
crates/splot-core/src/headers/sequence.rs
crates/splot-core/src/types.rs
crates/splot-validate/src/context.rs
crates/splot-validate/src/checks/mod.rs
crates/splot-validate/src/validator.rs
crates/splot-validate/src/diagnostic.rs
crates/splot-cli/src/commands/inspect.rs
crates/splot-cli/tests/cli.rs
xtask/src/feature_status.rs
```

Do not change crate dependency direction.

## 4. Sequence parser work

### 4.1 Implement shallow child parsers first

Implement these as typed structs and parser functions in the existing sequence module or a clearly named submodule:

```text
AV2-5.4.3-SEQUENCE-PARTITION-CONFIG
AV2-5.4.5-SEQUENCE-INTRA-CONFIG
AV2-5.4.7-SEQUENCE-SCC-CONFIG
AV2-5.4.12-TIMING-INFO
AV2-5.4.13-SEQUENCE-DECODER-MODEL-INFO
```

Requirements:

- parse fields exactly as AV2 syntax says;
- preserve inferred values explicitly;
- add doc comments with spec section and Feature ID;
- add positive tests, branch/inferred-value tests, and EOF tests;
- add validator diagnostics for local range rules.

### 4.2 Implement or bound complex children

Work through these next:

```text
AV2-5.4.2-SEQUENCE-TILE-CONFIG
AV2-5.4.4-SEQUENCE-SEGMENT-CONFIG
AV2-5.4.6-SEQUENCE-INTER-CONFIG
AV2-5.4.8-SEQUENCE-TQ-ENTROPY-CONFIG
AV2-5.4.9-SEGMENT-INFO
AV2-5.4.10-SEQUENCE-FILTER-CONFIG
AV2-5.4.11-USER-QM
```

If a parser requires a table/helper that is not implemented (`tile_params`, segmentation feature tables, transform/scan/QM tables), do one of these:

- implement the helper with tests and matrix proof, or
- stop at the exact feature boundary and return an unimplemented status/error with the owning Feature ID.

Do not silently skip bits and do not hand-transcribe large tables unless the matrix and docs explicitly say to.

### 4.3 Sequence-header payload boundary

After parsing implemented children, `open_bitstream_unit` must still know exactly how many bits were consumed so trailing bits and byte alignment are validated correctly. Add tests for payloads with malformed trailing bits after parsed sequence fields.

## 5. Validator state and semantics

Strengthen `ValidatorContext` and checks:

- store parsed sequence headers by `(xlayer, seq_header_id)` or an equivalent strong key;
- store a payload fingerprint for repeated activated-sequence checks;
- track active sequence by xlayer conservatively;
- enforce `obu_tlayer_id <= max_tlayer_id` and `obu_mlayer_id <= max_mlayer_id` when an active sequence is available;
- compare repeated activated sequence-header payloads for bit identity;
- detect monotonic output order mismatches across xlayer state when enough information exists;
- check timing consistency across embedded layers where parseable;
- keep frame/CLK-dependent activation limitations documented and bounded.

Add diagnostics from `docs/VALIDATOR-NEXT-DIAGNOSTICS.md`. Reuse existing IDs where they already exist.

## 6. HLS payload foundation

Implement:

```text
AV2-5.5-TEMPORAL-DELIMITER
AV2-5.6-MSDO
AV2-5.7-MULTI-FRAME-HEADER
```

Temporal delimiter:

- empty payload;
- state reset semantics needed by validator/ordering;
- reject non-empty payload unless it is only legal trailing bits according to the existing payload boundary model.

MSDO:

- parse fields;
- validate global/base layer ids;
- validate `num_streams_minus_2 <= 2`;
- store enough state for future multistream checks;
- add tests for malformed layer ids and too many streams.

Multi-frame header:

- parse `mfh_seq_header_id`, `mfh_id_minus_1`, optional frame size fields, and the initial update flags needed to preserve syntax boundaries;
- validate local id ranges where constants are modeled;
- store references to sequence header ids for HLS availability checks;
- do not implement full frame header reuse semantics yet.

## 7. Inspector JSON

Update `inspect --json` so parser progress is visible:

- show parsed sequence child sections where implemented;
- show `payload_status.status = "unimplemented"` and a `feature_id` for bounded unimplemented children;
- keep existing header fields stable;
- add CLI tests.

Do not make human output noisy unless tests are updated intentionally.

## 8. Tests and fixtures

Add direct unit tests first; add fixture files only when CLI/inspect tests need them.

Required tests:

```text
cargo test -p splot-core sequence
cargo test -p splot-validate sequence_header
cargo test -p splot-validate hls
cargo test -p splot-cli inspect
```

Specific cases:

- minimal still-picture sequence with inferred fields;
- non-still sequence with timing info;
- EOF after each sequence child group;
- zero timing values -> diagnostics;
- OBU temporal layer exceeds active sequence max -> diagnostic;
- OBU embedded layer exceeds active sequence max -> diagnostic;
- repeated identical sequence header -> OK;
- repeated non-identical sequence header -> diagnostic;
- duplicate temporal delimiter -> diagnostic;
- MSDO non-global layer id -> diagnostic;
- MSDO too many streams -> diagnostic.

## 9. Matrix and docs

Update `docs/IMPLEMENTATION-MATRIX.toml` only after code/tests exist. Then regenerate:

```bash
cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md
cargo xtask check-feature-status
cargo xtask spec-coverage
```

Update `STATUS.md` with:

- implemented items;
- still-stubbed items;
- deviations;
- exact command results.

Update OpenSpec tasks as completed.

## 10. Final acceptance

Run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo build --workspace --all-targets --locked
cargo test --workspace --all-targets --locked
cargo xtask ci
```

Also run:

```bash
cargo run -p splot-cli -- inspect tests/fixtures/conformant.av2 --json
cargo xtask feature-status --format table
cargo xtask spec-coverage
```

End your implementation report with:

- changed files;
- completed Feature IDs;
- Feature IDs still partial/todo and why;
- diagnostics added;
- exact commands and results;
- any spec uncertainties or intentionally bounded sections.
