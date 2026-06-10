# Tasks: OpenSpec hygiene

## 1. Pre-implementation bookkeeping

- [x] 1.1 Register the change in `openspec/changes/README.md` (Active changes
  table).

## 2. Fix the failing main specs

- [x] 2.1 `openspec/specs/bitstream/spec.md` requirement "New frame parsers
  never panic": reflow so SHALL is on the first body line; semantics
  unchanged.
- [x] 2.2 `openspec/specs/validator/spec.md` requirements "Frame QM reference
  diagnostics" and "Frame-header MFH layer-dependency checks": same reflow
  (the second also drops the stray blank line between title and body).
- [x] 2.3 `openspec validate --all --no-interactive` passes (12/12).

## 3. OpenSpec validation in the local gate

- [x] 3.1 `xtask/src/main.rs::run_ci`: add a run-if-present
  `openspec validate --all --no-interactive` step (probe the `openspec`
  binary; reuse `run_if_present`), placed with the other external-tool
  checks; matrix `XTASK-CI-QUALITY-GATES` notes record the completed parity.
- [x] 3.2 `AGENTS.md` § 4: document the command.

## 4. Park the encoder changes

- [x] 4.1 `openspec/changes/README.md`: state `parked (encoder track, behind
  the VALIDATOR-ROADMAP fence)` for `add-bitstream-writer` and
  `toy-intra-encoder-v0`.
- [x] 4.2 Add a one-line parked banner to both proposals pointing at
  `docs/VALIDATOR-ROADMAP.md` ("Do not start yet") and noting revival means
  re-proposing.
- [x] 4.3 Matrix `ENC-BITSTREAM-WRITER` / `ENC-INTRA-TOY-V0` notes record the
  parked state (no stage changes).

## 5. Generated artifacts and verification

- [x] 5.1 Regenerate `docs/FEATURE-STATUS.md` / `docs/SPEC-COVERAGE.md` if the
  matrix edit changes them; re-record the audit ledger.
- [x] 5.2 `cargo xtask ci` passes end to end with
  `RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin` (including the new OpenSpec
  step).
