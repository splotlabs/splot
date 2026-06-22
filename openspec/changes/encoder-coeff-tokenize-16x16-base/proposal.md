## Why

The minimal-working-encoder geometry unlock needs the coefficient tokenizer to handle
arbitrary 16×16 base-pass blocks, not just a single DC. This makes the general walk
size-generic (one codepath for 4×4 and 16×16) and adds the 16×16 base pass (eob 1..=32).

## What Changes

- Add `ENC-COEFF-TOKENIZE-16X16-BASE` as a private `splot-encode` encoder-tool feature.
- Introduce a `TxGeom` descriptor + `EobPtKind` and thread it through one walk codepath; the
  4×4 entry delegates with `TxGeom::TX_4X4` (byte-identical).
- Add `tokenize_general_16x16_luma_block` for eob 1..=32 (eob>32 rejected — the
  `eob_pt_extra` refinement is a later brick).

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: tokenize an arbitrary 16×16 luma base-pass block (eob 1..=32).

## Impact

- Affected code: new `general_walk_geom.rs`, `general_walk_16x16.rs` (+ tests); modified
  `general_walk.rs`, `general_walk_recover.rs`, `multi_coeff.rs`, `coeff_base_lf.rs`,
  `coefficient_tokenization.rs`, both CDF routers.
- The 4×4 path is byte-identical (all existing 4×4 + frame tests pass). Verification is §8.2
  self-consistency; the contexts are mirrored verbatim from the decoder.
- Scope (NOT claimed): the `eob_pt_extra` refinement (eobPt 8/9), packet emission, the 16×16
  `intra_tx_type`/`sec_tx_type` signaling.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature status.
