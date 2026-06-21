## Why

The closed loop now produces quantized blocks with real non-zero AC (the forward DCT
+ quantizer phase). To emit them, the encoder needs a GENERAL coefficient tokenizer:
the encoder coeff loop that walks an arbitrary quantized `Quant[16]` and emits the
§5.20.7.27 token stream the decoder coeff loop reads. Today only the single-DC
tokenizer exists; the multi-coefficient machinery is hand-composed traces for
specific magnitudes, not a general walk.

This is sub-brick 5a — the smallest general step: the low-frequency base tier
(eob <= 2, magnitudes 1..=4). It establishes the scan + eob + reverse-scan base pass
+ reverse-scan sign pass + recovery harness that the later sub-bricks (coeff_br,
eob_extra, high-frequency, golomb, chroma, decode cross-check) extend.

## What Changes

- Add `ENC-COEFF-GENERAL-WALK-LF-BASE` as a private `splot-encode` encoder-tool
  feature.
- Add `tokenize_general_lf_luma_block(quant, coeff_cdf_q_ctx)` in a new
  `coefficient_tokenization/general_walk.rs`: walks an arbitrary 4x4 DCT_DCT luma
  `Quant[16]` whose nonzeros are at scan indices 0..=1 (eob <= 2, LF) with base-tier
  magnitudes 1..=4, and emits the ordered §5.20.7.27 token stream (coded `all_zero`,
  `eob_pt_16`, reverse-scan base pass with the running-`Level[]` `coeff_base` context,
  reverse-scan interleaved sign pass, chroma all-zero). Reuses the existing token
  accessors + `coeff_base_lf_luma_context` + `roundtrip_block_symbol_trace`.
- Add `recover_quant_from_tokens(...)` that re-reads the emitted stream in the same
  reverse-scan order and rebuilds the signed `Quant[16]` — §8.2 self-consistency
  (internal reversibility of the emitted level/sign/position triples), NOT decoder
  conformance (deferred to the cross-check sub-brick).
- Add `Error::CoefficientTokenizationUnsupportedEob` for a nonzero beyond scan index
  1; `|quant| > 4` reuses the existing unsupported-magnitude error. Add the routed
  `CoeffBaseLf` 4x4 ctx-2 CDF row (sourced from the generated splot-core table) the
  asymmetric test reaches.
- No coeff_br, no golomb, no eob_extra, no high-frequency, no chroma coefficient
  coding, no packet output.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the general low-frequency base-tier
  coefficient walk (eob <= 2) over an arbitrary quantized 4x4 luma block.

## Impact

- Affected code: `crates/splot-encode/src/coefficient_tokenization/general_walk.rs`
  (+ tests), `coefficient_tokenization.rs` (submodule registration), `error.rs`,
  `block_symbol_trace/cdf_rows.rs` + `mod.rs` (one routed CDF row).
- Scope (explicitly NOT claimed): coeff_br / golomb magnitudes, eob > 2 / eob_extra,
  high-frequency coefficients, chroma coefficient coding, transform sizes other than
  4x4, types other than DCT_DCT, intra_tx_type/sec_tx_type signaling, packet output,
  and decoder/AVM context conformance (the §8.2 roundtrip proves self-consistency
  only; the splot-decode cross-check is a later sub-brick).
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature status
  / spec coverage.
