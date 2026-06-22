## Why

The minimal-working-encoder geometry unlock (Route B) needs the coefficient tokenizer to
handle the decoder-verified 16×16 transform size. The 16×16 forward DCT + quantizer landed;
this adds the smallest 16×16 tokenization slice — a single coded DC (eob=1) — establishing
the `TX_SIZE_16X16_CTX` CDF banks and the new `eob_pt_256` family.

## What Changes

- Add `ENC-COEFF-TOKENIZE-16X16-DC` as a private `splot-encode` encoder-tool feature.
- `general_intra_16x16_luma_dc_coded_tokens` — a single coded DC at `TX_SIZE_16X16_CTX`.
- New `EobPt256` selector/syntax (`DEFAULT_EOB_PT_256_CDF`); new TX_16X16 banks in **both**
  CDF routers (entropy-proof + trace).

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: tokenize a single 16×16 luma DC coefficient.

## Impact

- Affected code: `crates/splot-encode/src/coefficient_tokenization.rs`,
  `.../coefficient_tokenization/general_coded.rs`, `.../coefficient_tokenization/cdf_rows.rs`,
  `.../block_symbol_trace/{mod,cdf_rows}.rs`, `closed_loop.rs` (+ tests).
- Verification is §8.2 self-consistency (roundtrip); the load-bearing contexts are mirrored
  verbatim from the decoder (bit-exact decode-verify deferred to the packet path).
- Scope (explicitly NOT claimed): eob>1, the `eob_pt_extra` refinement, HF coefficients,
  golomb, packet emission, the 16×16 `intra_tx_type`/`sec_tx_type` signaling.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature status.
