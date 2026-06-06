# Tasks: validator coverage roadmap

`change-id: validator-coverage-roadmap`

## Documentation and tracking

- [x] Copy `docs/VALIDATOR-GAP-ANALYSIS.md` into the repo.
- [x] Copy `docs/VALIDATOR-ROADMAP.md` into the repo.
- [x] Copy `docs/VALIDATOR-IMPLEMENTATION-MATRIX-EXPANSION.md` into the repo.
- [x] Copy `docs/VALIDATOR-DIAGNOSTICS.md` into the repo.
- [x] Link the new docs from `docs/SPEC-MAPPING.md` and `docs/FEATURE-TRACKING.md`.
- [x] Add this OpenSpec change under `openspec/changes/validator-coverage-roadmap/`.
- [x] Update `openspec/changes/README.md` to list `validator-coverage-roadmap`.
- [x] Regenerate `docs/FEATURE-STATUS.md`.

## Matrix expansion

- [x] Add descriptor rows: `AV2-4.11.3-UVLC`, `AV2-4.11.5-LE`, `AV2-4.11.8-NS`.
- [x] Revise existing `AV2-5.2.3-TRAILING-BITS` and `AV2-5.2.4-BYTE-ALIGNMENT` rows.
- [x] Add `AV2-5.2.1-OBU-DISPATCH`.
- [x] Split `AV2-5.4-SEQUENCE-HEADER` into §5.4 child rows.
- [x] Add `AV2-6.4-SEQUENCE-HEADER-SEMANTICS`.
- [x] Add `AV2-6.2.2-OBU-HEADER-ACTIVATED-SEQUENCE-LIMITS`.
- [x] Split `AV2-7.3-OBU-ORDERING` into child rows before implementing ordering.
- [x] Add missing top-level OBU rows for §5.5-§5.17.
- [x] Split frame header, metadata, and tile group rows before coding them.
- [x] Split LCR and OPS rows before coding them.

## Phase 1 implementation: descriptors and payload boundaries

- [ ] Implement `uvlc()` in `splot-core` with EOF and bound tests.
- [ ] Implement `le(n)` if needed by first payload syntax.
- [ ] Implement `ns(n)` with power-of-two and non-power-of-two tests.
- [ ] Implement `trailing_bits(nbBits)` parser.
- [ ] Implement `byte_alignment()` parser/check.
- [ ] Add proptests/fuzz coverage for new bitreader paths.
- [ ] Add diagnostics for invalid trailing bits and alignment bits.
- [ ] Update matrix proof.

## Phase 2 implementation: payload dispatch

- [ ] Add `ParsedObu` / payload status type.
- [ ] Dispatch OBU payloads according to `obu_type`.
- [ ] Keep unimplemented payloads explicit and honest.
- [ ] Update `inspect` JSON to include payload status.
- [ ] Add tests preserving existing header-only behavior.
- [ ] Update matrix proof.

## Phase 3 implementation: sequence header

- [ ] Add sequence-header strong types.
- [ ] Add `crates/splot-core/src/headers/sequence.rs` or equivalent.
- [ ] Parse §5.4.1 general sequence header syntax.
- [ ] Add child parser stubs for §5.4.2-§5.4.13 with `TODO(spec: FEATURE-ID)` markers.
- [ ] Validate local §6.4.1 conformance rules that are decidable from the parsed header.
- [ ] Add positive/negative/EOF tests.
- [ ] Add small sequence-header fixtures or test builders.
- [ ] Update matrix proof.

## Phase 4 implementation: stateful validator

- [ ] Add `ValidatorContext` without breaking the public `Validator` API.
- [ ] Store parsed sequence headers by `seq_header_id` and layer context.
- [ ] Implement activated sequence state enough to enforce remaining §6.2.2 layer-limit checks.
- [ ] Add diagnostics for unknown/missing active sequence header and layer id exceeding sequence maximum.
- [ ] Add tests with sequence header followed by valid/invalid OBUs.
- [ ] Update matrix proof.

## Phase 5 implementation: OBU ordering

- [ ] Add temporal-unit state machine.
- [ ] Validate temporal delimiter presence/order.
- [ ] Validate global HLS prefix ordering.
- [ ] Validate ascending coded extended layer unit `obu_xlayer_id` order.
- [ ] Validate padding exceptions.
- [ ] Add tests for valid and invalid ordering.
- [ ] Update matrix proof.

## Later phases

- [ ] Implement MSDO parser/checks.
- [ ] Implement LCR parser/checks.
- [ ] Implement OPS parser/checks.
- [ ] Implement atlas parser/checks.
- [ ] Implement buffer removal timing parser/checks.
- [ ] Implement metadata parser/checks.
- [ ] Implement padding parser/checks.
- [ ] Implement film grain, QM, and content interpretation parser/checks.
- [ ] Split and implement frame header child features.
- [ ] Split and implement tile group child features.
- [ ] Add Annex A profile/level/tier checks.
- [ ] Add Annex E decoder-model checks.
- [ ] Add AVM differential harness proof.

## Final acceptance

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [ ] `cargo build --workspace --all-targets --locked`
- [ ] `cargo test --workspace --all-targets --locked`
- [ ] `cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md`
- [ ] `cargo xtask check-feature-status`
- [ ] `cargo xtask spec-coverage`
- [ ] `cargo xtask ci`
