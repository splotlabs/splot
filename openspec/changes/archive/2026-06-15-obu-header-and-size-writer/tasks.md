# Tasks

## Implementation

- [x] Add `WriteError::{EmptyTrailingBits, InconsistentHeader, NonInferableLayerIds,
      ObuTooLarge}` (`crates/splot-core/src/write/error.rs`).
- [x] Add `BitWriter::write_trailing_bits` (`crates/splot-core/src/write/bit_writer.rs`):
      the inverse of `crate::obu::parse_trailing_bits`.
- [x] Add `crates/splot-core/src/write/obu.rs`: `write_obu_header`,
      `write_obu_header_extension`, `write_annexb_obu`, and the `obu_total_len` helper —
      additive, no parser/model/error edits.
- [x] Wire `write/mod.rs` (`pub mod obu;` + re-exports).

## Tests and proof

- [x] Header semantic + byte-exact round-trip on the four canonical parser vectors.
- [x] Exhaustive header-byte sweep (no-extension type×tlayer; extension mlayer×xlayer).
- [x] `trailing_bits` round-trip (`write_trailing_bits` → `parse_trailing_bits`) + the
      `trailing_bits(0)` rejection.
- [x] Annex B framing byte-exact + reparse; non-canonical-size caveat test.
- [x] Error-path tests (`InconsistentHeader`, `NonInferableLayerIds`, `ObuTooLarge`).
- [x] Property tests (`roundtrip_obu_header`, `roundtrip_annexb_obu`, never-panics).

## Matrix and docs

- [x] Advance the `write` stage `todo -> done` on `AV2-5.2.2-OBU-HEADER` and
      `AV2-5.2.3-TRAILING-BITS`, with write proof recorded.
- [x] Update `ENC-BITSTREAM-WRITER` notes (OBU-header/framing/trailing-bits writers landed);
      keep `write = "partial"`.
- [x] Regenerate `docs/FEATURE-STATUS.md` from the matrix.
- [x] Note the OBU/Annex B writer in `README.md`.

## Checks

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [x] `cargo test --workspace --all-targets --locked` + `cargo test --doc`
- [x] `RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps --locked`
- [x] `cargo xtask feature-status` + `cargo xtask check-feature-status`
- [x] `cargo xtask ci`
- [x] `openspec validate obu-header-and-size-writer --strict`
