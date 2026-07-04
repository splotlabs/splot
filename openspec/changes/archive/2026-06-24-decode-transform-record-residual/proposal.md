## Why

The live `local-decoder-mission.ivf` decode now reaches §5.20.7.27 coefficient parsing from
the active Wiener NS LR selectable transform-record path and fails on a residual
geometry/order mismatch before the next structured frontier can be trusted. This
change tackles that transform-record residual handoff as the next coherent
decoder slice instead of making another single-symbol PR.

Feature ID: `DECODE-SELECTABLE-TRANSFORM-RECORDS`.

## What Changes

- Diagnose and fix the local decoder mission Wiener NS LR transform-record residual handoff so
  luma/chroma coefficient parsing uses the transform size, scan, and CCTX
  ordering required by AV2 §5.20.7.24, §5.20.7.25, §5.20.7.27, and §5.20.7.30.
- Add focused regression coverage for the observed EOB/scan-size frontier and
  any corrected chroma/CCTX transform-block ordering.
- Advance the local decoder mission probe to the next structured unsupported frontier
  without producing decoded output.
- Keep reconstruction, loop-restoration filtering/output, reference refresh,
  AVM/dav2d byte equality, and successful local decoder mission decode unsupported.

## Capabilities

### New Capabilities

- `transform-record-residual`: Covers the syntax-only transform-record
  residual handoff needed by the live local decoder mission Wiener NS LR path after selectable
  transform records and CCTX metadata are present.

### Modified Capabilities

- `selectable-transform-records`: Extends the existing partial
  selectable-transform requirement to cover the live residual geometry/order
  subcase instead of stopping at record derivation.
- `decoder-support`: Records the new live local decoder mission frontier evidence and explicit
  non-goals for output/reconstruction.

## Impact

- Affected code: `crates/splot-decode/src/runtime_minimal/wienerns_lr/`,
  `crates/splot-decode/src/tile_payload/general_intra_residual.rs`, and focused
  transform-record/ordinary-coefficient tests.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated status docs, and OpenSpec
  decoder-support specs.
- No new dependencies, public APIs, encoder behavior, or license changes.
