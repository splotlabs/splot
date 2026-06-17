## ADDED Requirements

### Requirement: Kernel-based 1D inverse transform primitive

The repository SHALL provide a scheduler-free `splot-recon` primitive for the
AV2 § 7.15.2.1 kernel-based 1D inverse transform, tracked by
`RECON-INVERSE-TRANSFORM-1D`, consuming the shared § 9.6 transform kernels from
the dependency-free `splot-tables` crate. The primitive SHALL provide an
`InverseTransform1dType` modeling the § 7.15.4.1 Table 7.1 kernel types
`Dct`, `Adst`, `Fdst`, `Ddtx`, and `Fddt` (the `IDT` identity transform is the
separate § 7.15.2.3 process and is excluded), and an `inverse_transform_1d`
function that matrix-multiplies the input coefficients by the size-and-type
kernel and then applies AV2 § 4.8 `Round2` and the § 7.15.2.1
`colTx`-dependent `Clip3(-(1 << (BitDepth + (colTx ? 0 : 7))), ... - 1, .)`.
The primitive SHALL reproduce the § 7.15.2.1 dispatch exactly: length-4 routes
`Fdst`/`Ddtx`/`Fddt` to the FDST kernel, length-32 uses the DCT kernel for every
type, and `Fddt` indexes the DDTX kernel column in reverse. The primitive SHALL
accept lengths 4, 8, 16, and 32, SHALL accumulate with widened intermediates so
in-range inputs do not overflow and the result stays within the clamp bound, and
SHALL return typed `ReconError` values for unsupported lengths and for
output/source length mismatches instead of panicking. The primitive SHALL NOT
implement the § 7.15.2.2 Walsh-Hadamard transform, the § 7.15.2.3 identity
transform, the § 7.15.3 secondary transform, the § 7.15.4 2D inverse transform
orchestration, dequantization, residual addition, tile syntax traversal, runtime
decode output, or reference-refresh semantics.

#### Scenario: 1D inverse transform succeeds with self-contained tests

- **WHEN** `cargo test -p splot-recon inverse_transform --locked` runs
- **THEN** the test suite covers a DC-only flat field, a single-coefficient
  kernel row, the § 4.8 arithmetic downshift, both `colTx` clamp ranges, the
  `Fddt`-reverses-`Ddtx` property, the length-4 FDST fallback, and the length-32
  DCT-for-every-type property
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: Invalid 1D inverse transform input is typed

- **WHEN** callers pass a source length other than 4, 8, 16, or 32, or an output
  buffer whose length differs from the source length
- **THEN** `splot-recon` returns a structured `ReconError`
- **AND** library code does not panic, overflow, unwrap, or emit `decode/*`
  diagnostics

#### Scenario: Full inverse transform remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the matrix records the kernel-based 1D inverse transform as supported
- **AND** broader reconstruction remains partial until the § 7.15.2.2
  Walsh-Hadamard transform, the § 7.15.2.3 identity transform, the § 7.15.3
  secondary transform, the § 7.15.4 2D inverse transform orchestration,
  dequantization, and residual addition are implemented and proven
