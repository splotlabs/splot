## Why

Completes the 16×16 coefficient tokenizer: the base pass (4b) covered eob 1..=32; this adds
the `eob_pt_256` symbol-7 `eob_pt_extra` refinement so the tokenizer handles the full eob
range (1..=256), which an arbitrary-content 16×16 block needs.

## What Changes

- Add `ENC-COEFF-TOKENIZE-16X16-REFINE` as a private `splot-encode` encoder-tool feature.
- Emit `eob_pt_256` symbol 7 for both eobPt 8 and 9, with a 1-bit `eob_pt_extra` literal
  (value `eobPt-8`) after the symbol and before `eob_extra`.
- Add a separate full-range entry `tokenize_general_16x16_luma_block_full` (the base entry's
  eob>32 reject contract is unchanged).

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: tokenize an arbitrary 16×16 luma block over the full eob range (1..=256).

## Impact

- Affected code: `general_walk.rs`, `general_walk_recover.rs`, `general_walk_16x16.rs`,
  `coefficient_tokenization.rs` (+ new refine tests).
- 4×4 and the 16×16 base pass stay byte-identical. Verification is §8.2 self-consistency; the
  symbol-7 mapping/order is mirrored verbatim from the decoder.
- Scope (NOT claimed): packet emission, the 16×16 `intra_tx_type`/`sec_tx_type` signaling.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature status.
