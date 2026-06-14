## ADDED Requirements

### Requirement: Square DC intra prediction primitive

The repository SHALL provide a scheduler-free `splot-recon` primitive for the
square-block subset of AV2 §7.13.2.10 DC intra prediction, tracked by
`RECON-INTRA-DC-SQUARE-PREDICTION`. The primitive SHALL derive
`w = h = 1 << log2_size`, validate the expected left and above edge sample
lengths for the declared availability, validate input samples against the active
decoded bit depth, and return a typed error instead of panicking on invalid
inputs or allocation failure. The primitive SHALL NOT change `splot decode`
runtime behavior, invoke external decoders, add scheduler state to `splot-recon`,
or claim support for rectangular DC prediction, non-DC prediction modes,
dequantization, inverse transforms, residual addition, or runtime decoded-frame
output.

#### Scenario: Square DC prediction succeeds with self-contained tests

- **WHEN** `cargo test -p splot-recon --locked` runs
- **THEN** the test suite covers both-edge, left-only, above-only, and no-edge
  square DC prediction cases for the supported sample types
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: Invalid square DC prediction input is typed

- **WHEN** callers provide an unsupported square block size, missing or
  wrong-length edge samples, a sample type that cannot represent the active bit
  depth, or an edge sample outside the active bit-depth range
- **THEN** `splot-recon` returns a structured `ReconError`
- **AND** library code does not panic, unwrap, or silently clamp invalid input

#### Scenario: Full intra reconstruction remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the matrix records square DC prediction as supported
- **AND** full scalar intra reconstruction remains partial or planned until
  rectangular DC prediction, non-DC intra prediction modes, transform syntax,
  dequantization, inverse transforms, residual addition, runtime hash output,
  runtime Y4M output, and reference refresh are implemented and proven
