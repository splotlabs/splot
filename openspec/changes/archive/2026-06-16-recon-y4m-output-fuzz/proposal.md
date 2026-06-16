## Why

Phase 9 requires fuzz coverage for output serialization. The repository has
source-backed Y4M writer tests for caller-supplied `DecodedFrame<T>` values, and
the runtime minimal Y4M path is covered by integration tests, but there is no
cargo-fuzz target that repeatedly drives the Y4M serialization surface with
bounded structured frame inputs.

## What Changes

- Add Feature ID `CONF-RECON-Y4M-OUTPUT-FUZZ`.
- Add a cargo-fuzz target named `recon_y4m_output_bytes`.
- Build small valid `splot-recon` decoded frames from arbitrary bytes across
  supported bit depths and pixel formats.
- Serialize one or more matching frames through `Y4mWriter`, optionally exercise
  stream/frame mismatch and failing-writer paths, and require typed return/no
  panic.
- Keep dimensions, frame counts, sample buffers, and output buffers bounded for
  CI fuzz smoke.
- Update support/status docs, testing docs, implementation matrix, and decoder
  conformance coverage metadata.

## Capabilities

### Modified Capabilities

- `conformance`: Add no-panic fuzz coverage for source-backed Y4M output
  serialization from bounded structured decoded-frame inputs.
- `decoder-support`: Track Y4M output serialization fuzz coverage as a scoped
  row without changing broad runtime decode or runtime output claims.

## Impact

- Affected code: `fuzz/Cargo.toml` and
  `fuzz/fuzz_targets/recon_y4m_output_bytes.rs`.
- Affected docs/status: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated status docs,
  `docs/TESTING.md`, and `xtask/src/decoder_conformance_coverage.rs`.
- Dependencies: add a direct path dependency from the out-of-workspace fuzz
  crate to `splot-recon`; no new third-party dependency.
- Runtime behavior: no `splot decode` behavior change.
- External tools: no AVM, dav2d, ffmpeg, filesystem output, network, or
  subprocess use.
