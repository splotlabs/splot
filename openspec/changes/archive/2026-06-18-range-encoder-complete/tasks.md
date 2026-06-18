## 1. OpenSpec And Flight Control

- [x] 1.1 Author `openspec/changes/range-encoder-complete/` proposal, design, tasks, `symbol-encoder` spec, and `encoder-tools` delta.
- [x] 1.2 Run `openspec validate range-encoder-complete --strict`.
- [x] 1.3 Re-check open PRs before implementation and before PR publication; do not open while PR #244 or another sibling owns shared generated matrix/status docs.

## 2. Symbol Encoder API And Errors

- [x] 2.1 Add a public `splot-core` symbol encoder module/API with configuration for CDF update mode, maximum output bytes, and maximum primitive operations.
- [x] 2.2 Share CDF validation/update helpers with `SymbolDecoder` without changing decoder behavior.
- [x] 2.3 Add writer-side typed errors for invalid symbol rows, symbol/literal domain errors, output-limit exhaustion, and operation-limit exhaustion.
- [x] 2.4 Retire, deprecate, or document the stale `bitio::RangeEncoder` unimplemented stub without disturbing the working `SymbolDecoder` API.

## 3. Arithmetic Encoder Implementation

- [x] 3.1 Implement deterministic range-state updates for `write_bool`, inverse to AV2 § 8.2.3.
- [x] 3.2 Implement `write_literal(n, value)` as MSB-first `write_bool` composition, inverse to AV2 § 8.2.5.
- [x] 3.3 Implement `write_symbol(cdf, symbol)` using the AV2 § 8.2.6 interval calculation, CDF validation, CDF adaptation, and checked output buffering.
- [x] 3.4 Implement consuming finalization that emits `exit_symbol()`-valid trailing/padding bits and returns owned payload bytes plus summary metadata.

## 4. Tests And Fuzzing

- [x] 4.1 Add unit tests for boolean/literal round trips, symbol round trips across arities `N = 2..=8`, CDF update parity, disabled-update parity, finalization padding, and deterministic bytes.
- [x] 4.2 Add negative tests proving invalid CDF rows, out-of-range symbols, too-wide literals, output-limit exhaustion, and operation-limit exhaustion fail before mutation.
- [x] 4.3 Add property tests over bounded random valid operation streams and CDF rows: encode -> decode recovers values and CDF rows.
- [x] 4.4 Add `fuzz/fuzz_targets/symbol_encoder_bytes.rs`, register it, and make it assert no panic plus decode-roundtrip invariants for bounded operation streams.
- [x] 4.5 Run targeted tests: `cargo test -p splot-core symbol --locked`, `cargo test -p splot-core symbol_encoder --locked`, and `cargo check --manifest-path fuzz/Cargo.toml --bins --locked`.

## 5. Proof, Docs, And Generated Status

- [x] 5.1 Update `docs/IMPLEMENTATION-MATRIX.toml` for `ENC-BITSTREAM-WRITER` with scoped § 8.2 symbol-encoder proof and exclusions.
- [x] 5.2 Update `docs/ENCODER-ROADMAP.md`, `docs/ENCODER-GAP-AUDIT.md`, and writer/spec coverage docs as applicable without claiming coded tile body support.
- [x] 5.3 Regenerate affected generated docs (`cargo xtask feature-status`, `cargo xtask spec-coverage`, writer coverage/status commands if required by changed rows).
- [x] 5.4 Run `cargo xtask check-feature-status`, `cargo xtask check-fuzz-targets`, `openspec validate --all --no-interactive`, and `cargo xtask ci`.

## 6. Review, Archive, And PR

- [x] 6.1 Run local independent reviews: correctness/spec, entropy/parser-roundtrip, security/zero-copy, determinism/concurrency, and test/evidence.
- [x] 6.2 Archive the OpenSpec change with `openspec archive range-encoder-complete --yes`, then rerun OpenSpec validation and `cargo xtask ci`.
- [x] 6.3 Recompute the Flight Manifest after syncing with merged `main`; confirm no open sibling PR owns overlapping files.
- [ ] 6.4 Open a ready PR with Feature IDs, AV2 sections, flight manifest, tests, fuzz coverage, reviewer decisions, exclusions, and final review checklist.
