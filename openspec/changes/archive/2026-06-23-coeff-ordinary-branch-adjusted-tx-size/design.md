## Context

The ordinary coefficient branch has been staged from explicit caller facts toward
AV2 section 5.20.7.27 `coeffs()` inputs. The latest wrapper derives raw
`Tx_Width[txSz]`, `Tx_Height[txSz]`, `Tx_Width_Log2[txSz]`, and
`Tx_Height_Log2[txSz]` from generated section 9.2 tables, but it still feeds raw
dimensions into the ordinary base-context pass. AV2 section 8.3.2 instead uses
`Adjusted_Tx_Size[txSz]` when deriving `coeff_base`, `coeff_base_eob`, and
`coeff_br` context geometry.

## Goals / Non-Goals

**Goals:**
- Resolve `Adjusted_Tx_Size[txSz]` through the generated
  `splot_core::tables::conversion::ADJUSTED_TX_SIZE` table.
- Use adjusted `Tx_Width_Log2`, `Tx_Width`, and `Tx_Height` only for ordinary
  base-context derivation.
- Preserve raw transform dimensions for `coeffs()` block geometry and EOB-size
  context derivation.
- Keep failures before any coefficient context state, CDF row, or symbol-decoder
  mutation.
- Prove 64-sample-side adjustment with focused tests.

**Non-Goals:**
- Do not derive `txSzCtx` from `Tx_Size_Sqr` and `Tx_Size_Sqr_Up` in this change.
- Do not implement section 5.20.7.29 `compute_tx_type`.
- Do not derive scan order or wire runtime `coeffs()`.
- Do not dequantize, inverse transform, residual-add, reconstruct, output, or
  refresh references.

## Decisions

- Keep one wrapper and derive both raw and adjusted dimension packs inside
  `apply_coeff_ordinary_branch_from_tx_size_dimensions`. The raw pack is already
  needed for `NonZeroCoeffEobContextInput` and block geometry; the adjusted pack
  should be local to the base-context handoff.
- Reuse the existing table-bound/value validation helpers for
  `ADJUSTED_TX_SIZE`. Invalid `txSz` or invalid adjusted table values therefore
  fail before downstream mutation and use the existing typed
  `CoeffOrdinaryBranchError` variants.
- Leave `tx_size_ctx` caller-supplied. AV2 section 5.20.7.27 derives it from
  `Tx_Size_Sqr[txSz]` and `Tx_Size_Sqr_Up[txSz]`, but that is a separate caller
  fact from the adjusted geometry used by section 8.3.2 base contexts.

## Risks / Trade-offs

- [Risk] Mixing raw and adjusted dimensions can be subtle.
  -> Mitigation: name the dimension packs distinctly, cite the relevant spec
  sections in comments, and add a test where `TX_64X32` keeps raw EOB/block
  geometry but uses adjusted `TX_32X32` base-context dimensions.
- [Risk] This can be mistaken for full `txSz` integration.
  -> Mitigation: tracking and roadmap notes continue to list `txSzCtx`,
  `compute_tx_type`, scan derivation, and runtime `coeffs()` as deferred.
