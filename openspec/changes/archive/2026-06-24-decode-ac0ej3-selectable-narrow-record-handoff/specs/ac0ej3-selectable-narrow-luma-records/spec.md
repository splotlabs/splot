## ADDED Requirements

### Requirement: ac0ej3 narrow luma leaves use actual extents

The decoder SHALL keep
`DECODE-AC0EJ3-SELECTABLE-NARROW-LUMA-RECORDS` as the prerequisite row for
admitting luma-only narrow selectable leaves in the local ac0ej3 Wiener NS LR
path. For the admitted narrow luma subset, transform-record derivation SHALL
consume partition syntax and SHALL record the actual luma leaf width and height
in 4x4 units only when applying the consumed partition would produce empty
geometry. It SHALL preserve the existing fail-closed guard for chroma-bearing
narrow leaves and SHALL not admit unobserved transposed narrow shapes.

#### Scenario: Observed 8x32 luma-only leaf is retained

- **WHEN** the selectable transform-record path reaches the observed luma-only
  `BLOCK_8X32` leaf
- **THEN** the retained transform record has the leaf's actual 2x8 4x4 extent
- **AND** subsequent residual and `LrTxSkip` derivation use that extent

#### Scenario: Observed luma-only chroma-offset 4x32 leaf is retained

- **WHEN** the selectable transform-record path reaches the observed
  chroma-offset `BLOCK_4X32` leaf with `has_chroma == false`
- **THEN** the retained transform record has the leaf's actual 1x8 4x4 extent
- **AND** chroma residual coordinate handoff remains out of scope

#### Scenario: Chroma-bearing narrow leaf remains unsupported

- **WHEN** a narrow selectable leaf or chroma-offset leaf also carries chroma
  residual syntax
- **THEN** the runtime rejects the path with a structured unsupported-feature
  diagnostic
- **AND** it does not reuse the luma-only fallback for chroma coordinates
