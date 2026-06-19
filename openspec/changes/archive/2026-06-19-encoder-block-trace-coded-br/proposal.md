## Why

The coded DC block trace currently supports only magnitudes 1..=4 (the
`coeff_base_eob` base tier) and rejects larger DC coefficients. AV2 § 5.20.7.27
codes a low-frequency EOB coefficient whose base level (`coeff_base_eob + 1`)
exceeds `LF_NUM_BASE_LEVELS` with an additional `coeff_br` (base-range) symbol.
This change adds `coeff_br`, extending the coded luma DC range to 5..=7 and
proving the base-range tier roundtrips through one §8.2 coder.

## What Changes

- Add `ENC-INTRA-BLOCK-TRACE-CODED-BR` as a private `splot-encode` encoder-tool
  feature.
- Add the `coeff_br` token syntax and a `CoeffBrLf` CDF-row selector
  (`TileCoeffBrLfCdf`) to `coefficient_tokenization`, and make the coded DC
  tokenizer emit `coeff_br = magnitude - (LF_NUM_BASE_LEVELS + 1)` when
  `magnitude > LF_NUM_BASE_LEVELS`, raising the supported magnitude cap from 4 to
  `LF_NUM_BASE_LEVELS + COEFF_BASE_RANGE = 7` (magnitude 8 reaches `maxLevel` and
  needs the §5.20.7.28 `read_quant` golomb tail, a later brick).
- Make `luma_dc_coded_tokens` the single source of the coded DC token shape
  (returning a variable-length token list), delegated to by
  `tokenize_coefficients`, with the `coded_dc_tokens_match_tokenizer` equivalence
  test extended over the full 1..=7 range.
- Extend `block_symbol_trace` with `compose_minimal_intra_dc_br_block_trace` and
  route the `coeff_br_lf` CDF row through the unified §8.2 roundtrip.
- Prove the ten-symbol base-range coded-block trace writes through one
  `SymbolEncoder` and decodes back through one `SymbolDecoder` with shared CDF
  state.
- Preserve the current no-packet-output invariant.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the coded base-range (`coeff_br`) intra
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
