## Why

The current supported minimal runtime validates the traced 64x64 flat intra tile
syntax but still constructs the output frame with a synthetic all-128 helper.
The next decoder-conformance step is to make that same narrow tier produce its
frame through reconstruction-owned primitives, so hash and Y4M success are backed
by a real runtime reconstruction handoff without broadening the supported AV2
surface.

## What Changes

- Add Feature ID `DECODE-MINIMAL-INTRA-RECONSTRUCTION-FRONTIER` to track the
  narrow minimal reconstruction handoff.
- Replace the minimal runtime's direct filled-plane YUV420 frame construction
  with a crate-private reconstruction adapter that uses existing `splot-recon`
  workspace primitives, including luma DC intra prediction, for the committed
  64x64 all-flat minimal fixture.
- Preserve the existing minimal hash and Y4M byte output exactly.
- Keep all out-of-tier streams failing closed with `decode/unsupported-feature`.
- Do not claim broad `decode_block()`, full intra prediction, residual/transform
  reconstruction, loop filtering, reference refresh, raw output, or film-grain
  support.

## Capabilities

### New Capabilities
- `minimal-intra-reconstruction-frontier`: Covers the narrow runtime handoff that
  constructs the existing minimal flat intra fixture through checked
  reconstruction workspace primitives while preserving current hash/Y4M identity
  and unsupported-feature boundaries.

### Modified Capabilities
- None.

## Impact

- Affected code: `crates/splot-decode/src/runtime_minimal.rs` and, if needed,
  a focused crate-private helper module under `crates/splot-decode/src/`.
- Affected docs/matrix: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated status/coverage docs, and
  `docs/DECODER-ROADMAP.md`.
- Affected tests: minimal runtime hash/Y4M tests plus focused reconstruction
  handoff tests proving byte-identical output and closed failure paths.
- No public API, CLI flag, dependency graph, license, AVM/dav2d integration, or
  validator behavior changes.
