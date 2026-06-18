## Context

The previous decoder bricks derived every §8.3.2 coefficient-symbol context in
`cdf/coeff_context.rs`, but the functions still take caller-provided
`Level[]`, `QuantSign[]`, `AboveDcContext[]`, and `LeftDcContext[]` slices. The
future §5.20.7.27 `coeffs()` loop needs decode-owned state for those buffers:
per-transform-block local coefficient magnitudes/signs and tile-neighbour level
/ DC-context lines.

The current minimal runtime does not execute a real coefficient loop, so this
change must remain loaded-but-unwired and preserve the existing flat-intra output
identity.

## Goals / Non-Goals

**Goals:**

- Add crate-private state for the transform-block-local `Level[]` and
  `QuantSign[]` arrays initialized as §5.20.7.27 describes.
- Add crate-private tile-neighbour context lines for
  `AboveLevelContext`, `LeftLevelContext`, `AboveDcContext`, and
  `LeftDcContext`.
- Provide checked update helpers for the §5.20.7.27 end-of-`coeffs()` writes and
  the §5.20 block-context reset writes.
- Keep the state reusable by future CDF-context and `coeffs()` loop work without
  importing `splot-recon` or changing crate dependencies.

**Non-Goals:**

- No symbol decoding, no `Quant[]` production, no `read_quant`, no dequant,
  inverse transform, residual add, reconstruction, output expansion, reference
  refresh, public API, or AVM/dav2d invocation.
- No claim that broad `decode_tile()` or broad `decode_block()` syntax is
  supported.

## Decisions

- **Separate `coeff_state.rs` module.** The existing CDF and partition modules are
  already close to or above the source-line budget. The new state is not CDF row
  storage, so it gets a focused tile-payload module beside `mi_size_state.rs`.

- **Three-plane neighbour lines, local block arrays.** The neighbour state owns
  three above lines sized by tile MI columns and three left lines sized by tile MI
  rows. The transform-block state owns row-major local `level` (`u32`) and
  `quant_sign` (`i32`) arrays sized by caller-resolved adjusted transform
  dimensions, capped at 32x32 as in §5.20.7.27.

- **Checked APIs, zero out-of-range reads.** State construction rejects zero or
  overflowing dimensions and allocation failures with typed errors. Update/reset
  helpers validate plane and coordinate ranges. Read views are slices so existing
  context functions keep their own total, out-of-range-as-zero behavior.

- **No dependency or scheduler change.** The state is decode-owned and scalar. It
  does not import `splot-recon`, use Rayon, or affect the existing
  `splot_parallel::WorkerPool` handoff.

## Risks / Trade-offs

- **Coordinate-model mismatch** -> Tests pin the §5.20.7.27 end-of-`coeffs()`
  above/left writes and the §5.20 reset ranges independently for luma and chroma
  style coordinates.
- **Pathological caller geometry** -> Constructors and range helpers use checked
  arithmetic and break/return on real buffer bounds rather than spinning on
  caller-provided counts.
- **Premature support claim** -> The new feature and support rows stay partial.
  The OpenSpec, matrix notes, and roadmap keep the coefficient loop and decode
  output changes out of scope.
