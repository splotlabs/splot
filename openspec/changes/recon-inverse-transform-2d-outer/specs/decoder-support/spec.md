## ADDED Requirements

### Requirement: 2D inverse transform outer process

The repository SHALL provide a scheduler-free `splot-recon` primitive for the AV2
§ 7.15.4 outer 2D inverse transform process, tracked by
`RECON-INVERSE-TRANSFORM-2D-OUTER`. The `inverse_transform_2d_outer` function
SHALL wrap the § 7.15.4.1 matrix transform core, deriving the adjusted operating
size (`1 << Min(log2, 5)`) and the original size (`1 << log2`) from the
caller-supplied original `txSz` log2 dimensions. When `lossless` and
`PlaneTxType` is `IDTX`, it SHALL produce `Residual = Dequant >> (3 - shift)`
with `shift = (pels > 256) + (pels > 1024)` (`pels` the adjusted `w * h`),
bypassing the matrix transform; otherwise it SHALL invoke the § 7.15.4.1 matrix
transform over the adjusted block. It SHALL then apply the § 7.15.4 DPCM
cumulative sum when requested (summing down columns for `V_PRED`, otherwise
across rows), and SHALL expand the adjusted block into the original-size residual
by sample duplication (nearest-neighbour 2x along any dimension whose original
size exceeds the adjusted size). The primitive SHALL validate the log2 shape
(each dimension `2..=6`, and both `2` when lossless) and that the dequantized
buffer is the adjusted `adjW * adjH` and the residual buffer is the original
`w * h`, returning typed `ReconError` values otherwise, and SHALL be panic-free
for valid shapes (a fixed 32x32 adjusted scratch, a shift in `1..=3`, and a
DPCM sum that cannot overflow). The primitive SHALL read no frame, segment, or
tile state and SHALL NOT implement the § 7.15.4 `Transform_Shift` or
`get_transform_1d_type` derivations (the caller resolves `rowType` / `colType` /
shifts), the § 7.15.3 secondary transform, the § 7.14.4 dequantization process,
residual addition, tile syntax traversal, runtime decode output, or
reference-refresh semantics.

#### Scenario: Outer 2D transform succeeds with self-contained tests

- **WHEN** `cargo test -p splot-recon inverse_transform_2d_outer --locked` runs
- **THEN** the test suite covers a no-adjustment case equal to the § 7.15.4.1
  core, the lossless IDTX bit-shift shortcut, vertical and horizontal DPCM
  running sums, and 64-wide and 64x64 sample duplication compared against the
  adjusted core
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: Sample duplication restores the original transform size

- **WHEN** a transform with a 64-sample dimension (original log2 `6`, adjusted
  `5`) is applied
- **THEN** the adjusted block (operating size at most 32) is expanded into the
  original-size residual so each adjusted sample is duplicated along the enlarged
  dimension(s)
- **AND** a 64x64 transform duplicates each adjusted sample into a 2x2 original
  block

#### Scenario: Invalid outer 2D transform input is typed

- **WHEN** callers pass a log2 dimension outside `2..=6`, a non-`(2, 2)` lossless
  shape, a dequant buffer that is not the adjusted `adjW * adjH`, or a residual
  buffer that is not the original `w * h`
- **THEN** `splot-recon` returns a structured `ReconError`
- **AND** library code does not panic, overflow, unwrap, or emit `decode/*`
  diagnostics

#### Scenario: Full reconstruction remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the matrix records the 2D inverse transform outer process as supported
- **AND** broader reconstruction remains partial until the § 7.15.4
  `Transform_Shift` / `get_transform_1d_type` derivations, the § 7.14.4
  dequantization process, the § 7.15.3 secondary transform, and
  prediction/workspace integration are implemented and proven
