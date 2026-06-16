## Why

Phase 9 requires fuzz coverage for every byte-consuming decoder stage. The
runtime hash byte API and pure Y4M writer are now fuzzed, but the existing
`DecodeContext::decode_y4m_bytes` path is still covered only by unit and CLI
tests.

## What Changes

- Add Feature ID `CONF-DECODE-RUNTIME-Y4M-FUZZ`.
- Add a cargo-fuzz target named `decode_runtime_y4m_bytes`.
- Feed arbitrary raw bytes and bounded mutations of the committed minimal IVF
  fixture into `DecodeContext::decode_y4m_bytes`.
- Use bounded in-memory writers, including a typed failing-writer mode, so the
  fuzz target exercises runtime Y4M success and output-error paths without
  touching the filesystem.
- Keep decode limits, input length, mutation count, and writer byte budgets
  finite for CI fuzz smoke.
- Update support/status docs, testing docs, implementation matrix, and decoder
  conformance coverage metadata.

## Capabilities

### New Capabilities

### Modified Capabilities

- `conformance`: Add no-panic fuzz coverage for the current minimal runtime
  Y4M byte-consuming API.
- `decoder-support`: Track runtime Y4M fuzz coverage as a scoped supported row
  without broadening runtime decode, CLI publication, or output conformance
  claims.

## Impact

- Affected code: `fuzz/Cargo.toml` and
  `fuzz/fuzz_targets/decode_runtime_y4m_bytes.rs`.
- Affected docs/status: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated status docs,
  `docs/TESTING.md`, and `xtask/src/decoder_conformance_coverage.rs`.
- Diagnostics: no new public diagnostic rule; fuzzing requires typed
  `DecodeError` returns for unsupported, malformed, resource-limit, or output
  failures.
- Dependencies: no new third-party dependency and no AVM/dav2d integration.
- Runtime behavior: no `splot decode` behavior change.
