# Tasks

## Implementation

- [x] Add `crates/splot-core/src/write/error.rs`: self-contained `WriteError`
      (`BitWidthTooLarge`, `ByteWidthTooLarge`, `ZeroWidth`, `ValueTooWide`,
      `ValueOutOfRange`) + `WriteResult`.
- [x] Add `crates/splot-core/src/write/bit_writer.rs`: `BitWriter` inverting every
      `BitReader` primitive — `write_bit`/`write_bits`/`write_bits_u8` (`f(n)`),
      `write_su`, `write_uvlc`, `write_svlc`, `write_le`/`write_le_value`/
      `write_le_u64`, `write_leb128`, `write_ns`, `write_rg`, `align_to_byte`,
      `into_bytes`, plus `is_byte_aligned`/`bit_len` introspection.
- [x] Add `crates/splot-core/src/write/mod.rs` and register `pub mod write;` in
      `lib.rs` (read-only dependency on `bitio`; no parser/model/error edits).

## Tests and proof

- [x] Property tests for `read(write(x)) == x`: `f(n)`, `su`, `uvlc`, `svlc`,
      `le(n)->u64`, `leb128`, `ns`, `rg` across their valid value spaces.
- [x] Unit tests for every `WriteError` path and canonical byte output for
      `leb128`/`rg`/byte-alignment cases that mirror the reader tests.
- [x] A "writer never panics on any value/width" property test.

## Matrix and docs

- [x] Advance the `write` stage `todo -> done` on `AV2-4.11.3-UVLC`,
      `AV2-4.11.4-SVLC`, `AV2-4.11.5-LE`, `AV2-4.11.6-LEB128`, `AV2-4.11.7-SU`,
      `AV2-4.11.8-NS`, and `AV2-5.2.4-BYTE-ALIGNMENT`, with write proof recorded.
- [x] Create the `AV2-4.11.10-RG` row (parse + write `done`, proof recorded) so the
      `rg(n)` descriptor is tracked independently like its `§ 4.11` siblings.
- [x] Advance `ENC-BITSTREAM-WRITER` (`write` stub -> `partial`), point its module at
      `crates/splot-core/src/write/mod.rs`, and update its `openspec_change`/notes.
- [x] Regenerate `docs/FEATURE-STATUS.md` and `docs/SPEC-COVERAGE.md` from the matrix
      (the new `AV2-4.11.10-RG` row adds a `§ 4.11.10` coverage entry).
- [x] Lift the bitstream writer from behind the `docs/VALIDATOR-ROADMAP.md`
      "do not start yet" fence (maintainer-approved).
- [x] Add a writer-foundation note to `README.md`.

## Checks

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [x] `cargo test --workspace --all-targets --locked` + `cargo test --doc`
- [x] `RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps --locked`
- [x] `cargo xtask feature-status` + `cargo xtask check-feature-status`
- [x] `cargo xtask ci`
- [x] `openspec validate bit-writer-primitives --strict`
