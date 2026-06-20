## ADDED Requirements

### Requirement: First decodable coded-chroma intra frame

`splot-encode` SHALL emit a complete, decodable AV2 IVF for one 64x64 all-intra
`OBU_CLOSED_LOOP_KEY` frame whose chroma U block carries a single coded DC coefficient (luma
and V skipped), tracked by `ENC-GENERAL-INTRA-CODED-CHROMA-DC`, via
`emit_minimal_intra_coded_chroma_ivf()`. The U DC SHALL be coded at the general `TX_32X32`
chroma contexts (`txb_skip == 0`, `eob_pt_1024` at chroma `eob_ctx 2`, `coeff_base_eob`, then
the U DC `sign_bit` § 8.2.5 bypass literal), with the V `txb_skip` at the `EobU != 0` context.
This is the encoder's first decodable output with a coded chroma coefficient; it is not a
general encoder or Baseline Encoder Profile v1.

#### Scenario: The emitted coded-chroma stream decodes with the residual on U only

- **WHEN** `emit_minimal_intra_coded_chroma_ivf()` produces an IVF and `splot decode
  --output-format raw` decodes it
- **THEN** decoding SHALL succeed and the decoded frame SHALL be 6144 bytes
- **AND** the luma plane SHALL be flat `128` (skipped)
- **AND** the U plane SHALL be flat `127` (the dequantized negative chroma DC residual)
- **AND** the V plane SHALL be flat `128` (skipped).

#### Scenario: The bridge does not produce packets

- **WHEN** the coded-chroma emitter is available
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet.
