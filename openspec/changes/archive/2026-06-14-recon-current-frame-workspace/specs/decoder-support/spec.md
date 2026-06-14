## ADDED Requirements

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
  and `decode_tile()` remain unsupported

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
  syntax, block availability, rectangular and non-DC prediction modes,
  dequantization, inverse transforms, residual addition, runtime hash output,
  runtime Y4M output, and reference refresh are implemented and proven
