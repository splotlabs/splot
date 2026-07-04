# local decoder mission Selectable Narrow Luma Records Specification

## Purpose
Define the fail-closed local decoder mission Wiener NS loop-restoration frontier that admits
luma-only narrow selectable transform-record leaves while keeping chroma-bearing
narrow records unsupported until separately proven.

## Requirements

### Requirement: local decoder mission Selectable Narrow Luma Records

The decoder SHALL track `DECODE-SELECTABLE-NARROW-LUMA-RECORDS` as a
partial runtime prerequisite for the local decoder mission Wiener NS LR path. When the local
stream's selectable transform-record handoff reaches a supported luma-only SDP
or narrow selectable leaf with valid nonzero luma dimensions, including the
observed `BLOCK_8X32` and luma-only chroma-offset `BLOCK_4X32` cases, the
runtime SHALL consume AV2 §5.20.6.3 partition syntax and record the leaf's
actual 4x4-grid extent when applying the consumed partition would produce empty
geometry. It SHALL consume §5.20.7.27 luma coefficient syntax needed to derive
`LrTxSkip`, without requiring chroma syntax for that leaf. For this bounded
actual-extent subset, the runtime SHALL not fabricate max-rectangle transform
cells outside the leaf. The runtime SHALL remain fail-closed before decoded
sample population or output.

#### Scenario: Luma-only narrow leaves advance the live frontier

- **WHEN** the local decoder mission stream reaches a supported luma-only narrow
  selectable transform-record leaf
- **THEN** the runtime consumes the luma mode and luma coefficient syntax needed
  for its `LrTxSkip` transform records
- **AND** it no longer emits
  `unsupported_wienerns_lr_selectable_transform_records_block_shape` or
  `unsupported_wienerns_lr_selectable_transform_records_empty_transform` for the
  supported leaf
- **AND** it stops at the next structured unsupported frontier before output

### Requirement: local decoder mission narrow luma leaves use actual extents

The decoder SHALL keep
`DECODE-SELECTABLE-NARROW-LUMA-RECORDS` as the prerequisite row for
admitting luma-only narrow selectable leaves in the local decoder mission Wiener NS LR
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

#### Scenario: Chroma claims remain excluded

- **WHEN** luma-only narrow selectable transform records have been consumed
- **THEN** the decoder SHALL NOT claim narrow chroma prediction, CfL prediction,
  decoded chroma samples, decoded `CurrFrame` or `CdefFrame` samples,
  `FilterClass` retention, loop-restoration filtering/output, reference
  refresh, AVM/dav2d byte equality, or successful local decoder mission decode
