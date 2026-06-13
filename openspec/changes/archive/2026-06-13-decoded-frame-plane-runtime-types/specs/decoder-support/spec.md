## MODIFIED Requirements

### Requirement: Decoded frame and plane model contract

The repository SHALL document a decoded frame and plane model contract and
SHALL provide source-backed runtime model types in `crates/splot-recon` before
any decoded-output, frame-hash, Y4M-output, reference-frame-store, or encoder
roundtrip row is marked supported. The contract SHALL be tracked by
`DOC-DECODED-FRAME-PLANE-MODEL-CONTRACT`; the runtime type implementation
SHALL be tracked by `INFRA-RECON-FRAME-PLANE-TYPES`. The model SHALL cite AV2
§ 6.4.1, § 6.17.4.1, § 6.17.4.4, § 7.1, § 7.21.1, § 7.21.2, § 7.21.5,
§ 7.21.6, and § 7.23. The runtime API SHALL remain self-contained and MUST NOT
claim byte-consuming decode, reconstruction algorithms, frame hashes, Y4M
output, reference-frame-store behavior, or mandatory AVM/dav2d execution.

#### Scenario: Contract defines format and dimensions

- **WHEN** a reader checks the decoded-frame model contract or the
  `splot-recon` runtime model API
- **THEN** it defines decoded-frame format fields from AV2-derived `BitDepth`,
  `SubsamplingX`, `SubsamplingY`, `Monochrome`, and `NumPlanes`
- **AND** it maps AV2 v1.0.0 `bit_depth_idc = 0` to 10-bit samples,
  `bit_depth_idc = 1` to 8-bit samples, and rejects reserved values
- **AND** it maps AV2 v1.0.0 `chroma_format_idc` values to monochrome, 4:2:0,
  4:2:2, and 4:4:4 formats with their subsampling and plane-count facts
- **AND** it states that AV2 v1.0.0 output samples are 8-bit or 10-bit and
  decoded sample values MUST fit `0..=(1 << bit_depth) - 1`
- **AND** it distinguishes coded luma frame dimensions `FrameWidth` and
  `FrameHeight` from cropped output dimensions
- **AND** it defines a repository-owned zero-based emission index over frames
  emitted by the AV2 output processes after supported stream/layer selection,
  not decode order
- **AND** it defines the visible output luma dimensions from the output process
  `w` and `h`
- **AND** it defines chroma visible dimensions as
  `((w + subX) >> subX) x ((h + subY) >> subY)` for non-monochrome output
- **AND** it states that U and V are absent or ignored when `NumPlanes == 1`

#### Scenario: Runtime validates plane storage invariants

- **WHEN** `splot-recon` constructs a `Plane<T>` from owned sample storage
- **THEN** the constructor requires explicit storage `width`, storage `height`,
  `stride_samples`, and visible rectangle metadata where storage and visible
  output differ
- **AND** it rejects zero storage dimensions or zero visible dimensions
- **AND** it requires `stride_samples >= storage width`
- **AND** it computes `required_samples = stride_samples * storage_height`
  using checked arithmetic
- **AND** it requires the backing buffer length to equal `required_samples`
- **AND** it computes `allocation_bytes = required_samples * bytes_per_sample`
  using checked arithmetic before reporting allocation size
- **AND** it rejects visible rectangles outside the storage rectangle
- **AND** allocation padding and stride samples are not visible decoded output
  and MUST be excluded from hashes, Y4M output, and fixture expectations

#### Scenario: Runtime validates decoded frame invariants

- **WHEN** `splot-recon` constructs a `DecodedFrame<T>`
- **THEN** the constructor validates that the visible luma crop is positive and
  inside the coded luma dimensions
- **AND** for non-monochrome formats it rejects luma crop origins that are not
  aligned to `SubsamplingX` and `SubsamplingY`
- **AND** it requires the Y plane visible size to match the visible luma crop
- **AND** it requires U and V planes to be absent for monochrome output
- **AND** it requires U and V planes to be present for non-monochrome output
- **AND** it requires U and V visible sizes to match the AV2-derived chroma
  visible dimensions
- **AND** it rejects a sample type that cannot represent the requested bit
  depth
- **AND** it rejects any stored sample value above the active bit-depth maximum

#### Scenario: Contract distinguishes output and reference storage

- **WHEN** a reader checks how future frame data is reused
- **THEN** the contract states that decoded output frames are cropped
  `OutY`/`OutU`/`OutV` arrays from AV2 § 7.21.1 and § 7.21.2
- **AND** it states that reference-frame storage is loop-restored `LrFrame`
  copied into `FrameStore` over coded/padded luma dimensions
  `MiCols * MI_SIZE` by `MiRows * MI_SIZE` and corresponding chroma dimensions
  shifted by `SubsamplingX` and `SubsamplingY`
- **AND** it states that future APIs MUST NOT treat output-frame crop dimensions
  and reference-store backing dimensions as interchangeable

#### Scenario: Runtime defines output ownership

- **WHEN** a decoded output frame is represented by `splot-recon`
- **THEN** the emitted frame model remains immutable and valid after creation
- **AND** overwriting a future reference slot MUST NOT mutate a previously
  emitted output frame
- **AND** borrowed or shared plane views are allowed only when backing samples
  are immutable for the output view, the output owns an independent copy, or
  copy-on-write / unique ownership is proven before any reference-slot mutation

#### Scenario: Contract defines allocation safety

- **WHEN** a future implementation derives plane dimensions, stride products,
  frame byte sizes, hash byte lengths, or reference-store byte accounting from
  bitstream values
- **THEN** it MUST use checked arithmetic before allocation or indexing
- **AND** future byte-consuming decode code MUST reject arithmetic overflow or
  configured limit excess as `decode/resource-limit`
- **AND** `splot-recon` constructors MUST reject local arithmetic overflow with
  typed reconstruction errors without emitting decoder diagnostics directly
- **AND** future decode code MUST check the relevant `DecodeLimits` threshold
  before allocating decoded frames, hashing output, writing Y4M, or storing
  reference frames

#### Scenario: Contract preserves reference-store facts

- **WHEN** a future decoded frame can be stored for reference-frame reuse
- **THEN** the model preserves or has an explicit extension point for the frame
  dimensions, crop rectangle, subsampling, bit depth, plane count, output order,
  order hint, and film-grain-present fact needed by AV2 § 7.23 reference-frame
  storage
- **AND** the contract does not require reference-frame-store runtime behavior
  before its support row is implemented and tested

#### Scenario: Runtime support is source-backed but narrowly scoped

- **WHEN** `decoded-frame-plane-runtime-types` is marked supported
- **THEN** the decoder support matrix records self-contained `splot-recon` tests
  proving the runtime model invariants
- **AND** the implementation matrix records `INFRA-RECON-FRAME-PLANE-TYPES`
  with source modules and proof commands
- **AND** the row does not claim byte-consuming decode, runtime allocation from
  a bitstream, frame hashes, Y4M support, reference-frame-store behavior, or
  mandatory AVM/dav2d execution
