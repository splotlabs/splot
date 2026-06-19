## Why

`ENC-INTRA-BLOCK-MODE-TRACE` composes the mode-info prefix (`y_mode_set`,
`y_mode_index`, `uv_mode`) through one §8.2 coder, but a coded tile body
interleaves mode *and* coefficient symbols through that same single entropy
coder. The next step is to extend the trace across both token kinds: prove the
mode prefix followed by the first `residual()` symbol — the luma `txb_skip`
(`all_zero`) — roundtrips through one §8.2 coder with shared CDF state. This
establishes the unified block-symbol coding model the tile body emits through.

## What Changes

- Add `ENC-INTRA-BLOCK-TRACE-LUMA-SKIP` as a private `splot-encode` encoder-tool
  feature, extending the `block_symbol_trace` module with a unified
  block-symbol token that spans the intra-mode and coefficient token kinds.
- Compose the ordered minimal trace `y_mode_set`, `y_mode_index`, `uv_mode`,
  then the luma `txb_skip` all-zero token (the first `residual()` symbol).
- Add a unified §8.2 roundtrip that holds the mode and `txb_skip` CDF rows from
  `splot-core` defaults and routes each token to its scoped row, proving the
  combined sequence writes through one `SymbolEncoder` and decodes back through
  one `SymbolDecoder` with shared CDF state.
- Expose a small `pub(crate)` luma all-zero token accessor from the coefficient
  tokenization module.
- Preserve the current no-packet-output invariant.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the unified block-symbol trace spanning
  mode and the luma `txb_skip` coefficient symbol through one §8.2 coder.

## Impact

- Affected code: `crates/splot-encode` internals and tests.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status/spec coverage, encoder roadmap/gap audit, and
  `openspec/specs/encoder-tools/spec.md`.
- Public API impact: none; the module is crate-private and not re-exported.
- Dependency impact: none; reuses the existing `splot-core` symbol coder and CDF
  tables.
- Validator/CLI impact: none; no coded packets or public encoder success path.
