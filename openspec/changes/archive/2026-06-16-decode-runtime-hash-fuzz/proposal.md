## Why

The current decode fuzz target stops at `DecodeContext::plan_bytes`, so arbitrary
byte fuzzing does not reach the minimal runtime hash path that now performs tile
frontier, CDF lifecycle, reconstruction, and hash-report work. Phase 3 should
extend no-panic fuzz coverage to `DecodeContext::decode_hash_report_bytes`
before broadening tile syntax further.

## What Changes

- Add Feature ID `CONF-DECODE-RUNTIME-HASH-FUZZ`.
- Add a self-contained cargo-fuzz target `decode_runtime_hash_bytes`.
- Feed arbitrary raw bytes and bounded mutations of the committed minimal IVF
  fixture into `DecodeContext::decode_hash_report_bytes`.
- Keep fuzz limits finite and small enough for CI fuzz-smoke.
- Assert only typed success structure or typed error return; do not write files,
  invoke AVM/dav2d, or add external fixtures.
- Update decoder support/status, testing docs, implementation matrix, and decoder
  conformance coverage fuzz-target lists.

## Capabilities

### New Capabilities

- `conformance`: Self-contained no-panic fuzz entry point for the minimal
  runtime hash byte-consuming API.

### Modified Capabilities

- `decoder-support`: Track runtime hash fuzz coverage as a distinct supported
  decoder support row without changing broad decode support claims.

## Impact

- Affected code: `fuzz/Cargo.toml` and
  `fuzz/fuzz_targets/decode_runtime_hash_bytes.rs`.
- Affected docs/status: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated status docs,
  `docs/TESTING.md`, and `xtask/src/decoder_conformance_coverage.rs`.
- Diagnostics: no new public diagnostic rule; fuzzing requires typed
  `DecodeError` returns on failure.
- Dependencies: no new third-party dependencies and no AVM/dav2d integration.
