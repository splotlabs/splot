## ADDED Requirements

### Requirement: Decoded frame and plane model contract

The repository SHALL document a decoded frame and plane model contract before
any decoded-output, frame-hash, Y4M-output, reference-frame-store, or encoder
roundtrip row is marked supported. The contract SHALL be tracked by
`DOC-DECODED-FRAME-PLANE-MODEL-CONTRACT`, SHALL cite AV2 § 6.4.1,
§ 6.17.4.1, § 6.17.4.4, § 7.1, § 7.21.1, § 7.21.2, § 7.21.5, § 7.21.6,
and § 7.23, and SHALL remain contract-only until source defines frame/plane
types and self-contained tests prove them.

#### Scenario: Contract defines format and dimensions

- **WHEN** a reader checks the decoded-frame model contract
- **THEN** it defines future `DecodedFrame` format fields from AV2-derived
  `BitDepth`, `SubsamplingX`, `SubsamplingY`, `Monochrome`, and `NumPlanes`
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

#### Scenario: Contract defines plane storage invariants

- **WHEN** a reader checks the future `Plane<T>` contract
- **THEN** it states that each plane owns a rectangular sample buffer or an
  equivalent borrowed view with explicit storage `width`, `height`,
  `stride_samples`, and visible rectangle metadata where storage and visible
  output differ
- **AND** it requires `stride_samples >= storage width`
- **AND** it computes `required_samples = stride_samples * storage_height` using
  checked arithmetic
- **AND** it requires the backing buffer to expose at least `required_samples`
  samples and computes `allocation_bytes = required_samples * bytes_per_sample`
  using checked arithmetic before allocation
- **AND** it states that the full backing allocation, including padding, MUST be
  charged against `DecodeLimits`
- **AND** it states that allocation padding and stride samples are not visible
  decoded output and MUST be excluded from hashes, Y4M output, and fixture
  expectations

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

#### Scenario: Contract defines output ownership

- **WHEN** a future decoder emits a decoded output frame
- **THEN** the emitted frame MUST remain immutable and valid after emission
- **AND** overwriting a reference slot MUST NOT mutate a previously emitted
  output frame
- **AND** borrowed or shared plane views are allowed only when backing samples are
  immutable for the output view, the output owns an independent copy, or
  copy-on-write / unique ownership is proven before any reference-slot mutation

#### Scenario: Contract defines allocation safety

- **WHEN** a future implementation derives plane dimensions, stride products,
  frame byte sizes, hash byte lengths, or reference-store byte accounting from
  bitstream values
- **THEN** it MUST use checked arithmetic before allocation or indexing
- **AND** it MUST reject overflow or configured limit excess as
  `decode/resource-limit`
- **AND** it MUST check the relevant `DecodeLimits` threshold before allocating
  decoded frames, hashing output, writing Y4M, or storing reference frames

#### Scenario: Contract preserves reference-store facts

- **WHEN** a future decoded frame can be stored for reference-frame reuse
- **THEN** the model preserves the frame dimensions, crop rectangle,
  subsampling, bit depth, plane count, output order, order hint, and
  film-grain-present fact needed by AV2 § 7.23 reference-frame storage
- **AND** the contract does not require reference-frame-store runtime behavior
  before its support row is implemented and tested

#### Scenario: Contract remains non-executable until implementation

- **WHEN** `decoded-frame-plane-model` is still contract-only
- **THEN** the decoder support matrix marks the row as `partial`
- **AND** the matrix row records self-contained docs/OpenSpec proof commands
- **AND** the row does not claim source type definitions, runtime allocation,
  frame hashes, Y4M support, reference-frame-store behavior, or mandatory
  AVM/dav2d execution
