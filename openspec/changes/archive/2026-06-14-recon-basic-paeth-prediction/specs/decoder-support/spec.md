## ADDED Requirements

### Requirement: Basic PAETH intra prediction primitive

The repository SHALL provide a scheduler-free `splot-recon` primitive for the
AV2 §7.13.2.2 basic intra prediction process, tracked by
`RECON-INTRA-BASIC-PAETH-PREDICTION`. The primitive SHALL predict rectangular
regions from caller-provided prepared `LeftCol[0..h)`, `AboveRow[0..w)`,
and `AboveRow[-1]` samples, validate left edge length against `h`, validate
above edge length against `w`, validate all edge samples against the active
decoded bit depth, and return typed `ReconError` values instead of panicking on
invalid inputs. The primitive SHALL implement the §7.13.2.2 candidate selection
using `base = AboveRow[j] + LeftCol[i] - AboveRow[-1]` and the three absolute
differences `pLeft`, `pTop`, and `pTopLeft`. The primitive SHALL NOT decide
AV2 edge availability, MRL, tile-boundary, superblock, CfL, directional,
smooth, DIP, transform, dequantization, residual, runtime decode, output, or
reference-refresh semantics.

#### Scenario: Basic PAETH prediction succeeds with self-contained tests

- **WHEN** `cargo test -p splot-recon --locked` runs
- **THEN** the test suite covers rectangular basic/PAETH prediction cases that
  select the left, above, and top-left candidates
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: Invalid basic PAETH prediction input is typed

- **WHEN** callers provide wrong-length edge samples, an edge sample outside the
  active bit-depth range, a sample type that cannot represent the active bit
  depth, a too-small output stride, or a too-small output buffer
- **THEN** `splot-recon` returns a structured `ReconError`
- **AND** library code does not panic, unwrap, silently clamp invalid input, or
  emit `decode/*` diagnostics

#### Scenario: Workspace supports in-storage basic PAETH writes

- **WHEN** callers request basic/PAETH intra prediction into a workspace plane
  whose top-left, left, and above neighbors are inside workspace storage
- **THEN** the workspace validates the target rectangle, uses the in-storage
  neighbors as prepared edge samples, and writes the predicted rectangle into
  workspace storage
- **AND** if the target touches the top or left storage boundary, the workspace
  returns a typed reconstruction error instead of inventing AV2 fallback
  availability samples
- **AND** the helper does not decide AV2 block availability, MRL, tile-boundary,
  superblock, CfL, directional, smooth, DIP, transform, dequantization,
  residual, runtime decode, output, or reference-refresh semantics

#### Scenario: Full intra reconstruction remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the matrix records basic/PAETH intra prediction as supported
- **AND** broad scalar intra reconstruction remains partial until full
  `predict_intra()` dispatch, directional prediction, smooth prediction, data
  driven prediction, subsampled DC, IBP, transform syntax, dequantization,
  inverse transforms, residual addition, runtime hash output, runtime Y4M
  output, and reference refresh are implemented and proven
