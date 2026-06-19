## Why

The encoder can tokenize coefficients (`ENC-COEFFICIENT-TOKENIZATION-MINIMAL`)
and run a closed loop (`ENC-CLOSED-LOOP-RECONSTRUCTION-MINIMAL`), but it cannot
yet emit the block-level *mode* syntax that precedes coefficients in a coded
tile. The first such symbols a minimal intra block needs are the luma intra mode
selectors (`y_mode_set`, `y_mode_index`). This change adds the next private,
non-emitting bridge so that mode syntax can be tested through the in-tree AV2
§8.2 symbol coder before any tile-body writer or packet path exists.

## What Changes

- Add `ENC-INTRA-MODE-SYMBOL-EMISSION` as a private `splot-encode` encoder-tool
  feature.
- Add an intra-mode emission module that produces the ordered AV2 §5.20.5.5
  `y_mode_set` / `y_mode_index` entropy-token records for the current minimal
  DC_PRED luma block at the tile-origin neutral context, deriving the §8.3.2 CDF
  rows and contexts.
- Prove those token values can be written through the in-tree AV2 §8.2
  `splot-core` symbol encoder with scoped default CDF rows and decoded back to
  the same values.
- Preserve the current no-packet-output invariant.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for minimal private luma intra-mode symbol
  emission over the current top-left DC_PRED block.

## Impact

- Affected code: `crates/splot-encode` internals and tests.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status/spec coverage, encoder roadmap/gap audit, and
  `openspec/specs/encoder-tools/spec.md`.
- Public API impact: none; the module is crate-private and not re-exported.
- Dependency impact: none; uses the existing `splot-core` dependency only.
- Validator/CLI impact: none; no coded packets or public encoder success path.
