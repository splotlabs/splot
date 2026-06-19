# Encoder gap audit

`status: active`
`owner: encoder`
`Feature ID: DOC-ENCODER-PROGRAM-CONTRACT`
`audit date: 2026-06-18`

This audit records the baseline for the first encoder contract PR. It is scoped to
planning and status; it does not claim encoder behavior exists.

## API and CLI baseline

- `splot-encode` is an API shell. `Frame` models validated borrowed 8-bit YUV420
  input views under `ENC-Y4M-INPUT`, and `Context` now exposes a typed
  accepting/draining/finished/failed lifecycle under
  `ENC-CONTEXT-STATE-MACHINE`.
- `ENC-SYNTAX-IR` adds a private deterministic planning model for future
  sequence/frame/tile/block/token decisions. It is not re-exported, owns no bit
  writer, and does not produce packets.
- `ENC-MINIMAL-HEADER-PLAN` adds a private bridge from current encoder
  configuration and first-frame metadata to typed sequence/frame/tile-group
  header intent. It rejects unsupported formats and mismatches, is not
  re-exported, owns no writer, and does not produce packets.
- `ENC-SPEED-PRESETS` defines the typed runtime preset boundary used by
  `splot encode --speed`. The preset is stored in `EncoderRuntimeConfig`, not
  `EncoderConfig`, and currently does not affect packet output because no packet
  output path exists.
- `ENC-RESIDUAL-FOUNDATION` adds the first private encoder arithmetic primitive:
  checked source-minus-prediction residual blocks for the current borrowed 8-bit
  input surface. It is not re-exported, owns no writer, and does not produce
  packets.
- `ENC-FORWARD-TRANSFORM-FOUNDATION` adds a private 4x4 DCT_DCT DC-only
  forward-transform primitive for uniform residual blocks. It proves the no-op
  quant/dequant inverse handoff through `splot-recon`, but it is not a broad
  transform family, quantizer, token writer, or packet path.
- `ENC-QUANTIZATION-V0` adds a private fixed-quantizer stage for that first 4x4
  DCT_DCT DC-only coefficient subset. It validates qindex and dequant inputs,
  proves the `splot-recon` dequant/inverse handoff for qindex zero, and is not a
  quantization-matrix path, token writer, range encoder, rate-control mode, or
  packet path.
- `ENC-COEFFICIENT-TOKENIZATION-MINIMAL` adds a private token-fact bridge for the
  current luma 4x4 DCT_DCT DC-only top-left neutral-spatial-context quantized
  subset. It derives scan, EOB, begin-position, sign/magnitude, q-context, and
  ordered base-tier entropy-token records, including the low-frequency EOB base
  CDF row, and proves those records through the in-tree AV2 §8.2 symbol
  encoder/decoder. It is not broad coefficient syntax, neighbor-derived spatial
  contexts, coefficient base-range / `read_quant` extension, tile CDF lifecycle,
  tile-body emission, rate control, or a packet path.
- `ENC-CLOSED-LOOP-RECONSTRUCTION-MINIMAL` adds a private closed-loop
  reconstruction for the current 8-bit luma 4x4 DCT_DCT DC-only top-left subset.
  It composes the encoder residual/forward-transform/quantization stages with
  the `splot-recon` decoder-visible AV2 §7.13.2.10 DC prediction,
  §7.14.2/§7.14.4 dequantization, §7.15.4 inverse transform, and §7.14.3
  reconstruct (residual addition), freezes the result into a `splot-recon`
  current-frame workspace, and hashes it. It proves the qindex-zero flat subset
  is lossless and that the emitted coefficient decisions reconstruct identically.
  It is not chroma/inter/multi-block reconstruction, reference-frame storage,
  tile-body emission, a packet path, or a public encode success path.
- `ENC-INTRA-MODE-SYMBOL-EMISSION` adds a private block-symbol emission bridge
  for the luma intra-mode selectors (`y_mode_set`/`y_mode_index` for DC_PRED at
  the tile-origin neutral context, AV2 §5.20.5.5/§8.3.2). It derives the scoped
  §8.3.2 CDF rows and proves the token values roundtrip through the in-tree AV2
  §8.2 symbol encoder/decoder. It is not chroma mode, coefficient/all-zero
  symbols, partition syntax, tile-body emission, or a packet path.
- `ENC-UV-MODE-SYMBOL-EMISSION` extends `intra_mode_emission` with the chroma
  `uv_mode` selector for the DC chroma mode (`Default_Mode_List_Uv` index 0 =
  DC_PRED) at the non-directional context (AV2 §5.20.5.6/§8.3.2), proven through
  the in-tree AV2 §8.2 symbol coder. It is not CfL/CCTX, directional-luma
  contexts, coefficient/all-zero symbols, tile-body emission, or a packet path.
- `ENC-INTRA-BLOCK-MODE-TRACE` adds a private `block_symbol_trace` module that
  composes the ordered AV2 §5.20.5.3 mode-info prefix (`y_mode_set`,
  `y_mode_index`, `uv_mode`) by reusing the merged mode emitters, proving the
  combined sequence roundtrips through one in-tree AV2 §8.2 coder with shared CDF
  state. It is the home for the growing block-symbol trace; coefficient symbols
  from `residual()` join later. It is not coefficient/all-zero symbols, tile-body
  emission, or a packet path.
- `ENC-INTRA-BLOCK-TRACE-LUMA-SKIP` extends `block_symbol_trace` with a unified
  `BlockSymbolToken` spanning the intra-mode and coefficient token kinds, and
  composes the mode prefix plus the first `residual()` symbol (the luma
  `txb_skip` / §5.20.7.27 `all_zero`), proving the combined sequence through one
  in-tree AV2 §8.2 coder with shared CDF state. It is not chroma `txb_skip`,
  non-all-zero coefficients, tile-body emission, or a packet path.
- `ENC-INTRA-BLOCK-TRACE-CHROMA-SKIP` completes the minimal all-zero intra block
  symbol trace: the mode prefix plus the per-plane luma/U/V `txb_skip`
  (§5.20.7.27 `all_zero`) symbols in `residual()` order, proven through one
  in-tree AV2 §8.2 coder with shared CDF state. Per §8.3.2
  `TileTxbSkipCdf[is_inter||fsc_mode][txSzCtx][ctx]`, U reuses the same bank as
  luma (the first index is `is_inter||fsc_mode` = 0 for an intra non-FSC block,
  not plane type) and is distinguished only by ctx 6; V uses the dedicated
  `TileVTxbSkipCdf` at ctx 0. It is not non-all-zero coefficients, tile-body
  emission, or a packet path.
- `ENC-INTRA-BLOCK-TRACE-CODED-DC` adds the minimal *coded* (non-all-zero) intra
  block symbol trace: the mode prefix, the coded luma `residual()` for a single
  nonzero DC coefficient (`txb_skip=0`, `eob_pt_16`, `coeff_base_eob`, `dc_sign`
  per §5.20.7.27), then the all-zero U/V `txb_skip`, proven through one in-tree
  AV2 §8.2 coder with shared CDF state. A `pub(crate)` `luma_dc_coded_tokens`
  accessor mirrors the tokenizer's coded DC path, guarded by an equivalence test.
  It is not multi-coefficient blocks, `coeff_br` base-range, chroma coefficients,
  partition syntax, tile-body emission, or a packet path.
- `ENC-INTRA-BLOCK-TRACE-CODED-BR` extends the coded DC trace with the
  §5.20.7.27 base-range tier: a larger luma DC coefficient (magnitude 5..=7)
  emits a `coeff_br` symbol after a saturated `coeff_base_eob`, using the
  low-frequency `TileCoeffBrLfCdf` at the DC context 0 (verified vs the decoder).
  Magnitude 8 reaches `maxLevel` (§5.20.7.27) so §5.20.7.28 `read_quant` would
  emit the golomb tail; it is rejected until that brick, so the cap is 7.
  `luma_dc_coded_tokens` is now the single coded-DC token source (the tokenizer
  delegates to it; an equivalence test covers magnitude 1..=7). It is not the
  golomb tail, multi-coefficient blocks, chroma coefficients, tile-body emission,
  or a packet path.
- `ENC-INTRA-BLOCK-TRACE-BYPASS-LITERAL` adds the §8.2.5 bypass-literal token kind
  (`BlockSymbolToken::Bypass { width, value }`) routed through the trace roundtrip
  via `SymbolEncoder::write_literal` / `SymbolDecoder::read_literal`. It is the
  foundation for syntax not coded as a CDF symbol — the `sign_bit` of a chroma or ordinary
  non-axis luma coefficient (§5.20.7.27 codes the luma DC sign as `dc_sign` and
  the directional luma axis signs as `dc_sign_horz_vert`, both CDF) and the
  §5.20.7.28 golomb tail. Proven by a mixed CDF+bypass roundtrip; it has no
  consumer yet and is not coded chroma signs, the golomb tail, tile-body emission,
  or a packet path.
- `ENC-INTRA-BLOCK-TRACE-CODED-CHROMA-DC` adds the first coded *chroma*
  coefficient (a coded U-plane DC), built on the §8.2.5 bypass-literal token. The
  U residual is `txb_skip=0` + chroma `eob_pt_16` (ctx 2) + chroma `coeff_base_eob`
  (the `TileCoeffBaseLfEobUvCdf`) as CDF tokens, then the U DC sign as a `sign_bit`
  `L(1)` bypass literal (§5.20.7.27 codes the `dc_sign` CDF only for the luma DC
  and `dc_sign_horz_vert` for the directional luma axis signs), then the all-zero
  V `txb_skip` at the §8.3.2 EobU context 6 (since the coded U sets `EobU != 0`) —
  all verified vs the decoder and proven through one §8.2 coder. The accessor
  rejects out-of-tier magnitudes with a typed error. It is not the chroma
  base-range/golomb tiers, V-plane coded coefficients, multi-coefficient blocks,
  tile-body emission, or a packet path.
- `ENC-INTRA-BLOCK-TRACE-GOLOMB-FINITE` adds the §5.20.7.28 `read_quant` finite-q
  golomb tail (the bypass-literal token's first real consumer), covering the
  full finite-q luma DC range (magnitude 8..=17), proven by a range loop test that
  composes/roundtrips/reconstructs every magnitude in the range (the base/range
  tokenizer accessor stays capped at 7; the golomb is composed in the trace). The luma residual is the fixed level
  tokens (level reaches `maxLevel=8`), then the `dc_sign` CDF token, then
  the golomb `q_length`/`coeff_rem` bypass bits encoding `x = magnitude - 8`
  (`m=1` for the first DC → `q=x>>1`, `coeff_rem=x&1`); §5.20.7.27's sign+quant
  pass reads the sign before `read_quant`. A conformance test reconstructs
  the magnitude from the decoded golomb bits via the decoder's finite-q
  arithmetic. It is not the golomb-prefix tier (magnitude 18+), multi-coefficient
  blocks, chroma golomb, tile-body emission, or a packet path.
- `ENC-INTRA-BLOCK-TRACE-GOLOMB-PREFIX` extends the golomb tail to the
  §5.20.7.28 golomb-prefix path (`q == cMax`), completing the single-coefficient
  luma DC magnitude vocabulary up to 525. The trace emits the fixed level tokens,
  the `dc_sign` CDF token, then the prefix bypass bits (5 q_length zeros, the
  golomb_length unary, and `coeff_rem` as one `L(length)` literal). A range loop
  test reconstructs every magnitude 18..=525 via the decoder's golomb-prefix
  arithmetic; out-of-range magnitudes return a typed runtime error. It is not
  multi-coefficient blocks, chroma golomb, tile-body emission, or a packet path.
- `ENC-COEFF-BASE-LF-CONTEXT` adds the §8.3.2 `coeff_base` low-frequency luma
  context derivation (the neighbour-sum context multi-coefficient blocks need),
  mirroring the decoder's `CoeffBaseContext` LF branch via the shared
  `SIG_REF_DIFF_OFFSET` table. It is loaded but unread (no token, CDF, or packet);
  the eob>1 trace brick consumes it. It is not chroma/parity-hidden/high-frequency
  contexts, token emission, or a packet path.
- `ENC-COEFF-BASE-LF-TOKEN` adds the non-EOB `coeff_base` low-frequency luma
  token and its `TileCoeffBaseLfCdf` row (the second multi-coefficient building
  block after the context derivation), roundtrip-proven through the §8.2 coder. It
  is available but not yet composed into a trace; the eob>1 trace brick consumes
  it. It is not a multi-coefficient trace, chroma/high-frequency `coeff_base`, or a
  packet path.
- `Packet` is still only a byte buffer wrapper, and no coded packet production
  path exists.
- `EncoderConfig` exposes `BitDepth::Twelve`, but current Baseline Encoder Profile
  v1 does not support 12-bit encode.
- The CLI encode command constructs a context, exercises the lifecycle boundary,
  and exits with the existing "not yet implemented" path. It does not read input
  or write output.

## Writer baseline

- `ENC-BITSTREAM-WRITER` is the current writer foundation.
- `splot-core` has writer primitives, OBU payload writers for the parsed OBU model,
  Annex B framing helpers, IVF helpers, the generic AV2 §8.2 `SymbolEncoder`
  primitive, and round-trip/fuzz coverage.
- This is still syntax/framing support, not an encoder. Coded tile payload
  generation is missing because broad encoder-owned §8.3 token/CDF selection,
  tile CDF lifecycle, coefficient-loop coverage, and the AV2 `decode_tile()`
  body path remain unimplemented.
- The private syntax IR and minimal header plan can stage future writer inputs,
  but no IR-to-writer serialization path exists yet.
- Inter first-group tile-group composition remains blocked on inter frame-header
  writer support.
- Partial or unimplemented syntax models must be rejected by writers rather than
  silently emitted.

## Reconstruction baseline

- `splot-recon` exposes frame/plane views, current-frame workspace, intra
  prediction primitives, dequant, inverse transforms, residual addition,
  reference-store pieces, hash input, and Y4M output pieces.
- It is not a byte-consuming decoder and does not yet provide a full encoder
  closed-loop reconstruction API.
- `splot-encode` has a direct `splot-recon` dependency and uses recon borrowed
  plane/shared-frame views for input. A private minimal closed-loop
  reconstruction now composes the encoder forward path with the `splot-recon`
  decoder-visible reconstruction process for the 8-bit luma 4x4 DCT_DCT DC-only
  top-left subset, including the current-frame workspace and decoded-frame hash
  (`ENC-CLOSED-LOOP-RECONSTRUCTION-MINIMAL`). It still has no reference-frame
  storage, packet generation, or public encode success path, and reconstruction
  beyond the minimal subset (chroma, inter, multi-block, broader transforms)
  remains future work.

## Conformance baseline

- `splot validate` is the first legality gate for future encoder output.
- `splot decode` evidence is required before public success for closed-loop output,
  but the exact phase boundary is still future work.
- Live AVM/dav2d differential runs are supplemental until self-contained harness
  work exists. They must not make CI depend on the network or uncommitted tools.
- `CONF-AVM-DIFF-HARNESS` remains future work.

## Active ownership baseline

As of the `encoder-coefficient-tokenization-minimal` branch point on
2026-06-18, PR #279 has merged into `main` as `8c9c5e27230e`, and this branch is
based on `origin/main` at that commit. PR #280
(`codex/decode-coeff-ordinary-branch-plane-tx-type`) is open against `main`; it
overlaps generated/tracking docs and coefficient-domain terminology, but its
code is decoder-local while this branch is private encoder tokenization. If
PR #280 lands first, rebase and regenerate matrix/status/coverage docs before
merge.

## Parked work

`toy-intra-encoder-v0` remains unchecked and parked. It is superseded as the
starting point for implementation by Baseline Encoder Profile v1. Future all-intra
work must be proposed with current writer, recon, validation, and conformance gates.
