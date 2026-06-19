## ADDED Requirements

### Requirement: Transform-block reconstruction residual chain

The repository SHALL provide a scheduler-free `splot-recon` composition that
reconstructs one transform block from its decoded quantized coefficients,
tracked by `RECON-RECONSTRUCT-TRANSFORM-BLOCK`. The
`reconstruct_transform_block_residual<T: ReconSample>(prediction, quant,
dequant_params, transform, dequant_scratch, residual_scratch, out)` function
SHALL compose the AV2 residual chain `out = Clip1(prediction +
inverse_transform(dequant(quant)))` by invoking § 7.14.4 `dequantize_block`, then
§ 7.15.4 `inverse_transform_2d_outer`, then § 7.14.3 `reconstruct_add_residual`,
over caller-resolved dequantization and transform parameters. It SHALL use the
caller-owned `dequant_scratch` and `residual_scratch` buffers and allocate
nothing, SHALL be total and panic-free for consistent inputs, and SHALL reject
any buffer-length or transform-geometry inconsistency by propagating the first
underlying typed `ReconError` before `out` is mutated, without adding a new error
variant. The composition SHALL read no frame, segment, or tile state and SHALL
NOT implement the coefficient entropy decode that produces `Quant`, the § 7.15.3
secondary transform, the § 7.14.4 `useQm` or shift derivation, the § 7.15.4
DPCM-direction selection, prediction sample production, runtime decode wiring,
output, or reference refresh.

#### Scenario: Reconstruction chain succeeds with self-contained tests

- **WHEN** `cargo test -p splot-recon reconstruct_block --locked` runs
- **THEN** the test suite covers an all-zero quantized block reproducing the
  prediction exactly and a single nonzero DC coefficient producing a flat,
  signed residual at TX_4X4 and at TX_64X64 (the latter exercising the
  adjusted-to-original sample duplication)
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: Inconsistent buffers are rejected fail-atomically

- **WHEN** `reconstruct_transform_block_residual` is called with a
  `dequant_scratch` length that disagrees with `dequant_params.tx_width *
  dequant_params.tx_height`
- **THEN** it returns the underlying `ReconError` from the dequantization step
  and leaves `out` untouched
