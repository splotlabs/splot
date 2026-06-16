## Why

Phase 9 needs fuzz coverage for source-backed decoded-frame hash output. The
repository already has unit tests for `DecodedFrameHashInput`, and the minimal
runtime hash path is covered by integration and byte-level fuzzing, but there is
no cargo-fuzz target that repeatedly drives the frame-hash serialization and
digest surface with bounded structured `DecodedFrame<T>` inputs.

## What Changes

- Add Feature ID `CONF-RECON-FRAME-HASH-FUZZ`.
- Add a cargo-fuzz target named `recon_frame_hash_bytes`.
- Build small valid `splot-recon` decoded frames from arbitrary bytes across
  supported bit depths, sample storage types, pixel formats, crop origins,
  padding, and strides.
- Exercise `DecodedFrameHashInput::byte_len`, `write_to`, and `compute_hash`
  without parsing AV2 bitstreams or invoking runtime decode.
- Assert stable hash-contract identifiers, deterministic emitted bytes and
  digest values, visible-region isolation from non-visible padding, and typed
  writer-error propagation.
- Update support/status docs, testing docs, implementation matrix, and decoder
  conformance coverage metadata.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `conformance`: Add no-panic fuzz coverage for source-backed decoded-frame
  hash byte serialization and digest computation from bounded structured
  decoded-frame inputs.
- `decoder-support`: Track decoded-frame hash fuzz coverage as a scoped row
  without changing broad runtime decode, metadata hash verification, output
  ordering, film-grain, or reference-update claims.

## Impact

- Affected code: `fuzz/Cargo.toml` and
  `fuzz/fuzz_targets/recon_frame_hash_bytes.rs`.
- Affected docs/status: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated status docs,
  `docs/TESTING.md`, `AGENTS.md`, `.github/workflows/ci.yml`, and
  `xtask/src/decoder_conformance_coverage.rs`.
- Dependencies: no new third-party dependency and no new `splot-*` dependency
  edge.
- Runtime behavior: no `splot decode` behavior change.
- External tools: no AVM, dav2d, ffmpeg, filesystem output, network, or
  subprocess use.
