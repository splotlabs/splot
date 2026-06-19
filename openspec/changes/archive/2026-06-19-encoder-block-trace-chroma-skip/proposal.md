## Why

`ENC-INTRA-BLOCK-TRACE-LUMA-SKIP` proves the mode prefix plus the luma `txb_skip`
through one §8.2 coder, but a coded all-zero intra block reads a per-plane
`all_zero` for luma, U, *and* V in `residual()` order. This change completes the
minimal all-zero intra block symbol sequence by adding the chroma U and V
`txb_skip` symbols, proving the full six-symbol trace through one coder.

## What Changes

- Add `ENC-INTRA-BLOCK-TRACE-CHROMA-SKIP` as a private `splot-encode`
  encoder-tool feature.
- Add `pub(crate)` chroma U and V `all_zero` token accessors to
  `coefficient_tokenization`, including a new `VTxbSkip` CDF-row selector for the
  dedicated `TileVTxbSkipCdf`.
- Extend `block_symbol_trace` with `compose_minimal_intra_dc_complete_all_zero_block_trace`,
  the ordered `y_mode_set`, `y_mode_index`, `uv_mode`, then per-plane luma/U/V
  `txb_skip` (`all_zero`) sequence, and route the U/V `txb_skip` CDF rows
  (`DEFAULT_TXB_SKIP_CDF[..][1][..][6]` for U; `DEFAULT_V_TXB_SKIP_CDF[..][0]` for
  V) through the unified §8.2 roundtrip.
- Prove the complete six-symbol trace writes through one `SymbolEncoder` and
  decodes back through one `SymbolDecoder` with shared CDF state.
- Preserve the current no-packet-output invariant.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the complete minimal all-zero intra
  block symbol trace (mode prefix + per-plane Y/U/V `txb_skip`).

## Impact

- Affected code: `crates/splot-encode` internals and tests.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status/spec coverage, encoder roadmap/gap audit, and
  `openspec/specs/encoder-tools/spec.md`.
- Public API impact: none; the module is crate-private and not re-exported.
- Dependency impact: none; reuses the existing `splot-core` symbol coder and CDF
  tables.
- Validator/CLI impact: none; no coded packets or public encoder success path.
