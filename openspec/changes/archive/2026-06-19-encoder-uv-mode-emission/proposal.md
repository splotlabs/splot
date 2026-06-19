## Why

The encoder can emit the luma intra-mode selectors (`ENC-INTRA-MODE-SYMBOL-EMISSION`),
but a coded intra block also needs the chroma intra-mode selector `uv_mode`
(AV2 §5.20.5.6). This change adds the next private, non-emitting bridge so the
chroma mode syntax can be tested through the in-tree AV2 §8.2 symbol coder before
any tile-body writer or packet path exists.

## What Changes

- Add `ENC-UV-MODE-SYMBOL-EMISSION` as a private `splot-encode` encoder-tool
  feature, implemented by extending the existing `intra_mode_emission` module and
  reusing its token / §8.2 roundtrip machinery.
- Emit the ordered AV2 §5.20.5.6 `uv_mode` entropy-token record for the current
  minimal DC chroma mode (Default_Mode_List_Uv index 0 = DC_PRED) when the luma
  mode is the non-directional DC_PRED, deriving the §8.3.2
  `TileUVModeCflNotAllowedCdf[ctx]` row at the non-directional context 0.
- Prove the token value can be written through the in-tree AV2 §8.2 `splot-core`
  symbol encoder with the scoped default CDF row and decoded back to the same
  value.
- Preserve the current no-packet-output invariant.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for minimal private chroma `uv_mode` symbol
  emission over the current DC chroma mode for a non-directional luma block.

## Impact

- Affected code: `crates/splot-encode` internals and tests.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status/spec coverage, encoder roadmap/gap audit, and
  `openspec/specs/encoder-tools/spec.md`.
- Public API impact: none; the module is crate-private and not re-exported.
- Dependency impact: none; uses the existing `splot-core` dependency only.
- Validator/CLI impact: none; no coded packets or public encoder success path.
