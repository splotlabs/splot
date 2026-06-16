# Testing

## Strategy (in priority order)

1. **Parser unit tests** — LEB128, AV2 OBU header, Annex B envelopes, and IVF
   container records, with positive, negative, and EOF cases. Implemented in each
   `splot-core` module.
2. **Property / fuzz tests** — the parsers and the validator must never panic on
   arbitrary input. Implemented as `*_never_panic(s)` tests across the
   `splot-core` parser modules and `crates/splot-validate/tests/validator_never_panics.rs`
   (mostly proptests, plus a few exhaustive-truncation unit tests). Thirteen
   `cargo fuzz` targets cover the parser, validator, symbol decoder,
   tile-payload frontier, byte-planner, and minimal runtime
   hash/Y4M byte surfaces, plus frame-hash serialization, reference-frame-store
   operations, Y4M output serialization from structured decoded frames, and
   intra prediction/workspace primitives from structured inputs, and need a
   nightly toolchain; they run as a blocking per-target smoke (~45s each) in PR
   CI:
   - `parse_obu` — `read_leb128`, `read_obu_header`, `parse_annex_b_obus`.
   - `parse_ivf` — `is_ivf`, `parse_ivf_header`, `parse_ivf_partial`.
   - `parse_bitstream` — `parse_bitstream_partial` (container auto-detect +
     Annex-B/IVF envelope parsing; OBU payload parsers are reached via
     `validate_bytes`, not this target).
   - `symbol_decoder_bytes` — public `splot-core` `SymbolDecoder` operations
     over bounded arbitrary tile-payload bytes plus bounded valid/invalid CDF
     rows.
   - `tile_payload_decode_bytes` — feature-gated `splot-decode` fuzzing
     harness over the current minimal tile-payload boundary and block-symbol
     frontier, using bounded arbitrary tile-payload bytes and bounded known-good
     payload mutations.
   - `validate_bytes` — `Validator::validate_bytes_with_options` (the
     highest-coverage target: transitively reaches every OBU payload parser, both
     container formats, and every validator check).
   - `decode_plan_bytes` — `DecodeContext::plan_bytes` with finite limits
     (bounded plan-only traversal over arbitrary raw Annex B or IVF/DKIF bytes).
   - `decode_runtime_hash_bytes` —
     `DecodeContext::decode_hash_report_bytes` with finite limits over arbitrary
     bytes and bounded mutations of the committed minimal runtime IVF fixture.
   - `decode_runtime_y4m_bytes` —
     `DecodeContext::decode_y4m_bytes` with finite limits over arbitrary bytes,
     bounded mutations of the committed minimal runtime IVF fixture, and bounded
     in-memory writer success/error paths.
   - `recon_frame_hash_bytes` — `splot-recon` `DecodedFrameHashInput`
     serialization and digest computation from bounded structured
     `DecodedFrame` inputs.
   - `recon_reference_frame_store_bytes` — `splot-recon` `ReferenceSlot` and
     `ReferenceFrameStore` storage operations from bounded state-machine
     inputs.
   - `recon_y4m_output_bytes` — `splot-recon` `Y4mWriter` serialization from
     bounded structured `DecodedFrame` inputs across supported Y4M formats.
   - `recon_intra_prediction_bytes` — `splot-recon` intra prediction and
     current-frame workspace primitives from bounded structured inputs.
3. **Decode planner unit tests** — `splot-decode` plan-only APIs over already
   parsed `splot-core` stream output must preserve OBU order/source metadata,
   reject malformed sources transactionally, enforce the limits they can derive
   from parsed or raw byte input, and prove deterministic plan metadata across
   decode thread-count policies. The raw byte planner is covered by the
   `decode_plan_bytes` fuzz target; the current minimal runtime hash byte API is
   covered by `decode_runtime_hash_bytes`, and the current minimal runtime Y4M
   byte API is covered by `decode_runtime_y4m_bytes`.
4. **CLI integration tests** — `crates/splot-cli/tests/cli.rs` runs the `splot`
   binary against the fixtures in `tests/fixtures/` and generated temporary IVF
   inputs (exit codes, `--json`, `inspect`). Implemented; snapshot tests for
   `inspect` output are planned (`insta`).
5. **Conformance vectors** — from AOMedia. Planned, once vectors are available
   (see [CONFORMANCE.md](./CONFORMANCE.md)).
6. **Differential testing against AVM** — the reference software is the oracle.
   Planned (directions and harness plan in [CONFORMANCE.md](./CONFORMANCE.md)).

## Commands

```bash
cargo test --workspace --all-targets --locked   # unit, property, and CLI integration tests (no doctests)
cargo test --doc --workspace --locked           # doctests (not covered by --all-targets)
cargo xtask ci
cargo xtask coverage            # local HTML coverage report (cargo-llvm-cov, run-if-present)
cargo xtask check-decoder-support # generated decoder support docs drift gate

# Fuzzing needs a NIGHTLY toolchain (cargo-fuzz uses AddressSanitizer + coverage,
# which are nightly-only). On stable, the per-module `*_never_panic(s)` tests and
# the splot-validate `validator_never_panics` proptest exercise the same
# never-panic invariant with bounded random inputs.
cargo xtask fuzz [--time <secs>]    # local fuzz smoke over every target (nightly + cargo-fuzz, run-if-present), default 30s each
cargo install cargo-fuzz --locked
cargo +nightly fuzz list            # parse_obu, parse_ivf, parse_bitstream, symbol_decoder_bytes, tile_payload_decode_bytes, validate_bytes, decode_plan_bytes, decode_runtime_hash_bytes, decode_runtime_y4m_bytes, recon_frame_hash_bytes, recon_reference_frame_store_bytes, recon_y4m_output_bytes, recon_intra_prediction_bytes
cargo +nightly fuzz run parse_obu   # run a single target (swap the name for any target above)

cargo xtask conformance         # run the committed conformance corpus (no AVM)
```

## Conventions

- Every parser change adds the relevant positive/negative/EOF cases.
- Tests may use `unwrap`/`expect` only inside `#[cfg(test)]` modules annotated with
  `#[allow(clippy::unwrap_used, clippy::expect_used)]`; production library code must
  not.
- **Record proof in the matrix.** When a feature's stage becomes `done`, record the
  test module/path, the reproducible command, the fixture/vector, and/or the
  diagnostic id in that row's `[feature.proof]` in
  [IMPLEMENTATION-MATRIX.toml](./IMPLEMENTATION-MATRIX.toml). `cargo xtask
  check-feature-status` rejects a `done` code stage with no proof; `cargo xtask
  spec-coverage` lists rows still missing proof.
