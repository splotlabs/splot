# coeff-eob-size-context Specification

## Purpose

Define the completed OpenSpec requirements for `coeff-eob-size-context`.

## Requirements
### Requirement: Derive coefficient EOB size class
The decoder coefficient-loop boundary SHALL derive the nonzero coefficient EOB
point CDF size class from caller-resolved `Tx_Width_Log2[txSz]` and
`Tx_Height_Log2[txSz]` values according to AV2 § 5.20.7.27.

#### Scenario: Valid transform log2 dimensions select EOB CDF families
- **WHEN** the caller provides transform width/height log2 values whose clamped
  `eobMultisize` values are 0 through 6
- **THEN** the helper returns the corresponding `EobPtSize` family from `Pt16`
  through `Pt1024`

#### Scenario: Oversized transform log2 dimensions are clamped
- **WHEN** the caller provides width and height log2 values greater than 5
- **THEN** the helper applies the spec `Min(..., 5)` clamp and selects `Pt1024`

#### Scenario: Invalid transform log2 dimensions are rejected
- **WHEN** the caller provides a width or height log2 value below 2
- **THEN** the helper returns a typed coefficient-loop context error before
  deriving an EOB CDF size class

### Requirement: Derive coefficient EOB plane context
The decoder coefficient-loop boundary SHALL derive `eobCtx` as 2 for chroma
planes and as `is_inter` for luma, matching AV2 § 5.20.7.27 and § 8.3.2.

#### Scenario: Luma context follows inter flag
- **WHEN** the caller provides plane 0 with intra or inter block state
- **THEN** the helper returns `eobCtx` 0 for intra and 1 for inter

#### Scenario: Chroma context overrides inter flag
- **WHEN** the caller provides any plane value greater than 0
- **THEN** the helper returns `eobCtx` 2 regardless of the `is_inter` value

### Requirement: Build EOB symbol-reader input
The decoder coefficient-loop boundary SHALL compose the derived EOB size class,
derived `eobCtx`, and caller-provided `coeff_cdf_q_ctx` into the existing
nonzero EOB symbol-reader input type.

#### Scenario: Derived input preserves caller quantization context
- **WHEN** the helper receives valid transform log2 dimensions, plane/inter facts,
  and a coefficient CDF quantization context
- **THEN** the returned symbol-reader input uses the derived size/context and the
  original `coeff_cdf_q_ctx`
