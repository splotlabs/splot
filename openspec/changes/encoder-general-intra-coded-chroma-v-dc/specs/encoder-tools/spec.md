## ADDED Requirements

### Requirement: Decodable coded-V intra frame

`splot-encode` SHALL emit a complete, decodable AV2 IVF for one 64x64 all-intra
`OBU_CLOSED_LOOP_KEY` frame whose chroma V block carries a single coded DC coefficient (luma
and U skipped), tracked by `ENC-GENERAL-INTRA-CODED-CHROMA-V-DC`, via
`emit_minimal_intra_coded_chroma_v_ivf()`. The V DC SHALL be coded at the general `TX_32X32`
chroma contexts (`VTxbSkip == 0` at the neutral `EobU == 0` context, `eob_pt_1024` at chroma
`eob_ctx 2`, `coeff_base_eob`, then the V DC `sign_bit` § 8.2.5 bypass literal). With the U and
V coded frames this completes the per-plane coded-residual set. It is not a general encoder or
Baseline Encoder Profile v1.

#### Scenario: The emitted coded-V stream decodes with the residual on V only

- **WHEN** `emit_minimal_intra_coded_chroma_v_ivf()` produces an IVF and `splot decode
  --output-format raw` decodes it
- **THEN** decoding SHALL succeed and the decoded frame SHALL be 6144 bytes
- **AND** the luma plane SHALL be flat `128` (skipped)
- **AND** the U plane SHALL be flat `128` (skipped)
- **AND** the V plane SHALL be flat `127` (the dequantized negative chroma DC residual).

#### Scenario: The bridge does not produce packets

- **WHEN** the coded-V emitter is available
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet.
