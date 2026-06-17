## ADDED Requirements

### Requirement: 2D matrix inverse transform core

The repository SHALL provide a scheduler-free `splot-recon` primitive for the AV2
§ 7.15.4.1 2D matrix transform, tracked by `RECON-INVERSE-TRANSFORM-2D`. The
`inverse_transform_2d` function SHALL apply the § 7.15.4.1 row-then-column 2D
matrix transform to a caller-supplied dequantized coefficient block, writing the
residual block, dispatching each pass to the § 7.15.2.2 Walsh-Hadamard transform
(lossless), the § 7.15.2.3 identity transform, or the § 7.15.2.1 kernel
transform. The parameters SHALL carry the original (unadjusted) `txSz` log2
dimensions `log2W` / `log2H` (each `2..=6`), and the primitive SHALL derive the
adjusted operating dimensions as `1 << Min(log2, 5)` per the `Adjusted_Tx_Size`
table. The primitive SHALL compute the § 7.15.4.1 `Round2(Dequant * 2896, 12)`
√2 rescale when `Abs(log2W - log2H)` is odd, and the `get_identity_scale` value
for each pass, from the original log2 dimensions, so transforms with a 64-sample
dimension rescale correctly. The primitive SHALL validate the log2 shape (each
dimension `2..=6`, and both `2` when lossless) and that the dequantized and
residual buffer lengths are each exactly `w * h`, returning typed `ReconError`
values otherwise, and SHALL be panic-free for valid shapes via fixed
32x32-bounded buffers and total 1D primitives. The primitive SHALL read no frame,
segment, or tile state and SHALL NOT implement the § 7.15.4 outer process (the
`Adjusted_Tx_Size` lookup itself, the `Transform_Shift` / `get_transform_1d_type`
derivations, the `Lossless && IDTX` bit-shift shortcut, the DPCM cumulative sum,
or the adjusted-size sample duplication), the § 7.15.3 secondary transform, the
§ 7.14.4 dequantization process, residual addition, tile syntax traversal,
runtime decode output, or reference-refresh semantics.

#### Scenario: 2D matrix transform succeeds with self-contained tests

- **WHEN** `cargo test -p splot-recon inverse_transform_2d --locked` runs
- **THEN** the test suite covers DC-only DCT flat fields (4x4 and 8x8), the
  lossless 4x4 Walsh-Hadamard vector, identity position preservation, the
  rectangular 4x8 rescale path, and a mixed row-DCT/column-identity
  energy-confinement case
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: Rescale parity uses the original transform dimensions

- **WHEN** a transform with a 64-sample dimension (original log2 `(6, 5)`, e.g.
  TX_64X32) is applied
- **THEN** the `Round2(Dequant * 2896, 12)` √2 rescale fires because
  `Abs(log2W - log2H)` is odd for the original dimensions, matching a manually
  pre-rescaled 32x32 block
- **AND** the result differs from the same 32x32 block fed the un-rescaled
  coefficients

#### Scenario: Invalid 2D transform input is typed

- **WHEN** callers pass a log2 dimension outside `2..=6`, a non-`(2, 2)` lossless
  shape, or dequant/residual buffers whose length is not `w * h`
- **THEN** `splot-recon` returns a structured `ReconError`
- **AND** library code does not panic, overflow, unwrap, or emit `decode/*`
  diagnostics

#### Scenario: Full reconstruction remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the matrix records the 2D matrix transform core as supported
- **AND** broader reconstruction remains partial until the § 7.15.4 outer
  orchestration, the § 7.14.4 dequantization process, the § 7.15.3 secondary
  transform, and prediction/workspace integration are implemented and proven
