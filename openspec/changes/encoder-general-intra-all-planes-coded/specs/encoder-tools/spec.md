## ADDED Requirements

### Requirement: All-planes-coded intra frame

`splot-encode` SHALL emit a complete, decodable AV2 IVF for one 64x64 all-intra
`OBU_CLOSED_LOOP_KEY` frame whose luma, U, and V blocks each carry a single coded DC
coefficient, tracked by `ENC-GENERAL-INTRA-ALL-PLANES-CODED`, via
`emit_minimal_intra_all_planes_coded_ivf()`. Because the U plane is coded (`EobU != 0`), the V
`txb_skip` SHALL use the § 8.3.2 context `6`. Decoding with `splot-decode` SHALL reconstruct
coded residual on every plane. This mirrors the q80 fixture's all-three-planes-coded structure
with sub-golomb magnitudes; it is not byte-exact q80, a general encoder, or Baseline Encoder
Profile v1.

#### Scenario: The emitted all-planes-coded stream decodes with residual on every plane

- **WHEN** `emit_minimal_intra_all_planes_coded_ivf()` produces an IVF and `splot decode
  --output-format raw` decodes it
- **THEN** decoding SHALL succeed and the decoded frame SHALL be 6144 bytes
- **AND** every plane SHALL be flat `127` (the dequantized negative coded DC residual).

#### Scenario: The bridge does not produce packets

- **WHEN** the all-planes-coded emitter is available
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet.
