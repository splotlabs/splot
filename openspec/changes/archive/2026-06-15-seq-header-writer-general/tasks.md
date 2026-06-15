# Tasks

## Implementation

- [x] Add `WriteError::NonCanonicalSequenceValue { what }` (`write/error.rs`).
- [x] Add `crates/splot-core/src/write/seq_header.rs`: `write_sequence_header_general`,
      `write_dependency_maps`, `write_cropping_window`, `write_sequence_decoder_model_info`,
      each with an up-front `check_*_encodable` validator — additive, no parser/model edits.
- [x] Register the module + re-export the four writers in `write/mod.rs`.

## Tests and proof

- [x] Semantic round-trip property test over parser-reachable models (all branches).
- [x] Byte-exact unit test on a parser-test fixture.
- [x] One rejection test per `WriteError` path, asserting `bit_len() == 0`.
- [x] A never-panics property test.

## Matrix and docs

- [x] Advance the `write` stage `todo -> done` on `AV2-5.4.1-SEQUENCE-HEADER-GENERAL`
      and `AV2-5.4.13-SEQUENCE-DECODER-MODEL-INFO`, with write proof recorded.
- [x] Set `AV2-5.4-SEQUENCE-HEADER` `write -> partial` with a writer note (configs pending).
- [x] Regenerate `docs/FEATURE-STATUS.md` from the matrix.

## Checks

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [x] `cargo test --workspace --all-targets --locked` + `cargo test --doc`
- [x] `RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps --locked`
- [x] `cargo xtask feature-status` + `cargo xtask check-feature-status`
- [x] `cargo xtask ci`
- [x] `openspec validate seq-header-writer-general --strict`
