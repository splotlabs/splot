## ADDED Requirements

### Requirement: First 2-D intra reconstruction

`splot-encode` SHALL emit a complete, decodable AV2 IVF for one 64x64 all-intra
`OBU_CLOSED_LOOP_KEY` frame whose luma block has `eob == 3` with two nonzero level-4 AC
coefficients — a negative vertical AC at scan index 1 and a positive horizontal AC at scan index
2, with a zero DC (U and V skipped), tracked by `ENC-GENERAL-INTRA-2D`, via
`emit_minimal_intra_2d_ivf()`. The two AC `sign_bit` § 8.2.5 bypasses SHALL be emitted in the AV2
§ 5.20.7.27 reverse-scan order (scan 2 then scan 1). Decoding with `splot-decode` SHALL
reconstruct a non-separable (2-D) luma plane. This is the first frame whose reconstruction varies
in both dimensions; it is not a general encoder or Baseline Encoder Profile v1.

#### Scenario: The 2-D block reconstructs a diagonal gradient

- **WHEN** `emit_minimal_intra_2d_ivf()` produces an IVF and `splot decode --output-format raw`
  decodes it
- **THEN** decoding SHALL succeed and the decoded frame SHALL be 6144 bytes
- **AND** the luma plane SHALL be non-separable (not every row is constant AND not every column is
  constant)
- **AND** the 3x3 luma band grid sampled at rows/columns {4, 32, 60} SHALL be
  `[[128,127,127],[129,128,127],[129,129,128]]`
- **AND** the U and V planes SHALL be flat 128.

#### Scenario: The 2-D stream is distinct from the single-AC eob=3 frame

- **WHEN** the 2-D IVF and the eob=3 IVF are both emitted
- **THEN** their bytes SHALL differ.
