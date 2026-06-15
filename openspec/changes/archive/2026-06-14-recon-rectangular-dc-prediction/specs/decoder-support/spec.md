## ADDED Requirements

### Requirement: Rectangular DC intra prediction primitive

The repository SHALL provide a scheduler-free `splot-recon` primitive for the
rectangular-block AV2 §7.13.2.10 DC intra prediction process, tracked by
`RECON-INTRA-DC-RECTANGULAR-PREDICTION`. The primitive SHALL derive
`w = 1 << log2W` and `h = 1 << log2H`, validate the expected left edge length
against `h`, validate the expected above edge length against `w`, validate input
samples against the active decoded bit depth, and return typed `ReconError`
values instead of panicking on invalid inputs or allocation failure. For the
both-edge case the primitive SHALL use the AV2 approximate division path based
on §7.13.3.22 rather than replacing it with normal integer division. The
primitive SHALL NOT change `splot decode` runtime behavior, invoke external
decoders, add scheduler state to `splot-recon`, or claim support for non-DC
prediction modes, dequantization, inverse transforms, residual addition, or
runtime decoded-frame output.

#### Scenario: Rectangular DC prediction succeeds with self-contained tests

- **WHEN** `cargo test -p splot-recon --locked` runs
- **THEN** the test suite covers both-edge, left-only, above-only, and no-edge
  rectangular DC prediction cases for supported sample types
- **AND** at least one both-edge case has `log2W != log2H`, proving the
  approximate divisor path rather than the square-only power-of-two shortcut
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: Invalid rectangular DC prediction input is typed

- **WHEN** callers provide an unsupported rectangular block dimension,
  wrong-length edge samples, a sample type that cannot represent the active bit
  depth, an edge sample outside the active bit-depth range, a too-small output
  stride, or a too-small output buffer
- **THEN** `splot-recon` returns a structured `ReconError`
- **AND** library code does not panic, unwrap, silently clamp invalid input, or
  emit `decode/*` diagnostics

#### Scenario: Square DC prediction remains compatible

- **WHEN** existing callers use the square DC prediction APIs
- **THEN** the APIs continue to accept `IntraSquareBlockSize`, produce the same
  samples as before, and remain covered by the existing square tests
- **AND** rectangular support is exposed as additive API rather than a breaking
  replacement

## MODIFIED Requirements

### Requirement: Square DC intra prediction primitive

The repository SHALL provide a scheduler-free `splot-recon` primitive for the
square-block subset of AV2 §7.13.2.10 DC intra prediction, tracked by
`RECON-INTRA-DC-SQUARE-PREDICTION`. The primitive SHALL derive
`w = h = 1 << log2_size`, validate the expected left and above edge sample
lengths for the declared availability, validate input samples against the active
decoded bit depth, and return a typed error instead of panicking on invalid
inputs or allocation failure. The square primitive may share implementation with
the rectangular DC primitive, but it SHALL keep the existing square public APIs
compatible. The primitive SHALL NOT change `splot decode` runtime behavior,
invoke external decoders, add scheduler state to `splot-recon`, or claim support
for non-DC prediction modes, dequantization, inverse transforms, residual
addition, or runtime decoded-frame output.

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
  non-DC intra prediction modes, transform syntax, dequantization, inverse
  transforms, residual addition, runtime hash output, runtime Y4M output, and
  reference refresh are implemented and proven

### Requirement: Current-frame reconstruction workspace

The repository SHALL provide a scheduler-free `splot-recon` mutable current-frame
workspace tracked by `RECON-CURRENT-FRAME-WORKSPACE`. The workspace SHALL be
constructed from existing decoded-frame metadata, allocate plane storage with
checked arithmetic and fallible allocation, expose bounded plane and rectangular
sample access, support edge extraction for future intra prediction callers, and
freeze into the existing immutable `DecodedFrame<T>` model. The workspace SHALL
NOT change `splot decode` runtime behavior, add a `splot-decode -> splot-recon`
dependency edge, invoke external decoders, add scheduler state to `splot-recon`,
or claim support for tile syntax traversal, dequantization, inverse transforms,
residual generation, loop filtering, output scheduling, reference refresh, or
runtime decoded-frame output.

#### Scenario: Workspace allocation is checked and typed

- **WHEN** callers construct a current-frame workspace from decoded-frame
  metadata and an initial fill sample
- **THEN** `splot-recon` derives Y/U/V plane storage from the frame bit depth,
  pixel format, coded luma size, and visible luma rectangle
- **AND** it computes plane sample counts and allocation byte counts using
  checked arithmetic before allocating
- **AND** allocation failure, unsupported sample type, out-of-range fill sample,
  or geometry mismatch returns a structured `ReconError`
- **AND** library code does not panic, unwrap, silently clamp invalid input, or
  emit `decode/*` diagnostics

#### Scenario: Workspace exposes bounded sample writes

- **WHEN** callers write samples or rectangular blocks into a workspace plane
- **THEN** `splot-recon` validates the target plane exists, the rectangle is
  inside the plane storage, the provided row/block shape matches the target, and
  every sample fits the active bit depth
- **AND** invalid coordinates, missing planes, shape mismatches, or out-of-range
  samples return structured `ReconError` values
- **AND** samples outside the requested rectangle remain unchanged

#### Scenario: Workspace supports square DC prediction writes

- **WHEN** callers request square DC intra prediction into a workspace plane
  using available left and/or above edge samples
- **THEN** the workspace validates the target square, derives or accepts edge
  samples without deciding AV2 block-availability semantics, calls the existing
  square DC prediction primitive, and writes the predicted square into the
  workspace storage
- **AND** rectangular DC prediction, non-DC intra prediction modes,
  transform-block syntax, dequantization, inverse transforms, residual addition,
  and `decode_tile()` remain unsupported by that square helper

#### Scenario: Workspace supports rectangular DC prediction writes

- **WHEN** callers request rectangular DC intra prediction into a workspace plane
  using available left and/or above edge samples
- **THEN** the workspace validates the target rectangle, extracts in-storage left
  samples using the rectangle height and above samples using the rectangle
  width, calls the rectangular DC prediction primitive, and writes the predicted
  rectangle into the workspace storage
- **AND** the helper does not decide AV2 block-availability, tile-boundary,
  subsampled-DC, transform syntax, dequantization, inverse transform, residual,
  or runtime decode semantics

#### Scenario: Workspace freezes into immutable output

- **WHEN** a caller freezes a completed current-frame workspace
- **THEN** `splot-recon` returns the existing immutable `DecodedFrame<T>` type
  after reusing the existing plane and frame validation paths
- **AND** existing decoded-frame hash, Y4M writer, and reference-store APIs can
  consume the frozen frame in self-contained tests
- **AND** the operation does not assign AV2 output order, synthesize film grain,
  run loop filters, refresh references, write runtime output, or invoke AVM,
  dav2d, ffmpeg, or any external decoder

#### Scenario: Scheduler-free boundary remains enforced

- **WHEN** `cargo xtask check-concurrency-policy` and dependency-direction checks
  run
- **THEN** `splot-recon` contains no direct Rayon, crossbeam, worker-pool,
  global-pool, ad-hoc thread, or pipeline-queue usage
- **AND** future parallel decode or encoder orchestration remains outside the
  workspace and owned by `splot-decode` `DecodeContext` /
  `splot_parallel::WorkerPool`

#### Scenario: Full intra reconstruction remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the matrix records the current-frame workspace as supported
- **AND** broad scalar intra reconstruction remains partial until tile block
  syntax, block availability, non-DC prediction modes, dequantization, inverse
  transforms, residual addition, runtime hash output, runtime Y4M output, and
  reference refresh are implemented and proven
