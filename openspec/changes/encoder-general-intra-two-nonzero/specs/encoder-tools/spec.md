## ADDED Requirements

### Requirement: First two-nonzero-coefficient intra block

`splot-encode` SHALL emit a complete, decodable AV2 IVF for one 64x64 all-intra
`OBU_CLOSED_LOOP_KEY` frame whose luma block carries two nonzero coefficients — a level-4 AC at
scan index 1 and a level-1 DC at scan index 0 (`eob == 2`, U and V skipped), tracked by
`ENC-GENERAL-INTRA-TWO-NONZERO`, via `emit_minimal_intra_two_nonzero_ivf()`. The block SHALL be
coded at the general `TX_64X64` contexts: the base pass (`txb_skip=0`, `eob_pt_1024=1`, AC
`coeff_base_eob`, DC non-EOB `coeff_base`) then the sign pass in scan order (the DC `dc_sign` CDF,
then the AC `sign_bit` § 8.2.5 bypass). Decoding with `splot-decode` SHALL reconstruct the
visible-AC cosine superimposed on a DC offset. This is the first block with more than one nonzero
coefficient; it is not a general encoder or Baseline Encoder Profile v1.

#### Scenario: The two-nonzero block reconstructs a cosine plus DC offset

- **WHEN** `emit_minimal_intra_two_nonzero_ivf()` produces an IVF and `splot decode
  --output-format raw` decodes it
- **THEN** decoding SHALL succeed and the decoded frame SHALL be 6144 bytes
- **AND** every luma row SHALL be constant across its 64 columns
- **AND** the top 14 luma rows SHALL be 129 and the remaining 50 rows 128
- **AND** the U and V planes SHALL be flat 128.

#### Scenario: The two-nonzero stream is distinct from the single-nonzero visible-AC frame

- **WHEN** the two-nonzero IVF and the visible-AC IVF are both emitted
- **THEN** their bytes SHALL differ.
