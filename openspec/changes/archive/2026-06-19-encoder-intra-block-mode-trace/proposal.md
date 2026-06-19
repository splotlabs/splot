## Why

The encoder can now emit the luma intra-mode selectors
(`ENC-INTRA-MODE-SYMBOL-EMISSION`) and the chroma `uv_mode` selector
(`ENC-UV-MODE-SYMBOL-EMISSION`), but each is tested in isolation with fresh CDF
state. A coded intra block reads them as one ordered sequence through a single
entropy decoder, so the next step is to compose them in AV2 §5.20.5.3 mode-info
order and prove the combined sequence roundtrips through one in-tree AV2 §8.2
coder with shared CDF state.

## What Changes

- Add `ENC-INTRA-BLOCK-MODE-TRACE` as a private `splot-encode` encoder-tool
  feature in a new `block_symbol_trace` module.
- Compose the ordered intra-block mode-info prefix — `y_mode_set`,
  `y_mode_index`, then `uv_mode` (AV2 §5.20.5.3 calls `read_intra_y_mode()`
  before `read_intra_uv_mode()`) — by reusing the existing mode emitters.
- Prove the composed sequence writes through one in-tree AV2 §8.2 `SymbolEncoder`
  and decodes back through one `SymbolDecoder` to the same ordered symbols with
  shared CDF state.
- Preserve the current no-packet-output invariant.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for composing the minimal intra-block
  mode-info symbol prefix into one ordered, roundtrip-proven trace.

## Impact

- Affected code: `crates/splot-encode` internals and tests.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status/spec coverage, encoder roadmap/gap audit, and
  `openspec/specs/encoder-tools/spec.md`.
- Public API impact: none; the module is crate-private and not re-exported.
- Dependency impact: none; reuses existing `splot-core` symbol coder.
- Validator/CLI impact: none; no coded packets or public encoder success path.
