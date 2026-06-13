# decoder-support Specification

## Purpose
Define the repository-owned decoder/reconstruction support status model,
including roadmap scope, generated status docs, self-contained proof
requirements, structured unsupported diagnostics, and the local-only reference
evidence boundary.
## Requirements
### Requirement: Decoder roadmap
The repository SHALL document the decoder scope in `docs/DECODER-ROADMAP.md`.
The roadmap SHALL state that decoder work exists to support future encoder
roundtrips and reconstruction correctness, not production playback. The roadmap
SHALL define the staged `splot decode` path, the first supported tier before it
is implemented, deterministic frame-hash expectations, unsupported-feature
handling, and the local-only AVM/dav2d evidence boundary.

#### Scenario: Reader checks decoder scope
- **WHEN** a reader opens `docs/DECODER-ROADMAP.md`
- **THEN** the document says whether `splot decode` currently reconstructs pixels,
  what the first supported tier is, and which broad AV2 tools remain unsupported

#### Scenario: Local reference boundary is visible
- **WHEN** a reader checks how AVM or dav2d may be used during decoder work
- **THEN** the roadmap states that they are local development evidence only and
  SHALL NOT be invoked by repo code, build scripts, tests, `xtask`, or CI

### Requirement: Decoder support matrix
The repository SHALL provide `docs/DECODER-SUPPORT-MATRIX.toml` as the canonical
decoder/reconstruction support status file. Each row SHALL include a stable row
id, a linked Feature ID where available, spec sections, parser source,
decode/reconstruction module, supported tier, status, self-contained tests,
diagnostics, local reference evidence, and notes. Row status SHALL be one of
`todo`, `partial`, `supported`, `unsupported-intentional`, or `blocked`.

#### Scenario: Matrix row records unsupported behavior
- **WHEN** a decoder area is intentionally unsupported
- **THEN** its matrix row records `status = "unsupported-intentional"` or
  `status = "todo"` with the relevant spec section and the diagnostic or
  planned diagnostic that will explain the unsupported feature

#### Scenario: Supported row has proof
- **WHEN** a matrix row has `status = "supported"`
- **THEN** the row records at least one self-contained test or fixture that does
  not require AVM or dav2d at test time

### Requirement: Generated decoder support status
The repository SHALL generate a committed decoder support status document from
`docs/DECODER-SUPPORT-MATRIX.toml`. The generated document SHALL summarize row
counts by status and tier, list each row with its spec sections and tests, and
name any local reference evidence as portable metadata only.

#### Scenario: Matrix is rendered
- **WHEN** `cargo xtask decoder-support --format markdown --output docs/DECODER-SUPPORT-STATUS.md` runs
- **THEN** the command writes a deterministic Markdown render of
  `docs/DECODER-SUPPORT-MATRIX.toml`

#### Scenario: Generated document drifts
- **WHEN** `docs/DECODER-SUPPORT-MATRIX.toml` changes without regenerating
  `docs/DECODER-SUPPORT-STATUS.md`
- **THEN** `cargo xtask check-decoder-support` fails and names the regeneration
  command

### Requirement: Structured decode unsupported diagnostics
Unsupported decoder features SHALL be represented in docs and matrix rows as
structured diagnostics with a stable rule id, severity, optional spec section,
matrix row id, human-readable message, and remediation. The `splot decode`
CLI entry point SHALL emit `decode/unsupported-feature` with severity `Error`,
spec section `7.1`, matrix row `cli-decode-entrypoint`, and Feature ID
`CLI-DECODE` until a supported decoder path replaces the intentional
unsupported implementation.

#### Scenario: Unsupported feature is documented
- **WHEN** a matrix row identifies an unsupported AV2 tool
- **THEN** the row links the unsupported behavior to a stable diagnostic code or
  planned diagnostic code and a spec section where applicable

#### Scenario: Decode command emits text diagnostic
- **WHEN** `splot decode <input> -o <output>` is run before decode support is
  implemented
- **THEN** it exits with code `1`
- **AND** stderr contains diagnostic rule id `decode/unsupported-feature`,
  severity `Error`, spec section `7.1`, matrix row `cli-decode-entrypoint`,
  and Feature ID `CLI-DECODE`
- **AND** no AVM, dav2d, ffmpeg, or external decoder is located or invoked

#### Scenario: Decode command emits JSON diagnostic
- **WHEN** `splot decode --json <input> -o <output>` is run before decode support
  is implemented
- **THEN** it exits with code `1`
- **AND** stdout is a machine-readable diagnostic object containing
  `rule_id = "decode/unsupported-feature"`, `severity = "Error"`,
  `spec_section = "7.1"`, `matrix_row = "cli-decode-entrypoint"`, and
  `feature_id = "CLI-DECODE"`
- **AND** stderr remains empty unless an operational error occurs

#### Scenario: Decode command avoids file I/O while unsupported
- **WHEN** `splot decode <missing-input> -o <output>` is run before decode
  support is implemented
- **THEN** it exits with code `1`
- **AND** it emits `decode/unsupported-feature`
- **AND** it does not create the missing input path or output path

### Requirement: Local reference evidence remains non-executable
The repository SHALL treat local AVM/dav2d evidence as non-executable metadata
only. Evidence may be recorded as commit hashes, command summaries, decoded
hashes, and comparison notes in documentation, PR descriptions, agent-log files,
or portable fixture manifests. The repository SHALL NOT add code paths, scripts,
wrappers, build probes, dependencies, tests, CI jobs, or `xtask` commands that
locate, build, invoke, or require AVM or dav2d.

#### Scenario: CI runs decoder support checks
- **WHEN** `cargo xtask ci` runs on a machine without AVM or dav2d installed
- **THEN** decoder support status checks pass or fail solely from committed
  repository files

#### Scenario: Local evidence is recorded
- **WHEN** a future decoder fixture records AVM or dav2d evidence
- **THEN** the committed evidence is portable metadata and does not contain local
  absolute paths or require the reference tools to be installed

### Requirement: canonical decoder diagnostic registry

Decoder diagnostics emitted by `splot decode` SHALL be documented in
`docs/DECODER-DIAGNOSTICS.md` with stable field names `rule_id`, `severity`,
`spec_section`, `matrix_row`, `feature_id`, `message`, and `remediation` when
applicable. The `spec_section` field SHALL cite an AV2 section when the
diagnostic is tied to AV2 decoding behavior, and the decoder support matrix
SHALL link emitted decoder diagnostics to support rows. Tracked by
`DOC-DECODER-DIAGNOSTICS`.

#### Scenario: decode diagnostic is emitted

- **WHEN** `splot decode` emits a `decode/*` diagnostic
- **THEN** the rule ID is present in `docs/DECODER-DIAGNOSTICS.md`
- **AND** the diagnostic is linked to a row in
  `docs/DECODER-SUPPORT-MATRIX.toml`

#### Scenario: unsupported decode entry point is documented

- **WHEN** `splot decode` reports the current unsupported entry point
- **THEN** `decode/unsupported-feature` is documented with severity `Error`,
  AV2 §7.1, `CLI-DECODE`, and matrix row `cli-decode-entrypoint`

### Requirement: Decode limits contract

The repository SHALL document a future `DecodeOptions { limits: DecodeLimits }`
contract before any `splot decode` path performs bitstream-derived allocation.
The contract SHALL treat limits as `splot` resource policy layered over
spec-derived values, not as AV2 conformance rules. The documented limits SHALL
cover input bytes, OBU count, decoded frame count, output frame count, frame
width, frame height, luma samples per frame, decoded frame bytes, reference
frames, tile count, tile payload bytes, and output bytes. Tracked by
`DOC-DECODE-LIMITS-CONTRACT`.

#### Scenario: Contract cites spec-derived values

- **WHEN** a reader checks the decode limits contract
- **THEN** it cites AV2 § 6.4.1 for sequence maximum frame dimensions,
  § 6.4.6 for reference-frame count, § 6.17.4.1 for per-frame dimensions,
  § 6.17.7.2 for tile grid counts, § 5.19 for tile group count derivation,
  § 5.20 for tile payload traversal, § 7.1 for the general decode input/output
  model, § 7.21 for decoded output arrays, and § 7.23 for reference frame
  storage
- **AND** it distinguishes those spec-derived values from the repository-owned
  limit thresholds

#### Scenario: Allocation is gated by limits

- **WHEN** a future byte-consuming decode planner accepts bytes or traverses OBUs
- **THEN** the planner MUST check `max_input_bytes` before buffering or
  accepting input bytes
- **AND** the planner MUST check `max_obus` before continuing OBU traversal or
  accumulating OBU state

#### Scenario: Derived sizes use checked arithmetic

- **WHEN** a future byte-consuming decode planner derives dimensions, strides,
  tile products, plane sizes, decoded frame bytes, reference-storage bytes,
  output bytes, frame counts, or output frame counts from input
- **THEN** it MUST compute the derived `actual` value with checked arithmetic
  before comparing against `DecodeLimits` or allocating
- **AND** arithmetic overflow during derivation MUST be treated as a
  `decode/resource-limit` failure

#### Scenario: Derived allocations are gated by limits

- **WHEN** a future byte-consuming decode planner derives dimensions, tile
  counts, output frame counts, reference frame counts, or decoded/output byte
  sizes from input
- **THEN** the planner MUST check the relevant `DecodeLimits` value before
  allocating, indexing, traversing tile payloads, storing a reference frame,
  producing Y4M, or producing a deterministic frame hash

### Requirement: Decode resource-limit diagnostic contract

The repository SHALL document `decode/resource-limit` as a planned decoder
diagnostic for future limit violations. Until source emits this diagnostic, it
SHALL NOT appear in the marker-delimited emitted decoder diagnostic registry.
When emitted, the diagnostic SHALL include the stable decoder diagnostic fields
`rule_id`, `severity`, `spec_section`, `matrix_row`, `feature_id`, `message`,
and `remediation`, plus resource fields `limit_name`, `limit`, `actual`, `unit`,
`byte_offset`, and `bit_offset`.

#### Scenario: Planned diagnostic stays out of emitted registry

- **WHEN** `cargo xtask check-diagnostic-registry` runs before
  `decode/resource-limit` is emitted by source
- **THEN** the decoder diagnostic registry contains only emitted `decode/*`
  rule IDs inside its enforced marker region
- **AND** the planned resource-limit diagnostic is documented outside that
  emitted registry region or in support/roadmap text

#### Scenario: Future limit violation reports measured value

- **WHEN** a future `splot decode` path rejects an input because a measured
  spec-derived value exceeds a `DecodeLimits` threshold
- **THEN** it emits `decode/resource-limit` with severity `Error`, matrix row
  `decode-limits-budget`, Feature ID `DOC-DECODE-LIMITS-CONTRACT`, the AV2
  section that supplied the measured value, the limit name, configured limit,
  measured actual value, unit, and any known byte/bit offset

### Requirement: Deterministic decoded-frame hash contract

The repository SHALL document a deterministic decoded-frame hash contract before
any decoder output row, Y4M output row, fixture manifest, or encoder roundtrip
expectation is marked supported. The contract SHALL be tracked by
`DOC-DETERMINISTIC-FRAME-HASH-CONTRACT`, SHALL cite AV2 § 5.17.12, § 6.16.13,
§ 7.21.1, § 7.21.2, and § 7.21.7, and SHALL remain contract-only until source
computes hashes from decoded output with self-contained tests.

#### Scenario: Contract defines sample byte stream

- **WHEN** a reader checks the hash contract
- **THEN** it defines frame order as zero-based AV2 § 7.21 output order after
  supported stream/layer selection
- **AND** it defines the future hash input as cropped output samples, excluding
  allocation padding, stride bytes, and codec metadata fields
- **AND** it defines luma dimensions as `w x h` and chroma dimensions as
  `((w + subX) >> subX) x ((h + subY) >> subY)` using the values from the AV2
  output process
- **AND** it defines sample traversal as row-major raster order within each
  plane and plane order as Y, then U, then V for non-monochrome output

#### Scenario: Contract defines byte representation and algorithm

- **WHEN** a reader checks how samples become hash bytes
- **THEN** the contract states that 8-bit samples are encoded as one byte
- **AND** samples with bit depth greater than 8 are encoded as two bytes in
  little-endian order
- **AND** the initial repository-owned algorithm is
  `splot-dfh-sha256-v1`, a SHA-256 digest over the canonical AV2 sample-byte
  serialization
- **AND** AV2 `hash_type = 0` MD5 remains a separate future
  `METADATA_TYPE_DECODED_FRAME_HASH` interop verification path
- **AND** other AV2 `hash_type` values remain reserved by AOMedia and are not
  `splot` hash variants

#### Scenario: Contract defines grain and variant labels

- **WHEN** a reader checks the supported hash variant
- **THEN** the default future `splot` frame hash is the raw decoded output
  variant corresponding to AV2 `has_grain = 0`
- **AND** any future post-film-grain hash MUST be explicitly labeled as a
  separate variant before being treated as supported
- **AND** the contract states that film-grain-capable hashes require the
  § 7.21.7 film-grain synthesis process to be implemented and tested

#### Scenario: Contract remains non-executable until implementation

- **WHEN** `deterministic-frame-hash` is still contract-only
- **THEN** the decoder support matrix marks the row as `partial`
- **AND** the matrix row records self-contained docs/OpenSpec proof commands
- **AND** the row does not claim source emission, runtime hash computation, Y4M
  support, or mandatory AVM/dav2d execution

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
