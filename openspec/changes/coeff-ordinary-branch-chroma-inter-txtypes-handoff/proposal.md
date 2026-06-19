## Why

The ordinary coefficient transform-type handoff still rejects the AV2 section
5.20.7.29 chroma-inter branch after `enable_chroma_dctonly` is false. Supporting
that caller-resolved `TxTypes[y4][x4]` subset removes the last non-lossless
transform-type state branch before the broader runtime `coeffs()` wiring work.

## What Changes

- Add Feature ID `DECODE-COEFF-ORDINARY-BRANCH-CHROMA-INTER-TXTYPES-HANDOFF`.
- Extend the crate-private ordinary coefficient transform-type handoff to accept
  caller-resolved chroma-inter `TxTypes[y4][x4]` for non-lossless chroma blocks.
- Add the AV2 section 5.20.7.29 `Tx_Type_In_Set_Inter` membership check and
  `DCT_DCT` fallback for caller-resolved chroma-inter transform types.
- Preserve existing all-zero, luma `TxTypes`, chroma-DCT-only, chroma intra
  `UVMode`, directional chroma, and lossless unsupported-subset behavior.
- Update implementation matrix, decoder support/conformance coverage, roadmap,
  and generated status docs.

## Capabilities

### New Capabilities

- `coeff-ordinary-branch-chroma-inter-txtypes-handoff`: loaded-but-unwired
  ordinary coefficient branch support for the AV2 section 5.20.7.29
  non-lossless chroma-inter `TxTypes[y4][x4]` transform-type subset.

### Modified Capabilities

- `decoder-support`: record the chroma-inter `TxTypes` ordinary branch row and
  its remaining decoder gaps.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/coeff_loop`.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/DECODER-ROADMAP.md`, generated
  status/coverage docs, and `xtask/src/decoder_conformance_coverage.rs`.
- No new dependencies, public API, crate dependency graph, encoder, writer, or
  runtime decode-output changes.
- Non-goals: runtime `coeffs()` integration, deriving `TxTypes`, `MiRow`,
  `MiCol`, or chroma subsampling facts from frame state, FSC/IDTX lossless
  handling, block syntax traversal, dequantization, inverse transform, residual
  add, output/reference refresh, and AVM/dav2d byte-match proof.
