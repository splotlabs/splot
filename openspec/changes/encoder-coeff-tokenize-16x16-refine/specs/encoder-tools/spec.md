## ADDED Requirements

### Requirement: 16x16 full-range coefficient tokenization (eob_pt_extra)

The encoder SHALL tokenize an arbitrary 16×16 DCT_DCT luma block over the full eob range
(1..=256, eobPt 1..=9) per AV2 §5.20.7.27, emitting `eob_pt_256` symbol 7 for both eobPt 8
and eobPt 9 and a 1-bit `eob_pt_extra` literal (`eobPt-8`) after the symbol and before
`eob_extra`, mirroring the decoder's `resolved_eob_pt`/`read_nonzero_coeff_eob`. The 4×4 path
and the 16×16 base pass SHALL remain byte-identical. This is tracked by
`ENC-COEFF-TOKENIZE-16X16-REFINE`.

#### Scenario: high-eob 16x16 blocks roundtrip

- **WHEN** 16×16 blocks reaching eobPt 8 (eob 96) and eobPt 9 (eob 200, and 256) are tokenized
- **THEN** each roundtrips through one §8.2 coder with `eob_pt_256` symbol 7 and the correct
  `eob_pt_extra` bit (0 / 1)

#### Scenario: the base pass is unchanged

- **WHEN** an eob-32 block is tokenized through the full entry
- **THEN** the token stream is identical to the base-pass entry (no behaviour change ≤ eob 32)
