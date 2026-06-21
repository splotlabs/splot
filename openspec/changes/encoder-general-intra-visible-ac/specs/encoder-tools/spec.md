## ADDED Requirements

### Requirement: First visibly non-flat intra reconstruction

`splot-encode` SHALL emit a complete, decodable AV2 IVF for one 64x64 all-intra
`OBU_CLOSED_LOOP_KEY` frame whose luma block carries a single nonzero level-4 AC coefficient at
scan index 1 with a zero DC (`eob == 2`, U and V skipped), tracked by
`ENC-GENERAL-INTRA-VISIBLE-AC`, via `emit_minimal_intra_visible_ac_ivf()`. The AC SHALL be coded
at the general `TX_64X64` contexts (`txb_skip=0`, `eob_pt_1024=1`, `coeff_base_eob` symbol 3 with
no `coeff_br`, the DC non-EOB `coeff_base` at its `Level[]`-derived low-frequency context, then
the AC `sign_bit` § 8.2.5 bypass). Decoding with `splot-decode` SHALL reconstruct a visibly
non-flat luma plane. This is the first frame where a coefficient shapes the reconstruction; it is
not a general encoder or Baseline Encoder Profile v1.

#### Scenario: The level-4 AC reconstructs a vertical cosine

- **WHEN** `emit_minimal_intra_visible_ac_ivf()` produces an IVF and `splot decode
  --output-format raw` decodes it
- **THEN** decoding SHALL succeed and the decoded frame SHALL be 6144 bytes
- **AND** every luma row SHALL be constant across its 64 columns
- **AND** the top 8 luma rows SHALL be 129, the middle 48 rows 128, and the bottom 8 rows 127
- **AND** the U and V planes SHALL be flat 128.

#### Scenario: The visible-AC stream is distinct from the sub-visible level-1 frame

- **WHEN** the visible-AC IVF and the level-1 eob=2 IVF are both emitted
- **THEN** their bytes SHALL differ.
