## ADDED Requirements

### Requirement: First eob>2 intra frame

`splot-encode` SHALL emit a complete, decodable AV2 IVF for one 64x64 all-intra
`OBU_CLOSED_LOOP_KEY` frame whose luma block has `eob == 3` — a single nonzero level-4 AC at scan
index 2 (the horizontal frequency-1 position), with scan indices 1 and 0 zero (U and V skipped),
tracked by `ENC-GENERAL-INTRA-EOB3`, via `emit_minimal_intra_eob3_ivf()`. The EOB SHALL be coded
as `eob_pt_1024 == 2` (eobPt 3) followed by the `eob_extra` CDF symbol `0` (eob 3; the
`eob_extra_bit` bypass width is 0). Decoding with `splot-decode` SHALL reconstruct a horizontal
low-frequency cosine. This is the first frame with `eob > 2`; it is not a general encoder or
Baseline Encoder Profile v1.

#### Scenario: The eob=3 block reconstructs a horizontal cosine

- **WHEN** `emit_minimal_intra_eob3_ivf()` produces an IVF and `splot decode --output-format raw`
  decodes it
- **THEN** decoding SHALL succeed and the decoded frame SHALL be 6144 bytes
- **AND** every luma column SHALL be constant down its 64 rows
- **AND** the left 8 luma columns SHALL be 129, the middle 48 columns 128, and the right 8
  columns 127
- **AND** the U and V planes SHALL be flat 128.

#### Scenario: The eob=3 stream is distinct from the eob=2 visible-AC frame

- **WHEN** the eob=3 IVF and the visible-AC IVF are both emitted
- **THEN** their bytes SHALL differ.
