## Why

Sub-brick 5d-i — the first high-frequency coefficient. The general tokenizer handled
the entire 4x4 low-frequency region (eob 1-10, `row+col < 4`). This lifts it to eob 11,
whose single HF coefficient (scan index 10 = raster 13, `row+col = 4`) is the EOB
coefficient — isolating the new HF EOB CDF family and the dual-router wiring with the
smallest surface. The non-EOB HF `coeff_base` is deferred to 5d-ii.

A 4-agent adversarial review and the decoder source confirmed three HF divergences,
each a silent bit-mismatch if copied from the LF path: the HF EOB level cap / 3-symbol
table (`NUM_BASE_LEVELS = 2`), the HF `coeff_br` plain-`mag` context (no `+7`), and the
4-symbol HF `coeff_base_eob` table.

## What Changes

- Add `ENC-COEFF-GENERAL-WALK-HF-EOB11` as a private `splot-encode` encoder-tool
  feature.
- Add `NUM_BASE_LEVELS = 2` to splot-core (AV2 §3, the non-low-frequency base-level
  threshold; the sibling of the existing `LF_NUM_BASE_LEVELS`).
- Lift `tokenize_general_lf_luma_block` to eob 11: per-coefficient `is_lf = (row+col<4)`
  selects the LF vs HF `coeff_base_eob` / `coeff_br` token + table; the HF EOB level
  saturates at 3 (3-symbol table) and the HF no-golomb magnitude cap is 5. Reject a
  nonzero at scan index ≥ 11.
- Add `CoeffBaseEob` / `CoeffBr` selectors + token constructors + HF CDF banks to BOTH
  §8.2 proof routers; add an `is_lf` param to `coeff_br_lf_luma_context` (HF non-DC
  returns plain `mag`).
- Split the §8.2 entropy-proof harness into `entropy_proof.rs` to keep the parent under
  the 1000-line budget.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: extend the general coefficient walk to eob 11 with the
  high-frequency EOB coefficient and its CDF family.

## Impact

- Affected code: `crates/splot-core/src/coefficient.rs` (additive `NUM_BASE_LEVELS`),
  `crates/splot-encode/src/coefficient_tokenization{.rs,/general_walk.rs,/multi_coeff.rs,
  /coeff_base_lf.rs,/cdf_rows.rs,/entropy_proof.rs}`, `block_symbol_trace/cdf_rows.rs`
  (+ tests).
- Scope (explicitly NOT claimed): non-EOB high-frequency coefficients (5d-ii), golomb
  magnitudes, chroma, non-4x4 sizes, non-DCT_DCT, packets, decoder context/table
  conformance (the §8.2 roundtrip proves self-consistency only).
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature status /
  spec coverage.
