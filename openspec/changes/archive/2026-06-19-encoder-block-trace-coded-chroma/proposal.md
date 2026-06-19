## Why

The coded block traces so far code only luma coefficients; chroma planes are
always all-zero. A real intra frame codes chroma coefficients too. This change
adds the minimal *coded chroma* block: a coded U-plane DC coefficient, proving the
§5.20.7.27 chroma coefficient symbols (`txb_skip`, `eob_pt_16`, `coeff_base_eob`,
`dc_sign`) with their §8.3.2 chroma contexts roundtrip through one §8.2 coder.

## What Changes

- Add `ENC-INTRA-BLOCK-TRACE-CODED-CHROMA-DC` as a private `splot-encode`
  encoder-tool feature.
- Add a `CoeffBaseLfEobUv` CDF-row selector (`TileCoeffBaseLfEobUvCdf`) and a
  `pub(crate)` `chroma_u_dc_coded_tokens` accessor to `coefficient_tokenization`
  returning the four ordered coded chroma U DC tokens with the §8.3.2 chroma
  contexts (eob ctx 2, chroma base-eob CDF at DC ctx 0, `dc_sign` ptype 1).
- Extend `block_symbol_trace` with `compose_minimal_intra_dc_coded_chroma_block_trace`
  (coded luma + coded U + all-zero V) and route the chroma `eob_pt_16`,
  `coeff_base_lf_eob_uv`, and chroma `dc_sign` CDF rows through the unified §8.2
  roundtrip.
- Prove the twelve-symbol coded-chroma-block trace writes through one
  `SymbolEncoder` and decodes back through one `SymbolDecoder` with shared CDF
  state.
- Preserve the current no-packet-output invariant.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the minimal coded chroma (U-plane) intra
  DC block symbol trace.

## Impact

- Affected code: `crates/splot-encode` internals and tests.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status/spec coverage, encoder roadmap/gap audit, and
  `openspec/specs/encoder-tools/spec.md`.
- Public API impact: none; the module is crate-private and not re-exported.
- Dependency impact: none; reuses the existing `splot-core` symbol coder and CDF
  tables.
- Validator/CLI impact: none; no coded packets or public encoder success path.
