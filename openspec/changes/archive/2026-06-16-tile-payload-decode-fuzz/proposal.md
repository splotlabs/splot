## Why

The current minimal runtime already reaches the tile-payload boundary for one
supported intra fixture, but cargo-fuzz coverage only reaches that path as part
of broad runtime hash/Y4M fuzzing. A targeted no-panic fuzz target will stress
tile-payload byte mutations through a `splot-decode` fuzzing-feature harness
while keeping the full `decode_tile()` and broad §5.20 traversal backlog honest.

Feature ID: `CONF-TILE-PAYLOAD-DECODE-FUZZ`.

## What Changes

- Add a self-contained cargo-fuzz target for bounded mutations of the committed
  minimal runtime fixture's tile-payload bytes.
- Drive a narrow `splot-decode` fuzzing-feature harness over the existing
  crate-private tile-payload boundary and minimal block-symbol frontier, using
  finite decode limits and no filesystem, subprocess, network, AVM, dav2d, or
  ffmpeg access.
- Assert only stable boundary/frontier invariants and accept typed decode errors
  for malformed or unsupported mutations.
- Register the target in the fuzz manifest, CI corpus seeding comments, testing
  docs, decoder support matrix, implementation matrix, and decoder conformance
  coverage row.
- Keep `tile-payload-decode`, `tile-cdf-selection-boundary`, and broad
  `symbol-decoder` support partial where full `decode_tile()`, §8.3 CDF
  selection, block syntax, reconstruction, and reference refresh remain future
  work.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `conformance`: add targeted no-panic fuzz coverage for the current
  tile-payload runtime byte boundary.
- `decoder-support`: record `CONF-TILE-PAYLOAD-DECODE-FUZZ` and the new fuzz
  target as self-contained support evidence without changing broad decoder
  status to supported.

## Impact

- Affected code: `fuzz/Cargo.toml`,
  `fuzz/fuzz_targets/tile_payload_decode_bytes.rs`,
  `crates/splot-decode/src/fuzzing.rs`, `.github/workflows/ci.yml`, and
  `xtask/src/decoder_conformance_coverage.rs`.
- Affected docs/status: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated status docs, `docs/TESTING.md`,
  and `AGENTS.md`.
- APIs/dependencies: adds a `splot-decode` `fuzzing` feature for the fuzz crate;
  no production API or dependency changes.
- Validator impact: none; this is decoder runtime fuzz coverage only.
- Diagnostics: no new decoder diagnostics; existing typed `decode/*` errors are
  accepted by the fuzz target.
- Non-goals: full AV2 §5.20 `decode_tile()`, recursive partition/block syntax,
  broad §8.3 CDF selection, all Tile/Saved CDF banks, reconstruction expansion,
  hash/Y4M behavior changes, reference refresh, AVM/dav2d/ffmpeg integration,
  file I/O, network I/O, subprocesses, scheduler changes, and new AV2
  semantics.
