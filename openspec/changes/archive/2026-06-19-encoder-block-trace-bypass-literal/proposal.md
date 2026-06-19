## Why

Every block-symbol token so far is a CDF-coded `S()` symbol. But AV2 codes some
syntax as `L(n)` *bypass literals* with no CDF — notably the `sign_bit` of a chroma
or ordinary non-axis luma coefficient (§5.20.7.27 codes the luma DC sign as
`dc_sign` and the directional luma axis signs as `dc_sign_horz_vert`, both CDF;
every other sign is `sign_bit L(1)`) and the §5.20.7.28 `read_quant` golomb tail. The block-symbol trace cannot represent those yet. This
change adds the bypass-literal token kind so the trace and its §8.2 roundtrip can
carry literal bits interleaved with CDF symbols, unblocking coded chroma signs and
the golomb tail.

## What Changes

- Add `ENC-INTRA-BLOCK-TRACE-BYPASS-LITERAL` as a private `splot-encode`
  encoder-tool feature.
- Add a `BlockSymbolToken::Bypass { width, value }` variant (with a `bypass`
  constructor) representing an AV2 §8.2.5 `L(n)` literal.
- Route bypass tokens through `roundtrip_block_symbol_trace` via the `splot-core`
  `SymbolEncoder::write_literal` / `SymbolDecoder::read_literal` primitives,
  dispatched before CDF-row selection (a literal has no CDF row).
- Prove bypass literals interleave bit-exactly with CDF symbols through one §8.2
  coder, and that the roundtrip is deterministic.
- Preserve the current no-packet-output invariant.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the bypass-literal block-symbol token and
  its §8.2 roundtrip.

## Impact

- Affected code: `crates/splot-encode` internals and tests (`block_symbol_trace`,
  one `error` reuse).
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status/spec coverage, encoder roadmap/gap audit, and
  `openspec/specs/encoder-tools/spec.md`.
- Public API impact: none; the module is crate-private and not re-exported.
- Dependency impact: none; reuses the existing `splot-core` symbol coder
  primitives.
- Validator/CLI impact: none; no coded packets or public encoder success path.
