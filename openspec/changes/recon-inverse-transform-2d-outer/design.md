## Context

The § 7.15.4.1 2D matrix transform core (`inverse_transform_2d`) is merged. The
§ 7.15.4 outer process drives it: it derives the adjusted/original sizes, takes
the lossless IDTX bit-shift shortcut where applicable, applies the DPCM
cumulative sum, and expands the adjusted block to the original size by sample
duplication. It is a thin orchestration layer with no new table or crate
dependency.

## Goals / Non-Goals

Goals:

- Implement the § 7.15.4 outer process exactly, layered on `inverse_transform_2d`.
- Keep it total, panic-free, and free of frame/segment/tile state.

Non-Goals:

- The § 7.15.4 `Transform_Shift` and `get_transform_1d_type` derivations (the
  caller resolves `rowType` / `colType` / shifts, as the § 7.15.4.1 core already
  requires), the § 7.15.3 secondary transform, the § 7.14.4 dequantization
  process, residual addition, or workspace integration.

## Decisions

- **Original log2 dims in, sizes derived; no tables.** `inverse_transform_2d`
  already takes the original `txSz` log2 dims and derives the adjusted size as
  `1 << Min(log2, 5)`. The outer process needs the original size too (for sample
  duplication: `Tx_Width[txSz] = 1 << log2W`), which is the same input. So the
  outer process derives both `adjW/adjH` and `w/h` from the original log2 dims —
  no `Adjusted_Tx_Size` / `Tx_Width` / `Tx_Height` conversion table, and no
  `splot-core` dependency (which `splot-recon` is forbidden anyway).
- **Adjusted scratch, original output.** The matrix transform (and the DPCM sum)
  operate on the adjusted `adjW * adjH` block, which is then expanded into the
  caller's original `w * h` residual by sample duplication. Using a fixed
  32x32 stack scratch for the adjusted block and writing the expansion into the
  caller buffer avoids the spec's in-place stride gymnastics (it duplicates
  within a single array reinterpreted at the wider stride); the direct expansion
  `residual[oi][oj] = scratch[oi / hFactor][oj / wFactor]` (factors 1 or 2) is
  provably equal to the spec's two-step width-then-height duplication.
- **Lossless IDTX shortcut.** When `lossless && plane_tx_type_is_idtx`, the
  process bypasses the matrix transform: `Residual = Dequant >> (3 - shift)` with
  `shift = (pels > 256) + (pels > 1024)` (pels = adjusted `w * h`). Lossless
  blocks are 4x4 (validated), so `shift` is 0 and the shift is `>> 3`, but the
  general formula is kept for fidelity. `shift` is `0..=2`, so `3 - shift` is
  `1..=3` (always a valid right shift).
- **DPCM via `wrapping_add`.** The cumulative sum is a plain integer sum in the
  spec. Conformant residuals are `Clip3`-bounded by the transform and never
  overflow, but `wrapping_add` makes the primitive unconditionally total
  (panic-free even for adversarial `i32` extremes) without diverging from the
  spec on reachable inputs. `DpcmDirection::Vertical` (for `V_PRED`) sums down
  columns; `Horizontal` sums across rows.
- **Caller-resolved transform selection.** `rowType` / `colType` / shifts stay
  caller-supplied (the `get_transform_1d_type` derivation pulls in `PlaneTxType`,
  the `Transform_1d_Type` table, and the inter-DDT flags — block/frame state out
  of scope), consistent with the § 7.15.4.1 core's API.

## Risks / Trade-offs

- Folding the adjusted/original size derivation into this brick (rather than
  consuming a `txSz` enum + conversion tables) keeps it self-contained and
  table-free, at the cost of the caller passing log2 dims rather than a `txSz`
  handle — consistent with the existing core API.

## Migration Plan

Additive; new module, one new `ReconError` variant, and new public exports. No
existing API changes, and the runtime is unaffected.

## Open Questions

None.
