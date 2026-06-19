## Why

The § 7.15.4 outer 2D inverse transform applies a DPCM cumulative sum (the
`InverseTransform2dOuter.dpcm` field) but leaves the direction selection to the
caller. The other § 7.15.4 parameter derivations — `Transform_Shift`,
`get_transform_1d_type`, and the combined `resolve` helper — are in place; the
DPCM-direction selection is the last one. With the residual-chain sink
(`reconstruct_transform_block_residual`) now consuming a fully-resolved
`InverseTransform2dOuter`, deriving its `dpcm` field is the natural completion of
the § 7.15.4 parameter-derivation surface.

## What Changes

- Add Feature ID `RECON-DPCM-DIRECTION`.
- Add `dpcm_direction(use_dpcm: bool, mode_is_v_pred: bool) -> Option<DpcmDirection>`
  to `crates/splot-recon/src/transform_params.rs`.
- Implement § 7.15.4 `useDpcm = (plane == 0 ? use_dpcm_y : use_dpcm_uv)` and
  `mode = (plane == 0 ? YMode : UVMode)`: `None` when `use_dpcm` is false,
  `Vertical` (down-column cumulative sum) for `V_PRED`, `Horizontal`
  (across-row) otherwise — the direction `inverse_transform_2d_outer` already
  applies.
- Take `use_dpcm` and `mode_is_v_pred` as caller-resolved scalars so `splot-recon`
  holds no frame state or prediction-mode enum (the crate-wide
  caller-resolves-spec-facts contract).
- Keep it a total `const fn` with no error path.
- Preserve the current runtime `splot decode` behavior and all output bytes (a
  pure `const fn` with no runtime rewiring).
- Add tests: the four-spec-case mapping (three pinned at compile time) and an
  integration test driving the § 7.15.4 outer transform through a lossless IDTX
  block so the selected `Vertical` direction produces the per-column cumulative
  sum while `None` leaves the residual flat.
- Update the implementation matrix, decoder support matrix, roadmap, generated
  status/coverage docs, the decoder-conformance-coverage group, and the crate
  `//!` docs (and `transform_params.rs`'s module doc, which listed this as a
  future row).

Non-goals:

- No runtime resolution of `use_dpcm_y` / `use_dpcm_uv` / the prediction mode
  (caller-resolved from frame and block syntax), no wiring into the runtime
  decode path, no § 7.15.3 secondary transform, no coefficient entropy decode,
  no dependency-graph change, and no AVM/dav2d invocation.

## Capabilities

### Modified Capabilities

- `decoder-support`: add a supported row for the § 7.15.4 DPCM-direction
  selection, completing the § 7.15.4 inverse-transform parameter-derivation
  surface.

## Impact

- Affected code: `crates/splot-recon/src/transform_params.rs`,
  `crates/splot-recon/src/lib.rs`, `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/DECODER-ROADMAP.md`, generated
  status/coverage docs, and `xtask/src/decoder_conformance_coverage.rs`.
- Public API impact: one additive `const fn` in `splot-recon`; no breaking
  changes.
- Diagnostics impact: none; existing minimal runtime diagnostics and output bytes
  remain unchanged.
- Dependencies and licensing: no new dependencies and no licensing changes.
