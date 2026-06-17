## ADDED Requirements

### Requirement: Reconstruct residual-addition step

The repository SHALL provide a scheduler-free `splot-recon` primitive for the AV2
§ 7.14.3 reconstruct residual-addition step, tracked by `RECON-RESIDUAL-ADDITION`.
The `reconstruct_add_residual` function SHALL compute, for each sample, the § 4.8
`Clip1(prediction[i] + residual[i])` (clamp to `0..=(2^BitDepth - 1)`), matching
`CurrFrame[plane][y + i][x + j] = Clip1(CurrFrame[plane][y + i][x + j] +
Residual[i][j])`, over a caller-supplied prediction block and signed residual.
The primitive SHALL validate the sample storage type against the active bit depth
and that the prediction, residual, and output lengths are equal, returning typed
`ReconError` values otherwise. The primitive SHALL sum with widened intermediates
so it is total and panic-free for every input, and the `Clip1` bound SHALL keep
every written sample within the validated storage type. The primitive SHALL read
no frame, segment, or tile state and SHALL NOT implement the § 7.15.4 2D inverse
transform that produces the residual, the § 7.14.4 dequantization process, the
§ 7.15.3 secondary transform, the DPCM adjustment, prediction-sample production,
tile syntax traversal, runtime decode output, or reference-refresh semantics.

#### Scenario: Residual addition succeeds with self-contained tests

- **WHEN** `cargo test -p splot-recon reconstruct --locked` runs
- **THEN** the test suite covers a plain addition, both `Clip1` clamp
  directions, the 10-bit `u16` path clamping to 1023, and the `i32` residual
  extremes
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: Invalid residual-addition input is typed

- **WHEN** callers pass a sample storage type that cannot represent the bit depth,
  or prediction/residual/output buffers of differing lengths
- **THEN** `splot-recon` returns a structured `ReconError`
- **AND** library code does not panic, overflow, unwrap, or emit `decode/*`
  diagnostics

#### Scenario: Full reconstruction remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the matrix records the residual-addition step as supported
- **AND** broader reconstruction remains partial until the § 7.15.4 2D inverse
  transform, the § 7.14.4 dequantization process, the § 7.15.3 secondary
  transform, and prediction/workspace integration are implemented and proven
