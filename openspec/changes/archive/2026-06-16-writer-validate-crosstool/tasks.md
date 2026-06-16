# Tasks

## Cross-tool test (test-only — no production change)
- [x] `crates/splot-validate/tests/writer_roundtrip_conformance.rs`: for each conformant
      writable-type fixture (`padding.av2`, `metadata-short.av2`, `metadata-group.av2`), parse →
      assert each OBU `roundtrip_obu` is `RoundTripped` → re-emit the Annex B stream via
      `write_complete_obu` + `leb128` size prefix → assert byte-exact to the original AND
      `Validator::validate_bytes(re-emission).is_conformant()` (zero error diagnostics). File-level
      `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]` like the sibling
      integration tests. Resolve the fixtures dir via `CARGO_MANIFEST_DIR/../../tests/fixtures`.
- [x] Sanity-assert the original fixtures are themselves conformant (guarding against a stale fixture)
      and that a re-emission of a deliberately corrupted stream is NOT silently accepted.

## Matrix and docs
- [x] Record the cross-tool test on `ENC-BITSTREAM-WRITER` (note + proof). Regenerate
      `docs/FEATURE-STATUS.md`.

## Checks
- [x] `cargo xtask ci` and `openspec validate writer-validate-crosstool --strict`
