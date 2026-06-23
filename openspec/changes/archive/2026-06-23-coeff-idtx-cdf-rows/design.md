## Context

`DECODE-COEFF-BASE-CDF-ROWS` loads the ordinary non-IDTX coefficient base,
base-EOB, and base-range row families. `DECODE-COEFF-BASE-PH-CDF-ROW` adds the
parity-hidden-only row needed by the ordinary state-derived first pass. The
remaining generated coefficient CDF defaults that the current row bundle can
reasonably expose without implementing runtime `coeffs()` are the FSC/IDTX row
families named by AV2 §8.3.2:

- `TileCoeffBaseBobCdf`
- `TileCoeffBaseIdtxCdf`
- `TileCoeffBrIdtxCdf`
- `TileIdtxSignCdf`

The default table source already exists in `splot_core::tables::cdf`, generated
from AV2 v1.0.0 §9.3. The context derivation helpers already exist in
`tile_payload/cdf/coeff_context.rs`, but the row storage and selector boundary
still need to expose the rows before a later `useFsc` symbol pass can consume
them.

## Goals / Non-Goals

**Goals:**

- Add the four generated FSC/IDTX coefficient CDF row families to
  `CoeffCdfRows`.
- Validate `coeff_cdf_q_ctx`, `tx_size_ctx` (`0..FSC_TX_SIZE_CONTEXTS`), and
  row-specific `ctx` axes with the existing typed selector error model.
- Include the rows in tile copy, save/average, and frame-end count scaling.
- Prove immutable/mutable row access, generated default selection, copy
  isolation, lifecycle behavior, and symbol-reader handoff.
- Keep matrix, decoder support, decoder conformance coverage, roadmap, and
  generated docs honest.

**Non-Goals:**

- No runtime `coeffs()` integration, `useFsc` branch execution, `QuantSign[]`
  writes, IDTX sign/level symbol sequencing, `read_quant` composition change,
  tile context commit, dequantization, reconstruction, reference refresh, or
  decoded output change.
- No public API, CLI, dependency, licensing, encoder, or broad CDF lifecycle
  expansion beyond the existing row subset.
- No AVM/dav2d invocation from repository code, tests, scripts, or CI.

## Decisions

1. **Keep IDTX rows inside `CoeffCdfRows`.**
   The four rows share the same coefficient-CDF lifecycle as the existing
   base/base-EOB/base-range banks, so adding them to `CoeffCdfRows` preserves the
   existing `TileCdfSelector::Coeff(...)` handoff and avoids a separate symbol
   path.

2. **Use row-specific typed selector variants.**
   `BaseBob`, `BaseIdtx`, `BrIdtx`, and `IdtxSign` encode the exact axes needed
   by §8.3.2 while still returning the common CDF row slice consumed by the
   symbol decoder.

3. **Bounds-check the clamped transform-size context explicitly.**
   The spec selector uses `Min(TX_16X16, txSzCtx)`. This boundary accepts the
   already-clamped `tx_size_ctx` and rejects values outside
   `FSC_TX_SIZE_CONTEXTS`, matching the staged caller-resolved selector pattern
   used elsewhere in the coefficient CDF layer.

4. **Do not route runtime symbols yet.**
   Loading the rows removes a CDF-bank gap without mixing in the larger `useFsc`
   coefficient-loop semantics, which require scan traversal, `Level[]`,
   `QuantSign[]`, and `Quant[]` mutation decisions.

## Risks / Trade-offs

- **Overclaiming support** -> Tracking must say the rows are loaded and
  selectable only; runtime FSC/IDTX coefficient decode remains unsupported.
- **Lifecycle omission** -> Tests cover tile copy isolation plus save/average
  and frame-end count scaling for representative IDTX rows.
- **Selector mismatch** -> The staged API takes a pre-clamped `tx_size_ctx`.
  A future runtime wrapper must derive the spec `Min(TX_16X16, txSzCtx)` value
  from real `txSz` facts before selecting these rows.
