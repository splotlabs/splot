## ADDED Requirements

### Requirement: 16x16 luma DC coefficient tokenization

The encoder SHALL tokenize a single coded DC coefficient (eob=1) in a 16×16 DCT_DCT luma
block per AV2 §5.20.7.27: `txb_skip=0` at `TX_SIZE_16X16_CTX` (=2), `eob_pt_256` symbol 0,
the low-frequency `coeff_base_eob` at `TX_SIZE_16X16_CTX` DC context 0, optional `coeff_br`,
and `dc_sign`. The `TX_SIZE_16X16_CTX` value and the `eob_pt_256` family SHALL mirror the
decoder, and the new selector + banks SHALL be routed in both CDF routers. This is a private,
non-emitting stage tracked by `ENC-COEFF-TOKENIZE-16X16-DC`; it does not code eob>1 or emit a
packet.

#### Scenario: a 16x16 DC roundtrips through the §8.2 coder

- **WHEN** a 16×16 luma block with a single asymmetric coded DC is tokenized
- **THEN** the token stream roundtrips through one §8.2 coder, decoding back to the exact
  symbol sequence (`eob_pt_256` symbol 0, the LF DC `coeff_base_eob`, `dc_sign`)
