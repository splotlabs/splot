## ADDED Requirements

### Requirement: eob 3-4 coefficient walk with eob_extra

The encoder coefficient tokenizer SHALL tokenize a general low-frequency 4x4 DCT_DCT
luma block with eob 3 or 4, emitting `eob_pt_16` with symbol `eobPt - 1` and, for
eobPt ≥ 3, the §5.20.7.27 `eob_extra` CDF flag = `eob - 3` (with no `eob_extra_bit`
literals at eobPt 3), then walking the 3-4 coefficients with the §8.3.2
low-frequency contexts. It SHALL reject a nonzero coefficient at scan index ≥ 4 with
a typed error. The recovery helper SHALL read the `eob_extra` flag to reconstruct the
eob. The 4x4 low-frequency CDF routing SHALL be free of single-context holes across
the full generated context dimension. This is a private, non-emitting stage tracked
by `ENC-COEFF-GENERAL-WALK-EOB-EXTRA`; it does not code eob > 4, golomb magnitudes,
high-frequency or chroma coefficients, or produce packets.

#### Scenario: an eob=3 block emits the eob_extra flag 0

- **WHEN** an eob=3 block is tokenized
- **THEN** `eob_pt_16` carries symbol 2 and the `eob_extra` flag is 0
- **AND** the roundtrip recovers the exact block

#### Scenario: an eob=4 block emits the eob_extra flag 1

- **WHEN** an eob=4 block is tokenized
- **THEN** `eob_pt_16` carries symbol 2 and the `eob_extra` flag is 1
- **AND** the roundtrip recovers the exact block

#### Scenario: every in-scope block routes hole-free

- **WHEN** every eob 3-4 block over all coefficient positions and base/coeff_br
  magnitude tiers is tokenized
- **THEN** each one roundtrips through the §8.2 coder and recovers its exact signed
  block (no unrouted CDF context)

#### Scenario: eob beyond 4 is rejected

- **WHEN** a nonzero coefficient sits at scan index ≥ 4
- **THEN** the tokenizer returns a typed unsupported-eob error without panicking
