## ADDED Requirements

### Requirement: Matrix-free 1D inverse transforms

The repository SHALL provide scheduler-free `splot-recon` primitives for the AV2
§ 7.15.2.2 inverse Walsh-Hadamard transform and the § 7.15.2.3 inverse identity
transform, tracked by `RECON-INVERSE-TRANSFORM-MATRIX-FREE`, extending the
§ 7.15.2 1D inverse transform module. The repository SHALL provide an
`inverse_walsh_hadamard` function implementing the § 7.15.2.2 4-element lossless
butterfly over a fixed `[i32; 4]` input with a pre-scaling shift, applying no
`Clip3`, and an `inverse_identity_transform` function implementing § 7.15.2.3 as
a per-sample `Clip3(-(1 << (BitDepth + (colTx ? 0 : 7))), ... - 1,
Round2(src[i] * scale, shift))` over a caller-supplied scale. Both primitives
SHALL use widened intermediates so they are total and panic-free for every
input, SHALL read no frame, segment, or tile state, and the identity transform
SHALL return a typed `ReconError` on a source/output length mismatch. The
primitives SHALL NOT implement the § 7.15.4.1 `get_identity_scale` derivation,
the § 7.15.3 secondary transform, the § 7.15.4 2D inverse transform
orchestration, dequantization, residual addition, tile syntax traversal, runtime
decode output, or reference-refresh semantics.

#### Scenario: Matrix-free transforms succeed with self-contained tests

- **WHEN** `cargo test -p splot-recon inverse_transform --locked` runs
- **THEN** the test suite covers the Walsh-Hadamard butterfly (DC-only and
  last-coefficient inputs plus the pre-shift case) and the identity transform
  (scale/round/clamp, both `colTx` ranges)
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: Invalid identity transform input is typed

- **WHEN** callers pass an output buffer whose length differs from the source
  length to the identity transform
- **THEN** `splot-recon` returns a structured `ReconError`
- **AND** library code does not panic, overflow, unwrap, or emit `decode/*`
  diagnostics

#### Scenario: Full inverse transform remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the matrix records the matrix-free 1D inverse transforms as supported
- **AND** broader reconstruction remains partial until the § 7.15.3 secondary
  transform, the § 7.15.4 2D inverse transform orchestration, dequantization, and
  residual addition are implemented and proven
