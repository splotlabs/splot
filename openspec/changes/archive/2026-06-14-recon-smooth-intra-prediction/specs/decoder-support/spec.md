## ADDED Requirements

### Requirement: Smooth intra prediction primitive

The repository SHALL provide a scheduler-free `splot-recon` primitive for the
AV2 §7.13.2.13 smooth intra prediction process, tracked by
`RECON-INTRA-SMOOTH-PREDICTION`. The primitive SHALL predict rectangular
regions for `SMOOTH_PRED`, `SMOOTH_V_PRED`, and `SMOOTH_H_PRED` from
caller-provided prepared `LeftCol[0..h]` and `AboveRow[0..w]` samples,
including the `LeftCol[h]` bottom-left and `AboveRow[w]` top-right sentinel
samples. The primitive SHALL validate left edge length against `h + 1`,
validate above edge length against `w + 1`, validate all edge samples and
computed output samples against the active decoded bit depth, validate output
stride and length, and return typed `ReconError` values instead of panicking on
invalid inputs. The primitive SHALL implement the §7.13.2.13 formulas using
AV2 §3 `BLEND_WEIGHT_MAX = 32` and AV2 §4.8 `Round2`. The primitive SHALL NOT
decide AV2 edge availability, MRL, tile-boundary, superblock, CfL,
directional, PAETH, DIP, transform, dequantization, residual, runtime decode,
output, or reference-refresh semantics.

#### Scenario: Smooth prediction succeeds with self-contained tests

- **WHEN** `cargo test -p splot-recon --locked` runs
- **THEN** the test suite covers rectangular smooth prediction for
  `SMOOTH_PRED`, `SMOOTH_V_PRED`, and `SMOOTH_H_PRED`
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: Invalid smooth prediction input is typed

- **WHEN** callers provide wrong-length edge samples, an edge sample outside the
  active bit-depth range, a sample type that cannot represent the active bit
  depth, a too-small output stride, a too-small output buffer, or a computed
  prediction outside the active bit depth
- **THEN** `splot-recon` returns a structured `ReconError`
- **AND** library code does not panic, unwrap, silently clamp invalid input, or
  emit `decode/*` diagnostics

#### Scenario: Workspace supports in-storage smooth writes

- **WHEN** callers request smooth intra prediction into a workspace plane whose
  left, above, bottom-left, and top-right prepared samples are inside workspace
  storage
- **THEN** the workspace validates the target rectangle, uses those in-storage
  samples as prepared edge inputs, and writes the predicted rectangle into
  workspace storage
- **AND** if any required prepared edge or sentinel sample is outside workspace
  storage, the workspace returns a typed reconstruction error instead of
  inventing AV2 fallback availability samples
- **AND** the helper does not decide AV2 block availability, MRL, tile-boundary,
  superblock, CfL, directional, PAETH, DIP, transform, dequantization,
  residual, runtime decode, output, or reference-refresh semantics

#### Scenario: Full intra reconstruction remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the matrix records smooth intra prediction as supported
- **AND** broad scalar intra reconstruction remains partial until full
  `predict_intra()` dispatch, directional prediction, data driven prediction,
  subsampled DC, IBP, transform syntax, dequantization, inverse transforms,
  residual addition, runtime hash output, runtime Y4M output, and reference
  refresh are implemented and proven
