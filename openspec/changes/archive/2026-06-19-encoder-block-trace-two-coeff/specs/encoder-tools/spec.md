## ADDED Requirements

### Requirement: Encoder eob=2 multi-coefficient block trace

The encoder SHALL provide a private eob=2 multi-coefficient luma block trace stage
tracked by `ENC-INTRA-BLOCK-TRACE-TWO-COEFF`, the first block trace with more than
one coefficient. For a 4x4 DCT_DCT luma block with one nonzero AC coefficient
(level 1) at scan index 1 (raster position 4, derived from the AV2 2D scan order)
and a zero DC at scan index 0, the luma `residual()`
SHALL emit `all_zero=0`, `eob_pt_16=1` (eob 2), then the base pass — the AC
`coeff_base_eob` at context 1 and the DC non-EOB `coeff_base` at the §8.3.2
low-frequency context DERIVED from the AC's `Level[]` via the merged context
derivation — then the AC `sign_bit` bypass literal (the zero DC carries no sign). The
trace SHALL compose the §5.20.5.3 mode-info prefix, that luma residual, then the
all-zero U and V `txb_skip`, and SHALL prove the complete trace writes through one
in-tree AV2 §8.2 symbol encoder and decodes back through one symbol decoder with
shared CDF state. The trace assumes a transform-set configuration where §5.20.7.27's
`transform_type()` reads no `intra_tx_type` symbol (the DCT-only set or
`reduced_tx_set == 2` intra) AND `enable_intra_ist == 0` (else §5.20.7.29 reads a
`sec_tx_type` symbol before the base pass for an eob > 1 DCT_DCT block), consistent
with the plain DCT_DCT transform. It SHALL NOT emit the general `eob > 1`
`intra_tx_type` / `sec_tx_type` signaling, blocks with eob > 2,
higher-magnitude AC coefficients, chroma multi-coefficient blocks, partition syntax, tile payloads,
coded packets, public CLI success, or modes beyond the DC minimal tier.

#### Scenario: eob=2 trace orders the multi-coefficient residual

- **WHEN** the minimal eob=2 multi-coefficient luma block trace is composed
- **THEN** the trace SHALL be exactly the ordered mode tokens, the coded luma
  `all_zero` (0), `eob_pt_16` (1), the AC `coeff_base_eob` at context 1, the DC
  `coeff_base` at the derived low-frequency context 1, the AC `sign_bit` bypass, then
  the all-zero U and V `txb_skip` tokens — symbols `[0,0,0,0,1,0,0,0,1,1]`.

#### Scenario: The DC coeff_base context is derived, not hard-coded

- **WHEN** the eob=2 trace is composed
- **THEN** the DC `coeff_base` token's context SHALL be the value returned by the
  §8.3.2 low-frequency context derivation for the AC's `Level[]` (context 1), not a
  hard-coded literal.

#### Scenario: eob=2 trace roundtrips and does not produce packets

- **WHEN** the composed eob=2 trace is roundtripped through one in-tree AV2 §8.2 coder
- **THEN** the roundtrip SHALL be deterministic
- **AND** `Context::receive_packet` SHALL continue to return no coded packet
- **AND** no documentation or matrix row SHALL claim eob > 2, chroma
  multi-coefficient, or Baseline Encoder Profile v1 output from the trace alone.
