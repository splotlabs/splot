## ADDED Requirements

### Requirement: eob 12-16 non-EOB high-frequency coefficients

The encoder coefficient tokenizer SHALL tokenize a general 4x4 DCT_DCT luma block with
eob 12 through 16, coding each non-EOB high-frequency coefficient (scan index ≥ 10,
`row+col ≥ 4`) with the high-frequency `coeff_base` table and context (`magLimit=3` with
no low-frequency near-DC carve-out and no DC special case, the `ctx2 + {0,5,10}` bands),
its level saturating at `NUM_BASE_LEVELS+1=3` (the 4-symbol table) with the high-frequency
`coeff_br` refinement and the magnitude cap of 5. It SHALL select each non-EOB
coefficient's low-frequency vs high-frequency `coeff_base` by `is_lf = (row+col < 4)`, route
the high-frequency selector through both §8.2 proof routers, and leave the low-frequency and
EOB-coefficient paths unchanged. This is a private, non-emitting stage tracked by
`ENC-COEFF-GENERAL-WALK-HF-MULTI`; it does not code golomb magnitudes, chroma, or produce
packets.

#### Scenario: an eob=12 block codes a non-EOB high-frequency coefficient

- **WHEN** an eob=12 block (one non-EOB HF coefficient at scan 10 plus the EOB HF
  coefficient at scan 11) is tokenized
- **THEN** the non-EOB HF coefficient emits the HF `coeff_base` selector and the
  low-frequency coefficients keep the LF selector
- **AND** the roundtrip recovers the exact block

#### Scenario: the full 4x4 scan roundtrips

- **WHEN** every eob 12-16 block over the coefficient positions and magnitude tiers is
  tokenized
- **THEN** each one roundtrips through the §8.2 coder and recovers its exact signed block
  with no unrouted CDF context

#### Scenario: the low-frequency path is unchanged

- **WHEN** an eob ≤ 11 block is tokenized
- **THEN** the emitted tokens are byte-identical to the prior brick
