## ADDED Requirements

### Requirement: eob 5-10 coefficient walk with eob_extra_bit literals

The encoder coefficient tokenizer SHALL tokenize a general low-frequency 4x4 DCT_DCT
luma block with eob 5 through 10, emitting `eob_pt_16` with symbol `eobPt - 1` (eobPt
4 for eob 5-8, eobPt 5 for eob 9-10), the §5.20.7.27 `eob_extra` CDF flag, and
`eobPt - 3` `eob_extra_bit` §8.2.5 bypass literals in MSB-first order (mirroring the
decoder `read_literal`), then walking the low-frequency coefficients with the §8.3.2
contexts. It SHALL reject a nonzero coefficient at scan index ≥ 10 with a typed error.
The recovery helper SHALL read the `eob_extra` flag and the bypass literals back in
the same order to reconstruct the eob. This is a private, non-emitting stage tracked
by `ENC-COEFF-GENERAL-WALK-EOB-EXTRA-BITS`; it does not code high-frequency or chroma
coefficients, golomb magnitudes, or produce packets.

#### Scenario: an eobPt-4 block emits one eob_extra_bit

- **WHEN** an eob=6 block is tokenized
- **THEN** `eob_pt_16` carries symbol 3, the `eob_extra` flag is 0, and one
  `eob_extra_bit` bypass literal carries 1
- **AND** the roundtrip recovers the exact block

#### Scenario: an eobPt-5 block emits two eob_extra_bits MSB-first

- **WHEN** an eob=10 block is tokenized
- **THEN** `eob_pt_16` carries symbol 4, the `eob_extra` flag is 0, and the two
  `eob_extra_bit` bypass literals are `[0, 1]` (MSB-first)
- **AND** the roundtrip recovers the exact block

#### Scenario: every in-scope refined eob routes hole-free

- **WHEN** every eob 5-10 block over the low-frequency scan positions and base/coeff_br
  magnitude tiers is tokenized
- **THEN** each one roundtrips through the §8.2 coder and recovers its exact signed
  block with no unrouted CDF context and no added CDF rows

#### Scenario: a high-frequency eob is rejected

- **WHEN** a nonzero coefficient sits at scan index ≥ 10 (the first high-frequency
  position)
- **THEN** the tokenizer returns a typed unsupported-eob error without panicking
