# Tasks

## Round-trip module (additive — no model change)
- [x] `write/roundtrip.rs`: `RoundtripOutcome` enum (`RoundTripped` / `Unwritable { feature }` /
      `Failed { reason }`); `recover_roundtrip_passthrough(payload, parsed)` (real bytes for padding,
      modeled-length zero-fill for the metadata blobs, empty otherwise; allocations bounded by
      `payload.len()`); `roundtrip_obu(header, payload, parsed)` (recover → `write_complete_obu` →
      Annex B size prefix → reparse → compare; never panics). Re-export in `write/mod.rs`; module
      `//!` doc cites the round-trip contract and the semantic-vs-byte-exact distinction.

## Fuzz target
- [x] `fuzz/fuzz_targets/roundtrip_obu_bytes.rs`: partial-parse arbitrary bytes; for every OBU whose
      `payload_status()` is `Parsed`, assert `roundtrip_obu` is `RoundTripped` or `Unwritable`.
- [x] Register `[[bin]] name = "roundtrip_obu_bytes"` in `fuzz/Cargo.toml`; update the `AGENTS.md`
      fuzz-target list.

## Tests and proof
- [x] `roundtrip.rs` unit tests: fixture round-trip → `RoundTripped` for each written type
      (temporal delimiter, sequence header, padding with a real run, metadata short with a blob,
      multi-unit metadata group, global-xlayer metadata group); `Unwritable` for an unwritten type;
      `recover_roundtrip_passthrough` edge cases (padding run, metadata zero-fill length, an
      over-large constructed length rejects rather than allocating).

## Matrix and docs
- [x] Add a WRITER note + proof to `ENC-BITSTREAM-WRITER` recording the round-trip harness + the new
      fuzz target. Regenerate `docs/FEATURE-STATUS.md`.

## Checks
- [x] `cargo xtask ci` and `openspec validate writer-roundtrip-harness --strict`
