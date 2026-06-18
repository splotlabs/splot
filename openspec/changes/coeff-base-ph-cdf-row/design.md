## Context

`DECODE-COEFF-BASE-DERIVED-LEVEL-PASS` already derives
`CoeffBaseSelection::Ph` when parity hiding becomes active before the final DC
coefficient. The helper currently returns an unsupported boundary because
`DECODE-COEFF-BASE-CDF-ROWS` loaded the ordinary luma/chroma base, base-EOB, and
base-range banks but left the parity-hidden-only `TileCoeffBasePhCdf` bank out of
the decode CDF subset.

The generated source-of-truth default table already exists in
`splot_core::tables::cdf::DEFAULT_COEFF_BASE_PH_CDF`, generated from AV2 v1.0.0
§9.3. The selector context is already derived by `CoeffBaseContext::select` from
AV2 §8.3.2. This change only connects those existing facts to row storage,
lifecycle handling, and the loaded-but-unwired first-pass consumer.

## Goals / Non-Goals

**Goals:**

- Add `TileCoeffBasePhCdf[coeff_cdf_q_ctx][ctx]` to the crate-private
  coefficient CDF row bundle.
- Validate q-context and `COEFF_BASE_PH_CONTEXTS` bounds with the existing
  typed selector error model.
- Include the Ph rows in tile copy, save/average, and frame-end count-scaling
  paths.
- Map `CoeffBaseSelection::Ph` to a real selector and prove an eob>=5
  hidden-parity first pass can consume it.
- Keep matrix, decoder support, decoder conformance coverage, roadmap, and
  generated docs honest.

**Non-Goals:**

- No runtime `coeffs()` integration, scan-table derivation, transform-type
  derivation, sign-source derivation, `read_quant` composition change, tile
  context commit, dequantization, reconstruction, reference refresh, or decoded
  output change.
- No public API, CLI, dependency, licensing, encoder, FSC/IDTX, or sign-CDF row
  expansion.
- No AVM/dav2d invocation from repository code, tests, scripts, or CI.

## Decisions

1. **Add `BasePh` to `CoeffCdfSelector`.**
   The selector lives beside the existing coefficient base selectors because
   `read_coeff_base_symbol` already consumes a generic `CoeffCdfSelector`.
   Keeping it in the same enum avoids a second symbol-read path and preserves
   the current `TileCdfSelector::Coeff(...)` handoff.

2. **Store Ph rows inside `CoeffCdfRows`.**
   The row participates in the same clone/copy/save/average lifecycle as the
   other coefficient banks. Reusing `CoeffCdfRows` avoids broadening
   `BlockCdfRows` or the public boundary shape.

3. **Use the existing generated default table.**
   The implementation imports `DEFAULT_COEFF_BASE_PH_CDF`; it does not duplicate
   table contents or spec prose. Shape constants stay local to
   `coeff_rows.rs`, matching the existing generated-default row pattern.

4. **Remove the unsupported first-pass error.**
   Once the row exists, `CoeffBaseSelection::Ph` should return
   `CoeffCdfSelector::BasePh`, and invalid Ph axes should fail through the
   selector bounds checks. The first-pass error enum no longer needs a Ph-only
   unsupported variant.

## Risks / Trade-offs

- **Fixture search cost** -> The first-pass test may need a payload with eob>=5
  and enough nonzero pre-DC levels to activate hidden parity. Keep the search
  bounded to small byte payloads and use existing helper patterns.
- **Overclaiming support** -> The matrix and roadmap must state this is still
  loaded-but-unwired and does not make runtime nonzero coefficient decode
  supported.
- **Lifecycle omission** -> Tests must prove default row selection and tile copy
  isolation; the existing lifecycle code path should include Ph rows through the
  same `avg_rows`/`scale_rows` helpers.
