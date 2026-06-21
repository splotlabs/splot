## ADDED Requirements

### Requirement: First multi-coefficient intra frame

`splot-encode` SHALL emit a complete, decodable AV2 IVF for one 64x64 all-intra
`OBU_CLOSED_LOOP_KEY` frame whose luma block carries two scan positions — one nonzero AC
coefficient at scan index 1 and a zero DC (`eob == 2`, U and V skipped), tracked by
`ENC-GENERAL-INTRA-TWO-COEFF`, via `emit_minimal_intra_two_coeff_ivf()`. The residual SHALL be
coded at the general `TX_64X64` contexts (`txb_skip=0`, `eob_pt_1024=1`, the AC `coeff_base_eob`,
the DC non-EOB `coeff_base` at its `Level[]`-derived low-frequency context, then the AC
`sign_bit` § 8.2.5 bypass). Decoding with `splot-decode` SHALL validate the eob=2 entropy stream
and reconstruct the frame. This is the first multi-coefficient (`eob > 1`) frame; it is not a
general encoder or Baseline Encoder Profile v1.

#### Scenario: The emitted eob=2 stream decodes successfully

- **WHEN** `emit_minimal_intra_two_coeff_ivf()` produces an IVF and `splot decode
  --output-format raw` decodes it
- **THEN** decoding SHALL succeed (the § 8.2.4 `exit_symbol` check validates the AC
  `coeff_base` symbols) and the decoded frame SHALL be 6144 bytes
- **AND** the frame SHALL be flat `128` (the level-1 AC residual is sub-visible).

#### Scenario: The eob=2 stream is distinct from a skip frame

- **WHEN** the eob=2 IVF and the skip IVF are both emitted
- **THEN** their bytes SHALL differ (the eob=2 stream carries the AC coefficient symbols).
