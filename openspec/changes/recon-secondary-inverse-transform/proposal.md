## Why

The § 7.15 residual-math transform stack in `splot-recon` was complete except for
the § 7.15.3 secondary transform, which the roadmap had deferred as "entangled."
Its prerequisites have since landed: the § 9.7 IST kernels and `Stx_Scan_Map`
live in `splot-tables`, and `coefficient_scan_order` (`get_scan`) provides the 2D
scan. The remaining entanglement — deriving the `kernel` / `transpose` / `n` from
`YMode` / `pAngle` / `most_probable_stx_set` / `PlaneTxType` block state — is the
caller's job under the established `splot-recon` caller-resolves-spec-facts
contract, leaving a clean matrix-transform primitive.

## What Changes

- Add Feature ID `RECON-SECONDARY-INVERSE-TRANSFORM`.
- Add `crates/splot-recon/src/secondary_transform.rs` with
  `secondary_inverse_transform(dequant, params)` and the
  `SecondaryInverseTransform` params struct.
- Implement § 7.15.3: gather the first `n` coefficients of the `w * h` `Dequant`
  block in § 5.20.7.30 2D scan order (zeroing them), multiply by the § 9.7 IST
  kernel (`IST_4X4_KERNEL` / `IST_8X8_KERNEL` selected by `large = w >= 8 && h >=
  8`), apply § 4.8 `Round2Signed(t, 7)` and `Clip3(±(1 << (BitDepth + 7)))`, and
  scatter via `Stx_Scan_Order_4x4` / `Stx_Scan_Order_8x8` (hand-written
  spec-cited process-body constants) and `STX_SCAN_MAP`, honoring `transpose`.
- Take `w`, `h`, `n`, `kernel`, `sec_tx_type`, `transpose`, `bit_depth` as
  caller-resolved facts (the § 7.15.3 kernel/transpose/`n` block-state derivation
  stays with the caller, like the other § 7.15 transform primitives).
- Keep it total and panic-free (i64 accumulation, validated table indices, a
  fixed 32x32 scan scratch) with three new typed `ReconError` variants for
  shape / buffer / param rejection.
- Preserve the current runtime `splot decode` behavior and all output bytes (a
  `pub` primitive with no runtime rewiring).
- Add tests: a `Round2Signed` both-signs test, a hand-computed single-DC test
  against literal IST kernel values, small-4x4 and large-8x8 matches against an
  independent in-test re-trace, transpose, the reduced 8x8 height case,
  fail-atomic rejection, and an i32-extreme totality sweep.
- Update the implementation matrix, decoder support matrix, roadmap, generated
  status/coverage docs, the decoder-conformance-coverage group, and the crate
  `//!` docs.

Non-goals:

- No parsing of `sec_tx_type`, no kernel/transpose/`n` derivation, no frame or
  block state, no wiring into the runtime decode path, no dependency-graph
  change, and no AVM/dav2d invocation.

## Capabilities

### Modified Capabilities

- `decoder-support`: add a supported row for the § 7.15.3 secondary inverse
  transform, completing the § 7.15 residual-math transform stack.

## Impact

- Affected code: `crates/splot-recon/src/secondary_transform.rs`,
  `crates/splot-recon/src/error.rs`, `crates/splot-recon/src/lib.rs`,
  `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated status/coverage docs, and
  `xtask/src/decoder_conformance_coverage.rs`.
- Public API impact: one additive `pub fn` and `pub struct` in `splot-recon`,
  plus three additive error variants; no breaking changes.
- Diagnostics impact: none; existing minimal runtime diagnostics and output bytes
  remain unchanged.
- Dependencies and licensing: no new dependencies (reuses the existing
  `splot-recon → splot-tables` edge) and no licensing changes.
