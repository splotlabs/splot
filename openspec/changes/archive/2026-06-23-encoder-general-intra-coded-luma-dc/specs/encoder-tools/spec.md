## ADDED Requirements

### Requirement: First decodable coded-DC intra frame

`splot-encode` SHALL emit a complete, decodable AV2 IVF for one 64x64 all-intra
`OBU_CLOSED_LOOP_KEY` frame whose luma block carries a single coded DC coefficient (U and V
skipped), tracked by `ENC-GENERAL-INTRA-CODED-LUMA-DC`, via `emit_minimal_intra_coded_dc_ivf()`.
The luma DC SHALL be coded at the general `TX_64X64` contexts (`txb_skip == 0`, the `eob_pt_1024`
EOB symbol, `coeff_base_eob`, optional `coeff_br`, `dc_sign`), and decoding with `splot-decode`
SHALL reconstruct a flat luma plane carrying the dequantized residual. This is the encoder's
first decodable output with a coded coefficient; it is not a general encoder or Baseline Encoder
Profile v1.

#### Scenario: The emitted coded stream decodes to a flat 127 luma frame

- **WHEN** `emit_minimal_intra_coded_dc_ivf()` produces an IVF and `splot decode --output-format
  raw` decodes it
- **THEN** decoding SHALL succeed
- **AND** the decoded frame SHALL be 6144 bytes (8-bit 4:2:0 64x64)
- **AND** the luma plane SHALL be flat `127` (the `128` predictor plus the dequantized negative
  DC residual)
- **AND** the chroma planes SHALL be flat `128` (skipped).

#### Scenario: The coded luma tokens target the general EOB and transform contexts

- **WHEN** the coded luma DC tokens are emitted
- **THEN** the EOB symbol SHALL be `eob_pt_1024` (the 1024-position size class)
- **AND** the `txb_skip` and `coeff_base_eob` symbols SHALL use the `TX_64X64` `txSzCtx`.

#### Scenario: The bridge does not produce packets

- **WHEN** the coded-DC emitter is available
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
- **AND** no documentation or matrix row SHALL claim a general encoder or Baseline Encoder
  Profile v1 output from it.
