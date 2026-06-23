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

### Requirement: DC Subsampled Prediction Support Row
The decoder support model SHALL track
`RECON-INTRA-DC-SUBSAMPLED-PREDICTION` as a distinct `splot-recon`
source-backed row named `intra-dc-subsampled-prediction`. The row SHALL mark
only AV2 §7.13.2.11 prepared-edge scalar prediction and workspace handoff as
supported, SHALL cite AV2 v1.0.0 §7.13.2.1, §7.13.2.11, and §7.13.3.22, and
SHALL keep broad intra reconstruction, full `predict_intra()` dispatch, CfL,
runtime decode, transform/residual, loop-filter, and reference-refresh rows
honestly partial or unsupported.

#### Scenario: Matrix records narrow support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `intra-dc-subsampled-prediction` appears with Feature ID
  `RECON-INTRA-DC-SUBSAMPLED-PREDICTION`
- **AND** it names focused unit/workspace tests plus the extended
  `recon_intra_prediction_bytes` fuzz target
- **AND** it does not claim full CfL, data-driven prediction, IBP, general
  directional prediction, residuals, transforms, loop filters, reference
  refresh, film grain, AVM/dav2d evidence, or full decoder conformance

#### Scenario: Broad rows remain partial
- **WHEN** decoder support and conformance coverage status documents are
  regenerated
- **THEN** `intra-reconstruction`, `prediction-process`, and other broad runtime
  decoder rows remain partial or unsupported until separately implemented with
  runtime evidence

### Requirement: Tile Partition Traversal Support Row
The decoder support model SHALL track `DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY`
as a distinct crate-private row named `tile-partition-traversal-boundary`. The
row SHALL mark only the partition traversal frontier to the first
`decode_block()` boundary as supported, and SHALL keep broader
`tile-payload-decode`, `symbol-decoder`, CDF lifecycle, runtime decode output,
block syntax, `MiSizes` mutation, and reconstruction rows honest when they
remain partial.

#### Scenario: Traversal row is supported without broad decode overclaim
- **WHEN** the decoder support matrix is regenerated after this change
- **THEN** `tile-partition-traversal-boundary` appears with Feature ID
  `DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY`
- **AND** it cites AV2 §5.20.3.1, §5.20.3.2, §5.20.9.1, §8.3.2, and §9.2 as
  applicable evidence sections
- **AND** it does not cite §5.20.10.4/§5.20.10.5 as parsed or tested evidence
  while loop-restoration syntax remains outside this boundary
- **AND** it does not cite §5.20.4.1 as parsed or tested evidence while block
  syntax remains outside this boundary
- **AND** `tile-payload-decode` remains partial for full `decode_tile()`, block
  syntax, `MiSizes` mutation, reconstruction, output, CDF lifecycle, and
  reference refresh work

#### Scenario: Matrix evidence names focused tests
- **WHEN** `cargo xtask check-decoder-support` validates the matrix
- **THEN** the traversal row names focused crate-private tests for prefix
  child-call ordering, frontier records, transactional CDF handling, checked
  arithmetic/resource failures, and unsupported SDP/BRU/inter gates

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
matrix row id, human-readable message, and remediation. `splot-decode` SHALL
own the `decode/unsupported-feature` descriptor with severity `Error`. When
`splot decode` reaches the runtime decode/output boundary after successful byte
planning, the descriptor SHALL cite AV2 §7.1 with matrix row
`cli-decode-entrypoint` and Feature ID `CLI-DECODE`. When the byte or stream
planner rejects a parsed but unsupported structure, the diagnostic SHALL reuse
`decode/unsupported-feature` with the planner-owned matrix row, Feature ID,
spec section, reason, OBU type, and byte offset. The `splot decode` CLI entry
point SHALL render library-owned diagnostic reports without changing their base
text or JSON field values.

#### Scenario: Unsupported feature is documented
- **WHEN** a matrix row identifies an unsupported AV2 tool
- **THEN** the row links the unsupported behavior to a stable diagnostic code or
  planned diagnostic code and a spec section where applicable

#### Scenario: Decode crate owns the runtime unsupported diagnostic descriptor
- **WHEN** `splot-decode` is tested
- **THEN** it exposes the runtime `decode/unsupported-feature` descriptor with
  severity `Error`, spec section `7.1`, matrix row `cli-decode-entrypoint`, and
  Feature ID `CLI-DECODE`

#### Scenario: Decode command emits runtime unsupported text diagnostic
- **WHEN** `splot decode <input> -o <output>` is run on bytes that can be
  planned but runtime decode support is not implemented
- **THEN** it reads the input bytes, plans them through `DecodeContext::plan_bytes`,
  and exits with code `1`
- **AND** stderr contains diagnostic rule id `decode/unsupported-feature`,
  severity `Error`, spec section `7.1`, matrix row `cli-decode-entrypoint`,
  Feature ID `CLI-DECODE`, and a plan summary
- **AND** no AVM, dav2d, ffmpeg, or external decoder is located or invoked
- **AND** the requested output path is not created, truncated, or written

#### Scenario: Decode command emits runtime unsupported JSON diagnostic
- **WHEN** `splot decode --json <input> -o <output>` is run on bytes that can be
  planned but runtime decode support is not implemented
- **THEN** it exits with code `1`
- **AND** stdout is a machine-readable diagnostic object containing
  `rule_id = "decode/unsupported-feature"`, `severity = "Error"`,
  `spec_section = "7.1"`, `matrix_row = "cli-decode-entrypoint"`, and
  `feature_id = "CLI-DECODE"`
- **AND** stdout includes a runtime-unsupported detail block with input length,
  detected bitstream format, OBU count, frame-candidate count, and selected
  output format
- **AND** stderr remains empty unless an operational error occurs

#### Scenario: Decode command emits planner unsupported diagnostic
- **WHEN** `splot decode <input> -o <output>` reads bytes whose source parses but
  contains a structure outside the initial planner tier
- **THEN** it exits with code `1`
- **AND** it emits `decode/unsupported-feature` using the planner matrix row,
  Feature ID, AV2 spec section, unsupported reason, OBU type, and byte offset
- **AND** the requested output path is not created, truncated, or written

#### Scenario: Decode command reports missing input as operational error
- **WHEN** `splot decode <missing-input> -o <output>` is run
- **THEN** it exits with code `2`
- **AND** it does not emit a `decode/*` diagnostic
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

The repository SHALL document and source-back a
`DecodeOptions { limits: DecodeLimits }` contract before any `splot decode` path
performs bitstream-derived allocation. The contract SHALL treat limits as
`splot` resource policy layered over spec-derived values, not as AV2 conformance
rules. The documented and source-backed limits SHALL cover input bytes, OBU
count, decoded frame count, output frame count, frame width, frame height, luma
samples per frame, decoded frame bytes, reference slots, reference store bytes,
tile count, tile payload bytes, and output bytes. Tracked by
`DECODE-LIMITS-RUNTIME-API`, while the older docs-only row
`DOC-DECODE-LIMITS-CONTRACT` remains the contract umbrella until byte-consuming
enforcement and diagnostics exist.

#### Scenario: Contract cites spec-derived values

- **WHEN** a reader checks the decode limits contract
- **THEN** it cites AV2 § 4.11.6, Annex B.2-B.3, and § 5.2.1 for input and OBU
  byte surfaces; § 6.4.1, § 5.18.4.1, § 6.17.4.1, § 5.18.4.4, and § 6.17.4.4
  for sequence, frame, and output geometry; § 5.18.7.2, § 6.17.7.2, § 5.19,
  § 6.18, § 5.20.1, and § 6.19.1 for tile counts and payload traversal; § 7.1
  and § 7.21 for decoded output arrays; and § 6.4.6 and § 7.23 for reference
  slot and reference store surfaces
- **AND** it distinguishes those spec-derived measured values from the
  repository-owned limit thresholds

#### Scenario: Runtime policy API exists

- **WHEN** `splot-decode` is tested
- **THEN** it exposes dependency-free `DecodeOptions`, `DecodeLimits`, typed
  limit-name and unit types, finite defaults, inclusive limit comparison
  helpers, checked arithmetic helpers, allocation-size handoff checks, and local
  typed errors
- **AND** those types do not require `serde`, new dependencies, `splot-cli`, AVM,
  dav2d, or any byte-consuming decode entry point

#### Scenario: Runtime defaults are finite policy

- **WHEN** a caller constructs default decode options or default decode limits
- **THEN** the runtime API returns finite nonzero thresholds suitable for CI and
  fuzzing and current large-stream decoder-mission traversal rather than AV2
  normative conformance limits
- **AND** the default OBU and frame-count thresholds are large enough for the
  current `ac0ej3.ivf` target's inspected 12964 OBU stream to advance past the
  prior `max_frames_to_decode = 128` planner gate
- **AND** tests pin the default thresholds so policy changes are explicit

#### Scenario: Allocation is gated by limits

- **WHEN** a future byte-consuming decode planner accepts bytes or traverses OBUs
- **THEN** the planner MUST check `max_input_bytes` before buffering or
  accepting input bytes
- **AND** the planner MUST check `max_obus` before continuing OBU traversal or
  accumulating OBU state

#### Scenario: Derived sizes use checked arithmetic

- **WHEN** a future byte-consuming decode planner derives dimensions, strides,
  tile products, plane sizes, decoded frame bytes, reference-store bytes,
  output bytes, frame counts, or output frame counts from input
- **THEN** it MUST compute the derived `actual` value with checked arithmetic
  before comparing against `DecodeLimits` or allocating
- **AND** arithmetic overflow during derivation MUST be represented by the local
  runtime limit error API before any future diagnostic adaptation

#### Scenario: Derived allocations are gated by limits

- **WHEN** a future byte-consuming decode planner derives dimensions, tile
  counts, output frame counts, reference slot counts, reference-store byte
  sizes, or decoded/output byte sizes from input
- **THEN** the planner MUST check the relevant `DecodeLimits` value before
  allocating, indexing, traversing tile payloads, storing a reference frame,
  producing Y4M, or producing a deterministic frame hash

#### Scenario: Runtime API does not emit diagnostics

- **WHEN** the runtime limit helper rejects a value or reports arithmetic
  overflow
- **THEN** it returns a local typed error and does not emit
  `decode/resource-limit`
- **AND** the emitted decoder diagnostic registry remains unchanged until a
  future byte-consuming decode path maps local helper errors into structured
  diagnostics

### Requirement: Decode resource-limit diagnostic contract

The repository SHALL document and emit `decode/resource-limit` when a
byte-consuming `splot decode` path rejects an input because a measured
spec-derived or repository-owned decode-planner value exceeds a configured
`DecodeLimits` threshold. The diagnostic SHALL include the stable decoder
diagnostic fields `rule_id`, `severity`, `spec_section`, `matrix_row`,
`feature_id`, `message`, and `remediation`, plus resource fields `limit_name`,
`limit`, `actual`, and `unit`; `byte_offset` and `bit_offset` SHALL be included
only when the rejecting path knows them. Resource limits are `splot` policy over
measured values and SHALL NOT be described as AV2 conformance failures.

#### Scenario: Limit violation reports measured value

- **WHEN** `splot decode` rejects an input because byte planning exceeds a
  `DecodeLimits` threshold
- **THEN** it emits `decode/resource-limit` with severity `Error`, matrix row
  `decode-limits-budget`, Feature ID `DOC-DECODE-LIMITS-CONTRACT`, the relevant
  AV2 or policy section, the limit name, configured limit, measured actual
  value, unit, and known byte/bit offsets when available
- **AND** it omits byte/bit offset fields when the rejecting planner path does
  not know a precise location
- **AND** the requested output path is not created, truncated, or written

#### Scenario: Oversized input is bounded before full read

- **WHEN** `splot decode` is given an input path whose byte length exceeds the
  finite default `max_input_bytes` policy
- **THEN** it limits file reading before constructing a full input buffer
- **AND** it emits `decode/resource-limit` for `limit_name = "max_input_bytes"`
  with the configured limit, measured actual value, and unit `bytes`
- **AND** it leaves `spec_section` unset because `max_input_bytes` is repository
  policy rather than AV2 syntax
- **AND** the requested output path is not created, truncated, or written

#### Scenario: Resource-limit diagnostic is in emitted registry

- **WHEN** `cargo xtask check-diagnostic-registry` runs after source emits
  `decode/resource-limit`
- **THEN** `decode/resource-limit` appears inside the emitted decoder diagnostic
  registry marker region
- **AND** the decoder support matrix links the emitted diagnostic to a support row

### Requirement: Deterministic decoded-frame hash contract

The repository SHALL document a deterministic decoded-frame hash contract before
any decoder output row, Y4M output row, fixture manifest, or encoder roundtrip
expectation is marked supported. The contract SHALL be tracked by
`DOC-DETERMINISTIC-FRAME-HASH-CONTRACT`, SHALL cite AV2 § 5.17.12, § 6.16.13,
§ 7.21.1, § 7.21.2, and § 7.21.7, and SHALL remain contract-only until source
computes hashes from decoded output with self-contained tests.

#### Scenario: Contract defines sample byte stream

- **WHEN** a reader checks the hash contract
- **THEN** it defines frame order as the repository-owned zero-based emission
  index over frames emitted by the AV2 output processes after supported
  stream/layer selection
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

### Requirement: Minimal decode tier contract

The decoder support docs SHALL define the first supported decode tier before any
output, Y4M output, or encoder roundtrip expectation is marked supported. The
contract SHALL use Feature ID `DOC-MINIMAL-DECODE-TIER-CONTRACT`, contract ID
`splot.decode.minimal_tier`, contract version `1`, and tier ID
`minimal-intra-8bit420-hash-v1`. The contract SHALL remain docs-only until
source implements the tier and self-contained tests prove it.

#### Scenario: Contract avoids Annex A conformance overclaim

- **WHEN** a reader checks the minimal decode tier contract
- **THEN** it states that the tier is a `splot` implementation-supported subset
  and not an Annex A level-conformant decoder claim
- **AND** it keeps current `splot decode` behavior as intentionally unsupported
  until runtime support lands

#### Scenario: Contract defines accepted input and layer shape

- **WHEN** a reader checks the minimal decode tier input boundary
- **THEN** it admits only Annex B length-delimited OBU input, including
  IVF/DKIF streams whose frame payloads are Annex B
- **AND** it requires one selected stream/layer with non-global
  `obu_xlayer_id == 0`, `obu_tlayer_id == 0`, inferred `obu_mlayer_id == 0`,
  and no temporal or embedded enhancement layer
- **AND** it excludes bare OBU streams, Y4M input, multistream composition,
  external HLS, MSDO, LCR, Atlas, OPS selection, sub-bitstream extraction, and
  any external decoder wrapper

#### Scenario: Contract defines sequence format and limits

- **WHEN** a reader checks the minimal decode tier sequence boundary
- **THEN** it requires `seq_profile_idc == 0` (`Main_420_10_IP0`) input
  further narrowed to `chroma_format_idc == 0`, `bit_depth_idc == 1`,
  `max_tlayer_id == 0`, `max_mlayer_id == 0`, `SeqMaxMlayerCnt == 1`, and
  `film_grain_params_present == 0`
- **AND** it requires frame dimensions, tile counts, decoded-frame bytes,
  reference-store bytes, hash bytes, and output bytes to pass `DecodeLimits`
  using checked arithmetic before allocation or output

#### Scenario: Contract defines accepted frame and tile shape

- **WHEN** a reader checks the minimal decode tier frame boundary
- **THEN** it accepts only closed-loop key-frame output whose parsed facts prove
  `obu_type == OBU_CLOSED_LOOP_KEY`, `FrameType = KEY_FRAME`, and
  `FrameIsIntra = 1`
- **AND** it requires inline frame headers with `cur_mfh_id == 0`,
  `frame_size_override_flag == 0`, `immediate_output_frame == 1`,
  `implicit_output_frame == 0`, and no sequence cropping window
- **AND** it requires a single tile with one first-and-only tile group
- **AND** it excludes open-loop key frames, RAS, switch, SEF/show-existing, TIP,
  bridge, inter frames, `INTRA_ONLY_FRAME`, multi-frame headers, multiple tiles,
  multiple tile groups, film grain application, quantizer-matrix-dependent
  decode, decoder-model scheduling, and unsupported tools without supported
  matrix rows and tests

#### Scenario: Contract defines success and rejection artifacts

- **WHEN** a future implementation proves a stream is inside the minimal tier
- **THEN** deterministic `splot-dfh-sha256-v1` frame hashes over cropped visible
  output samples are the first runtime success artifact
- **AND** runtime `splot decode` Y4M output remains unsupported until a
  byte-consuming decode/output row wires the `splot-recon` Y4M writer to real
  decoded frames and tests the CLI output path
- **AND** source-backed Y4M writing over caller-supplied `DecodedFrame<T>` values
  MAY be tracked separately by the `output-y4m` row without claiming runtime
  decode support
- **AND** streams outside the tier SHALL fail with structured
  `decode/unsupported-feature` diagnostics that identify the blocking matrix row
  where possible, while limit overflow or configured-limit excess SHALL use the
  emitted `decode/resource-limit` diagnostic when surfaced through `splot decode`

#### Scenario: Contract remains non-executable until implementation

- **WHEN** `minimal-decode-tier-contract` is still contract-only
- **THEN** the decoder support matrix marks the row as `partial`
- **AND** the row records self-contained docs/OpenSpec proof commands
- **AND** the row does not claim source implementation, runtime byte
  consumption, stream traversal, layer selection, reconstruction, runtime frame
  hashes, runtime Y4M output, fixture support, fuzz coverage, emitted new
  diagnostics, or mandatory AVM/dav2d execution

### Requirement: Decode hash output CLI contract

The `splot decode` CLI SHALL provide a hash-output selection contract before any
runtime decode path is marked supported. The contract SHALL be tracked by
Feature ID `CLI-DECODE-HASH-OUTPUT`. The CLI SHALL preserve
`splot decode <input> -o <output>` as the compatibility form for future Y4M
output, SHALL allow explicit `--output-format y4m`, and SHALL allow
`--output-format hash` without a Y4M output path. Until runtime decode support
lands, every valid `splot decode` invocation SHALL route the input through the
byte-planner handoff; if byte planning succeeds, it SHALL emit the runtime
`decode/unsupported-feature` diagnostic and exit with code `1`, and if byte
planning fails it SHALL emit the appropriate planner diagnostic. It SHALL avoid
output writes, decoded hash/Y4M production, and external decoder invocation.

#### Scenario: Compatibility Y4M form remains valid but unsupported

- **WHEN** `splot decode <input> -o <output>` is run before runtime decode
  support is implemented on input bytes that can be planned
- **THEN** it remains a valid CLI invocation
- **AND** it reads and byte-plans `<input>`
- **AND** it exits with code `1` and emits the runtime
  `decode/unsupported-feature` diagnostic
- **AND** it does not modify `<output>`, produce Y4M, compute a decoded-frame
  hash, or invoke an external decoder

#### Scenario: Explicit hash format is accepted without Y4M output

- **WHEN** `splot decode <input> --output-format hash` is run before runtime
  decode support is implemented on input bytes that can be planned
- **THEN** it is a valid CLI invocation
- **AND** it reads and byte-plans `<input>`
- **AND** it exits with code `1` and emits the runtime
  `decode/unsupported-feature` diagnostic
- **AND** it does not create any output file, compute a decoded-frame hash, or
  invoke an external decoder

#### Scenario: Explicit Y4M format still requires an output path

- **WHEN** `splot decode <input> --output-format y4m` is run without
  `-o/--output`
- **THEN** clap rejects the invocation as a usage error
- **AND** no `decode/unsupported-feature` runtime diagnostic is emitted

#### Scenario: Missing output selection remains a usage error

- **WHEN** `splot decode <input>` is run without `-o/--output` and without
  `--output-format`
- **THEN** clap rejects the invocation as a usage error
- **AND** no `decode/unsupported-feature` runtime diagnostic is emitted

#### Scenario: JSON mode remains diagnostic-only while unsupported

- **WHEN** `splot decode --json <input> --output-format hash` is run before
  runtime decode support is implemented on input bytes that can be planned
- **THEN** stdout contains the existing machine-readable
  `decode/unsupported-feature` diagnostic object
- **AND** stderr remains empty unless an operational error occurs
- **AND** no hash report schema or decoded-frame hash support is claimed

#### Scenario: Hash format refers to repository-owned decoded output hashes

- **WHEN** a reader checks the hash-output CLI contract
- **THEN** it identifies future hash output as `splot-dfh-sha256-v1` over
  decoded AV2 output samples in repository-owned emission-index order
- **AND** it does not describe AV2 `METADATA_TYPE_DECODED_FRAME_HASH`,
  `hash_type = 0` MD5, reserved AV2 hash types, OBU bytes, metadata payloads,
  parser facts, Y4M output, or AVM/dav2d execution as current `splot decode`
  output support

### Requirement: Portable local-reference evidence manifest

The repository SHALL provide a versioned, portable local-reference evidence
manifest for future decoder fixtures and hash comparisons. The manifest SHALL be
tracked by Feature ID `XTASK-LOCAL-REFERENCE-EVIDENCE-MANIFEST`. Manifest
entries SHALL be non-executable metadata only: they may record reference tool
identity, upstream revisions, sanitized command summaries, committed fixture
identity, decoded-output digests, and comparison assertions, but they SHALL NOT
require AVM, dav2d, ffmpeg, a network connection, or any external decoder to be
installed or executed. Manifest metadata SHALL NOT claim current `splot decode`
runtime support, reconstruction support, deterministic hash computation, Y4M
output, AV2 decoder conformance, or AV2 bitstream conformance.

#### Scenario: Manifest validates without external tools

- **WHEN** the local-reference evidence manifest is checked
- **THEN** the checker parses and validates only committed metadata and fixture
  bytes
- **AND** it does not locate, build, spawn, or require AVM, dav2d, ffmpeg, the
  network, or `splot decode`

#### Scenario: Manifest records portable fixture identity

- **WHEN** a manifest entry references a committed fixture
- **THEN** the fixture path is repo-relative, normalized, and points to an
  existing committed regular file
- **AND** the manifest records the fixture byte length and lowercase SHA-256
  digest
- **AND** the checker verifies both values from the committed fixture bytes

#### Scenario: Manifest rejects local machine state

- **WHEN** any manifest field contains a local absolute path, `file://` URL,
  home-relative path, Windows absolute path, local environment path token,
  executable path, or shell command composition syntax
- **THEN** the checker rejects the manifest
- **AND** the rejected metadata is not treated as portable evidence

#### Scenario: Manifest cross-references repository tracking

- **WHEN** a manifest entry names a Feature ID or decoder-support row
- **THEN** the checker verifies that the Feature ID exists in
  `docs/IMPLEMENTATION-MATRIX.toml`
- **AND** every decoder-support row exists in
  `docs/DECODER-SUPPORT-MATRIX.toml`

#### Scenario: Matrix evidence pointers resolve reciprocally

- **WHEN** a decoder-support row cites a local-reference evidence pointer of
  the form `docs/LOCAL-REFERENCE-EVIDENCE.toml::<evidence-id>`
- **THEN** `cargo xtask check-decoder-support` verifies that `<evidence-id>`
  exists in the committed manifest
- **AND** the referenced manifest entry lists the citing row in
  `decoder_support_rows`
- **AND** the check still does not locate, build, spawn, or require AVM, dav2d,
  ffmpeg, the network, `splot decode`, reconstruction, `splot` deterministic
  frame-hash computation, or Y4M output

#### Scenario: Manifest assertions are self-contained metadata

- **WHEN** a manifest entry records decoded-output digests or equality
  assertions from local reference tools
- **THEN** each digest field uses the declared algorithm and valid hex length
- **AND** each assertion references recorded digest IDs in the same evidence
  entry
- **AND** equality assertions compare the recorded metadata values only, without
  rerunning external tools

#### Scenario: Real reference agreement entries are portable metadata

- **WHEN** `docs/LOCAL-REFERENCE-EVIDENCE.toml` contains real `[[evidence]]`
  entries for local AVM/dav2d agreement
- **THEN** each entry records committed fixture identity, reference tool
  identity, upstream revisions, sanitized command summaries, raw
  reference-output digest metadata, and digest-equality assertions only
- **AND** the checker validates committed fixture size and SHA-256, known
  Feature IDs, known decoder-support rows, digest format, and equality between
  distinct reference runs
- **AND** no AVM, dav2d, ffmpeg, network access, `splot decode`,
  reconstruction, hash computation, or Y4M output is run or required

#### Scenario: Evidence entries do not upgrade decoder support

- **WHEN** a real local-reference evidence entry is added for a
  decoder-support row
- **THEN** the row may cite the evidence as `local_reference_evidence`
- **AND** the entry does not change the row status to `supported`
- **AND** it does not count as self-contained proof of runtime decode,
  reconstruction, deterministic hash digest computation, Y4M output, AV2
  decoder conformance, or AV2 bitstream conformance

#### Scenario: Archived raw MD5 agreement is transcribed

- **WHEN** archived local AVM/dav2d raw MD5 agreement is moved from prose into
  the manifest
- **THEN** each fixture is represented as a separate portable evidence entry
- **AND** `output_scope` labels the digest as reference raw decoder output, not
  `splot-dfh-sha256-v1`
- **AND** `command_summary` remains descriptive metadata, not a runnable shell
  command

### Requirement: Decoder crate scaffolding

The repository SHALL provide approved workspace crate scaffolds for future
decoder and reconstruction work. The scaffold SHALL be tracked by Feature ID
`INFRA-DECODER-CRATE-SCAFFOLDING`. `splot-recon` SHALL be the future home for
pixel buffers, deterministic decoded-frame hash primitives, reconstruction
primitives, and reference-frame storage. `splot-decode` SHALL be the future home
for the decoder driver that combines `splot-core` parsing with `splot-recon`
state. The scaffold SHALL NOT claim runtime AV2 decode, reconstruction,
deterministic hash, Y4M output, bit-exact output, or AV2 conformance support.

#### Scenario: Scaffolds build without runtime API claims

- **WHEN** the workspace is checked
- **THEN** `crates/splot-recon` and `crates/splot-decode` build as library
  crates with crate-level documentation and workspace lint inheritance
- **AND** they do not expose public placeholder reconstruction or decode APIs
  merely to prove the crates exist

#### Scenario: Decode behavior remains unsupported

- **WHEN** `splot decode` runs after the scaffold is added
- **THEN** it keeps the existing structured unsupported diagnostic behavior
- **AND** no input bytes are decoded, no output is written, and no external
  decoder is located or invoked

#### Scenario: Crate support status stays honest

- **WHEN** decoder support status is rendered
- **THEN** the scaffold row is represented as repository infrastructure
- **AND** codec decode stages remain `todo`, `partial`, `blocked`, or
  `unsupported-intentional` according to their own proof, not because the crates
  exist

### Requirement: Reference frame store runtime model

The repository SHALL provide a source-backed `splot-recon` reference-frame-store
runtime model for future decoder and encoder closed-loop reuse. The model SHALL
store immutable caller-owned frame payload values in typed zero-based
`ReferenceSlot` positions without requiring output-emission metadata.
`ReferenceSlot::MAX_SLOTS` SHALL be `16`, matching the AV2 § 3
`NUM_REF_FRAMES` slot ceiling that motivates § 7.23 storage, while active
sequence reference counts and `RefValid` state remain future decoder
responsibilities. The model SHALL validate slot construction, store capacity,
and slot bounds before access; support replacement, clearing, immutable lookup,
occupancy reporting, and stable slot-order iteration; and report failures
through typed `ReconError` values. The model SHALL cite AV2 § 3 for the slot
ceiling and AV2 § 7.23 as the reference-frame-storage motivation while making
clear that AV2 reference refresh, prediction, frame selection, output
scheduling, and byte-consuming decode semantics remain future work.

#### Scenario: Store rejects invalid capacity

- **WHEN** a caller constructs a `ReferenceSlot` above `ReferenceSlot::MAX_SLOTS`
  or creates a reference-frame store with zero capacity or capacity above the
  source-backed slot ceiling
- **THEN** construction returns a typed `ReconError` and no store is created

#### Scenario: Store validates slot bounds

- **WHEN** a caller reads, writes, or clears a slot outside the store capacity
- **THEN** the operation returns a typed `ReconError` without panicking

#### Scenario: Store replaces immutable frames

- **WHEN** a caller puts an immutable frame payload into an empty valid slot and
  then puts another frame into the same slot
- **THEN** the first operation reports no previous frame
- **AND** the second operation returns the previous immutable frame payload
- **AND** later lookup returns the replacement frame

#### Scenario: Store iterates occupied slots in slot order

- **WHEN** a store contains frames in multiple non-contiguous slots
- **THEN** iteration returns only occupied slots in ascending `ReferenceSlot`
  order with immutable frame borrows

#### Scenario: Runtime model does not claim full AV2 refresh semantics

- **WHEN** a reader checks the decoder roadmap and support matrix
- **THEN** the reference-frame-store row states that the source-backed API is a
  safe runtime storage model only
- **AND** byte-consuming decode, AV2 reference refresh semantics,
  `RefValid`/output scheduling, `decode/resource-limit` emission, AVM/dav2d
  invocation, and CI reference-tool requirements remain unsupported

### Requirement: Decoded frame hash input serialization

The repository SHALL provide source-backed `splot-recon` APIs that serialize
canonical decoded-frame hash input bytes from a validated caller-supplied
`DecodedFrame<T>` and compute the repository-owned `splot-dfh-sha256-v1`
decoded-frame digest. The byte stream SHALL use identifier
`av2-output-samples-v1` and raw output variant `raw_intermediate_output`. The
serializer and digest computation SHALL follow AV2 § 6.16.13 sample-byte
conversion for the frame's modeled visible output rows: visible samples only, Y
then U then V plane order for non-monochrome frames, Y only for monochrome
frames, raster scan order within each plane, one byte per 8-bit sample, and
little-endian two-byte values for samples with bit depth greater than 8. The
serializer and digest computation SHALL exclude stride padding, backing
allocation padding, output index, frame dimensions, pixel format metadata, OBU
bytes, container metadata, and decoded-frame-hash metadata from the byte stream
and digest. The digest API SHALL expose stable algorithm, byte-stream, and
variant identifiers, raw 32-byte digest access, and lowercase hex formatting.
AV2 metadata MD5 verification, byte-consuming decode, output ordering,
film-grain synthesis, Y4M output, AVM/dav2d invocation, and CI reference-tool
requirements remain future work.

#### Scenario: Visible rows exclude padding

- **WHEN** a decoded frame stores non-visible padding or stride samples around a
  visible crop rectangle
- **THEN** hash input serialization writes only the visible samples in raster
  order
- **AND** padding and stride samples do not appear in the output bytes
- **AND** `splot-dfh-sha256-v1` is computed over the same visible bytes only

#### Scenario: Monochrome and chroma plane order

- **WHEN** a decoded frame is monochrome
- **THEN** hash input serialization writes only Y-plane bytes
- **AND** `splot-dfh-sha256-v1` hashes only those Y-plane bytes
- **WHEN** a decoded frame has chroma planes
- **THEN** hash input serialization writes Y bytes, then U bytes, then V bytes
- **AND** `splot-dfh-sha256-v1` hashes that same Y/U/V byte order

#### Scenario: Sample byte width follows bit depth

- **WHEN** a decoded frame has 8-bit output samples
- **THEN** hash input serialization writes one byte per visible sample
- **AND** `splot-dfh-sha256-v1` hashes those one-byte sample values
- **WHEN** a decoded frame has greater-than-8-bit output samples
- **THEN** hash input serialization writes each visible sample as two
  little-endian bytes
- **AND** `splot-dfh-sha256-v1` hashes those little-endian sample bytes

#### Scenario: Byte length matches emitted bytes

- **WHEN** a caller asks for the hash input byte length and writes the same frame
  to an in-memory byte buffer
- **THEN** the checked byte length equals the number of emitted bytes

#### Scenario: Writer errors are propagated

- **WHEN** the caller-provided writer returns an error while receiving hash
  input bytes
- **THEN** serialization returns that writer error without panicking

#### Scenario: Digest identifiers and hex formatting are stable

- **WHEN** a caller computes a decoded-frame digest
- **THEN** the digest reports contract identifier `splot.decoded_frame_hash`
- **AND** it reports contract version `1`
- **AND** it reports algorithm identifier `splot-dfh-sha256-v1`
- **AND** it is tied to byte-stream identifier `av2-output-samples-v1`
- **AND** it is tied to variant identifier `raw_intermediate_output`
- **AND** raw digest access returns exactly 32 bytes
- **AND** text formatting returns exactly 64 lowercase hexadecimal characters

#### Scenario: Digest matches canonical byte stream

- **WHEN** a caller computes a decoded-frame digest and also writes the same
  frame through the canonical hash-input serializer
- **THEN** the digest equals SHA-256 over the emitted canonical byte stream

#### Scenario: Runtime model does not claim decode output

- **WHEN** a reader checks the decoder roadmap and support matrix
- **THEN** the deterministic-frame-hash row states that source-backed hash input
  serialization and `splot-dfh-sha256-v1` digest computation exist for
  caller-supplied decoded frames
- **AND** AV2 metadata MD5 verification, byte-consuming decode, output ordering,
  film-grain synthesis, Y4M output, AVM/dav2d invocation, and CI reference-tool
  requirements remain unsupported

### Requirement: Decoder runtime concurrency contract

The decoder support model SHALL incorporate the repository runtime concurrency
policy tracked by `INFRA-PARALLEL-RUNTIME-POLICY` before any byte-consuming
decode, reconstruction, deterministic frame-hash, Y4M-output, reference-update,
or encoder roundtrip row is marked supported. Future decoder work SHALL use the
single approved `splot_parallel` model: one `WorkerPool` owned by each
`splot-decode` context, parallel iterators reached through
`splot_parallel::prelude` and driven inside `WorkerPool::install`, and bounded
queues only through `splot_parallel::bounded_queue` at coarse pipeline
boundaries. `splot-recon` SHALL remain pool-agnostic reconstruction and data
model infrastructure; it MUST NOT construct worker pools, use direct Rayon or
crossbeam APIs, spawn codec worker threads, or own pipeline queues. Observable
decoder output SHALL remain deterministic across `--threads 1`,
`--threads auto`, and fixed positive `--threads N`.

#### Scenario: Decoder roadmap documents the runtime policy

- **WHEN** a reader opens `docs/DECODER-ROADMAP.md`
- **THEN** the roadmap links future decoder/reconstruction work to
  `INFRA-PARALLEL-RUNTIME-POLICY` and `docs/CONCURRENCY.md`
- **AND** it states that `splot-decode` owns runtime orchestration through a
  single context-owned `WorkerPool`
- **AND** it states that `splot-recon` remains pool-agnostic and reusable by the
  future encoder

#### Scenario: Future parallel decode work uses the context pool

- **WHEN** future decode or reconstruction orchestration adds data-parallel work
- **THEN** it MUST reach parallel iterator traits through
  `splot_parallel::prelude`
- **AND** it MUST run those iterators inside the owning decode context's
  `WorkerPool::install`
- **AND** it MUST NOT build a nested pool, initialize the Rayon global pool,
  spawn ad-hoc codec worker threads, or depend on `rayon` outside
  `splot-parallel`

#### Scenario: Reconstruction primitives stay pool-agnostic

- **WHEN** `splot-recon` gains reconstruction, reference, hash, or output helper
  APIs
- **THEN** those APIs MUST remain callable without constructing or owning a
  worker pool
- **AND** any parallel scheduling wrapper MUST live in `splot-decode` or another
  caller that already owns the context runtime policy
- **AND** `splot-recon` MUST NOT depend directly on `rayon`,
  `crossbeam-channel`, or another runtime/channel crate

#### Scenario: Queues are bounded coarse pipeline boundaries

- **WHEN** future byte-consuming decode needs a producer/consumer boundary
- **THEN** it MUST use `splot_parallel::bounded_queue` with an explicit
  `QueueCapacity`
- **AND** it MUST NOT use unbounded channels, `std::sync::mpsc`, or queues for
  per-pixel, per-block, per-row, or other hot inner-loop signalling

#### Scenario: Decode output is deterministic across thread counts

- **WHEN** a future decode row claims runtime support for decoded-frame hashes,
  Y4M output, diagnostics, stats, reference updates, or another observable
  artifact
- **THEN** its proof MUST include self-contained evidence that observable output
  is committed in AV2 bitstream, presentation, or repository-owned emission
  order rather than worker completion order
- **AND** its tests MUST cover the supported behavior across all required
  thread-count forms: `--threads 1`, `--threads auto`, and at least one fixed
  positive `--threads N`

#### Scenario: Current unsupported decode remains honest

- **WHEN** this contract is added before runtime decode support exists
- **THEN** `splot decode` continues to emit the runtime
  `decode/unsupported-feature` diagnostic for byte-planner-successful
  invocations
- **AND** newly emitted diagnostics remain limited to the byte-planner handoff
  diagnostics for malformed sources, planner resource limits, and unsupported
  planner structures
- **AND** no tile-decoding runtime path, reconstruction algorithm, frame-hash
  digest computation, Y4M output, AVM/dav2d invocation, or decoded-output
  diagnostic is claimed

### Requirement: Parsed decode stream planner

The decoder support model SHALL provide a plan-only stream traversal API for
`DECODE-STREAM-STATE-PLANNER` in `splot-decode`. The planner SHALL be owned by
`DecodeContext`, consume already parsed `splot_core::stream::ParsedBitstream`
values plus caller-supplied input length, apply `DecodeOptions`, and return a
deterministic ordered plan for the selected base-layer stream. The planner
SHALL NOT accept raw bytes, read files, change `splot decode` CLI behavior,
decode tile payloads, reconstruct pixels, compute hashes, write Y4M, refresh
references, invoke AVM/dav2d, or add source/build/test/CI integration for
external decoders.

The initial planner SHALL select only the base minimal-tier layer: non-global
OBUs must have `obu_xlayer_id == 0`, `obu_tlayer_id == 0`, and
`obu_mlayer_id == 0`. It SHALL also enforce AV2 § 6.2.2 global/local xlayer
constraints for OBU types that require or forbid `GLOBAL_XLAYER_ID`. It SHALL
preserve AV2 bitstream/container order for raw Annex B and IVF-wrapped Annex B
parser output, including byte offsets and IVF frame context where present. It
SHALL treat `OBU_CLOSED_LOOP_KEY`, `OBU_REGULAR_TILE_GROUP`, and
`OBU_REGULAR_TIP` as selected frame candidates in this slice, and SHALL reject
multistream/layer-selection structures, invalid xlayer scope, non-base layers,
unsupported frame-carrying OBUs, malformed parsed sources, and resource-limit
failures transactionally.

The planner SHALL enforce only the resource limits it can derive honestly from
the parsed stream model: `max_input_bytes` before planner traversal,
`max_obus` before adding the next planned OBU, `max_ivf_frame_records` before
traversing the next IVF frame record, and `max_frames_to_decode` before
accepting the next selected frame candidate. Raw-byte traversal is
specified separately by the byte-consuming decode stream planner requirement.

#### Scenario: Raw Annex B is planned in order

- **WHEN** `DecodeContext::plan_stream` receives a parsed raw Annex B stream
  containing accepted base-layer OBUs
- **THEN** it returns a `DecodeStreamPlan` whose format is Annex B
- **AND** planned OBU records appear in original bitstream order with stable
  OBU indexes, byte offsets, sizes, headers, and roles
- **AND** no payload bytes, decoded frames, hashes, Y4M output, or reference
  updates are exposed as supported output

#### Scenario: IVF Annex B is planned with frame context

- **WHEN** `DecodeContext::plan_stream` receives a parsed IVF stream whose frame
  payloads contain accepted Annex B OBUs
- **THEN** it returns a `DecodeStreamPlan` whose format is IVF
- **AND** planned OBU records preserve source order, absolute byte offsets, IVF
  frame index, PTS, and frame payload offset metadata
- **AND** IVF warnings remain source metadata rather than decode success or
  external reference evidence

#### Scenario: Malformed parsed source is transactional

- **WHEN** raw Annex B parsing, IVF container parsing, or an IVF frame payload
  parse recorded an error in the supplied `ParsedBitstream`
- **THEN** `DecodeContext::plan_stream` returns a typed malformed-source error
- **AND** it returns no partial `DecodeStreamPlan`

#### Scenario: Planner resource limits are enforced

- **WHEN** the supplied input length exceeds `max_input_bytes`, traversed OBU
  count would exceed `max_obus`, traversed IVF frame records would exceed
  `max_ivf_frame_records`, or accepted selected frame candidates would
  exceed `max_frames_to_decode`
- **THEN** `DecodeContext::plan_stream` returns a typed local limit error
- **AND** it returns no partial `DecodeStreamPlan`
- **AND** it does not itself emit the `decode/resource-limit` CLI diagnostic

#### Scenario: Unsupported structures are rejected

- **WHEN** the parsed stream contains an invalid global/local xlayer binding,
  non-base-layer OBU, MSDO, layer configuration record, atlas segment,
  operating point set, non-closed-loop frame-carrying OBU,
  metadata/output-effect OBU, reserved OBU, or another structure outside the
  minimal planner tier
- **THEN** `DecodeContext::plan_stream` returns a typed unsupported-structure
  error linked to rule id `decode/unsupported-feature`, matrix row
  `decode-stream-state`, Feature ID `DECODE-STREAM-STATE-PLANNER`, and the
  relevant AV2 section where applicable
- **AND** it returns no partial `DecodeStreamPlan`

#### Scenario: Planning is deterministic across thread policies

- **WHEN** the same parsed stream is planned through `DecodeContext` configured
  with `--threads 1`, `--threads auto`, and a fixed positive thread count
- **THEN** the returned plan metadata is identical
- **AND** planner implementation does not use direct Rayon, direct crossbeam,
  a global pool, ad-hoc codec worker threads, or unbounded queues

#### Scenario: CLI runtime remains intentionally unsupported

- **WHEN** the planner APIs exist before runtime CLI decode output support
- **THEN** `splot decode` may read and plan input bytes through
  `DecodeContext::plan_bytes`
- **AND** successful planning still emits the runtime `decode/unsupported-feature`
  diagnostic rather than decode success
- **AND** it does not write output or invoke AVM/dav2d

### Requirement: Byte-consuming decode stream planner

The decoder support system SHALL provide a source-backed byte-consuming stream
planning entrypoint tracked by Feature ID `DECODE-BYTE-STREAM-PLANNER`. The
entrypoint SHALL accept raw AV2 Annex B length-delimited bytes and IVF/DKIF
bytes whose frame payloads contain Annex B bytes, SHALL return the existing
`DecodeStreamPlan` type, and SHALL preserve the `decode-stream-state` base-layer
selection policy. It SHALL not reconstruct pixels, decode tile payloads, compute
hashes, write Y4M, invoke external decoders, or change `splot decode` CLI
success behavior.

#### Scenario: Raw Annex B bytes produce the same bounded plan

- **WHEN** the byte-consuming planner receives a complete raw Annex B input that
  contains only structures accepted by `decode-stream-state`
- **THEN** it returns a `DecodeStreamPlan` with source format `annex_b`
- **AND** planned OBUs retain source order, byte offsets, declared OBU size,
  payload length, parsed OBU header metadata, and planner roles
- **AND** the plan is equivalent to planning the same bytes through the existing
  parsed-input planner for representative self-contained tests

#### Scenario: IVF bytes preserve frame context

- **WHEN** the byte-consuming planner receives a complete IVF/DKIF input whose
  frame payloads are accepted Annex B payloads
- **THEN** it returns a `DecodeStreamPlan` with source format `ivf`
- **AND** each planned OBU from an IVF frame records the IVF frame index, frame
  header offset, frame payload offset, declared frame payload size, and PTS
  metadata
- **AND** IVF timestamps are preserved only as container metadata and are not
  used for output scheduling or media-player behavior

#### Scenario: Limits are enforced during byte traversal

- **WHEN** the byte-consuming planner receives configured `DecodeLimits`
- **THEN** it checks `max_input_bytes` before traversing bytes
- **AND** it checks `max_obus` before retaining the next OBU
- **AND** it checks `max_ivf_frame_records` before processing the next IVF frame
  record
- **AND** it checks `max_frames_to_decode` before retaining the next accepted
  frame candidate
- **AND** a limit failure returns a typed `DecodeError::Limit` and no partial
  plan

#### Scenario: Malformed bytes are transactional

- **WHEN** raw Annex B bytes, IVF container bytes, or Annex B bytes inside an IVF
  frame payload are malformed
- **THEN** the byte-consuming planner returns `DecodeError::MalformedSource`
- **AND** the error records a `DecodeSourceIssue` with the source category,
  offset when known, IVF frame index when frame-local, and parser message
- **AND** no partial plan is returned

#### Scenario: Unsupported structures stay structured

- **WHEN** the byte-consuming planner encounters a non-base layer, invalid
  global/local layer scope, unsupported multistream/external-HLS structure,
  reserved OBU type, non-CLK frame-carrying OBU, or output-affecting OBU outside
  the initial planner tier
- **THEN** it returns `DecodeError::UnsupportedStructure`
- **AND** the unsupported metadata uses rule id `decode/unsupported-feature`,
  matrix row `decode-stream-state`, and Feature ID
  `DECODE-STREAM-STATE-PLANNER`
- **AND** the `splot decode` CLI adapter renders the unsupported metadata as a
  structured user-facing diagnostic without changing the planner metadata

#### Scenario: Byte planner is fuzzed without external decoders

- **WHEN** the repository fuzz smoke is run for decoder entrypoints
- **THEN** the byte-consuming stream planner has a finite-limit fuzz target that
  feeds arbitrary bytes through `DecodeContext::plan_bytes`
- **AND** the target does not require AVM, dav2d, network access, generated
  external fixtures, or checked-in local reference paths

#### Scenario: Runtime concurrency model is preserved

- **WHEN** callers use the byte-consuming planner with thread count `1`, `auto`,
  or a fixed non-zero count
- **THEN** planning executes inside `DecodeContext`'s single owned
  `splot_parallel::WorkerPool`
- **AND** plan records, errors, and source issue ordering are deterministic
  across thread counts
- **AND** no direct Rayon, crossbeam, global worker pool, ad-hoc worker thread,
  or decode pipeline queue is introduced

### Requirement: Decode byte-planner CLI handoff

The `splot decode` CLI SHALL enforce finite default `max_input_bytes` before
constructing a full input buffer, construct a `DecodeContext` from the requested
thread policy for inputs within that bound, and call `DecodeContext::plan_bytes`
with finite default `DecodeOptions` before reporting byte-planner diagnostics.
The CLI SHALL NOT duplicate byte parsing or stream planning logic, call
`WorkerPool::install` directly, spawn threads, use global pools, or add
concurrency dependencies.

#### Scenario: Malformed source emits structured diagnostic

- **WHEN** `splot decode` reads malformed raw Annex B bytes, a malformed IVF
  container, or malformed Annex B inside IVF
- **THEN** it exits with code `1`
- **AND** it emits `decode/malformed-source` with severity `Error`, matrix row
  `decode-byte-stream-planner`, Feature ID `DECODE-BYTE-STREAM-PLANNER`,
  source issue kind, parser rule ID when known, byte offset when known, IVF
  frame index when known, and parser message
- **AND** it leaves `spec_section` unset when the parser issue cannot be cited
  to one precise AV2 section
- **AND** the requested output path is not created, truncated, or written

#### Scenario: Thread policy uses decode context

- **WHEN** `splot decode --threads auto`, `--threads 1`, or another fixed
  positive thread count is run on the same diagnostic-producing input
- **THEN** each invocation reaches the same `DecodeContext::plan_bytes`
  diagnostic result
- **AND** the CLI code does not introduce direct Rayon, crossbeam, global-pool,
  queue, or ad-hoc thread usage outside `splot_parallel`

#### Scenario: Prior byte planner review feedback stays protected

- **WHEN** tests exercise byte planning after the CLI handoff
- **THEN** unsupported structures keep precedence over later traversal limits,
  fatal IVF first-frame header errors remain retry-stable, `decode_plan_bytes`
  fuzz seeds include prefixed valid fixture paths, and `DecodeContext` docs
  accurately describe raw-byte planning

### Requirement: Byte planner review regressions stay fixed

The byte-consuming decode stream planner SHALL preserve the diagnostics,
cursor contracts, fuzz smoke coverage, and public documentation promised by
Feature ID `DECODE-BYTE-STREAM-PLANNER`.

#### Scenario: Unsupported prefix wins over later traversal limits

- **WHEN** raw Annex B byte traversal has retained an OBU prefix that the
  existing stream planner classifies as `DecodeError::UnsupportedStructure`
- **THEN** `DecodeContext::plan_bytes` preserves that unsupported-structure
  error while continuing transactional byte traversal
- **AND** a later `max_obus` or `max_frames_to_decode` failure does not mask the
  earlier unsupported prefix

#### Scenario: Malformed suffix wins over earlier unsupported prefix

- **WHEN** raw Annex B byte traversal has retained an OBU prefix that the
  existing stream planner classifies as `DecodeError::UnsupportedStructure`
- **AND** later bytes in the same Annex B payload are malformed
- **THEN** `DecodeContext::plan_bytes` returns `DecodeError::MalformedSource`
- **AND** the malformed parser error is not masked by the earlier unsupported
  prefix

#### Scenario: Malformed IVF frame payload wins over earlier unsupported frame

- **WHEN** IVF byte traversal has retained an earlier frame payload containing
  an OBU that the existing stream planner classifies as
  `DecodeError::UnsupportedStructure`
- **AND** a later IVF frame payload contains malformed Annex B bytes
- **THEN** `DecodeContext::plan_bytes` returns `DecodeError::MalformedSource`
- **AND** the source issue records `IvfFramePayloadError` for the malformed
  later frame

#### Scenario: Parsed IVF OBU limits win before later payload errors

- **WHEN** parsed IVF planning has traversed complete earlier frame payload OBUs
  that exceed `max_obus` or `max_frames_to_decode`
- **AND** a later IVF frame payload contains malformed Annex B bytes
- **THEN** `DecodeContext::plan_stream` returns `DecodeError::Limit` for the
  first exceeded OBU traversal or frame-candidate limit
- **AND** the later payload parse error does not mask that already reached
  limit failure

#### Scenario: IVF frame-record limit stays typed after earlier unsupported frame

- **WHEN** IVF byte traversal has retained an earlier frame payload containing
  an OBU that the existing stream planner classifies as
  `DecodeError::UnsupportedStructure`
- **AND** a later complete IVF frame record exceeds `max_ivf_frame_records`
- **THEN** `DecodeContext::plan_bytes` returns `DecodeError::Limit`
- **AND** the limit source name is `DecodeLimitName::MaxIvfFrameRecords`
- **AND** the unsupported-prefix carve-out remains scoped to `max_obus` and
  `max_frames_to_decode`

#### Scenario: IVF cursor retry preserves fatal frame-header errors

- **WHEN** `IvfFrameCursor::next_frame_record()` returns a fatal IVF
  frame-header error
- **THEN** retrying the same public cursor returns the same fatal error
- **AND** the cursor does not advance to `End` or a warning state before the
  caller observes the retry

#### Scenario: Decode byte planner fuzz seeds exercise valid traversal

- **WHEN** CI seeds fuzz corpora from committed AV2 fixtures
- **THEN** the `decode_plan_bytes` corpus receives flag-prefixed variants
  because that target consumes byte zero as limit flags
- **AND** those variants preserve the original fixture bytes as the bitstream
  payload passed to `DecodeContext::plan_bytes`

#### Scenario: Decode context docs match byte planning API

- **WHEN** generated API docs describe `DecodeContext`
- **THEN** they state that the context owns byte-consuming and parsed-stream
  planning entry points
- **AND** they do not claim the context avoids raw byte traversal
- **AND** they continue to state that filesystem I/O, reconstruction, output
  writing, and external decoder invocation remain unsupported

### Requirement: Reconstruction Y4M output writer

The decoder support model SHALL provide a source-backed Y4M writer in
`splot-recon` for caller-supplied `DecodedFrame<T>` values. The writer SHALL be
tracked by Feature ID `RECON-Y4M-OUTPUT-WRITER`, SHALL use AV2-derived decoded
output facts from § 6.4.1, § 6.17.4.4, § 7.21.1, and § 7.21.2, and SHALL treat
Y4M container syntax and chroma tags as repository-owned output policy outside
the AV2 specification. The writer SHALL NOT read AV2 bitstreams, select layers,
decode tile payloads, reconstruct pixels, apply film grain, schedule output
order, refresh references, invoke AVM/dav2d, or change `splot decode` CLI
runtime behavior.

#### Scenario: Stream header uses visible output format

- **WHEN** a caller builds a Y4M stream header from a decoded frame and a valid
  nonzero frame rate
- **THEN** the header uses the frame's visible luma width and height
- **AND** it uses progressive `Ip` output
- **AND** it derives the Y4M chroma tag from the frame's `BitDepth` and
  `PixelFormat`
- **AND** it does not use coded padding, reference-store dimensions, OBU bytes,
  IVF timestamps, output index, AV2 metadata, AVM, or dav2d to construct the
  header

#### Scenario: Invalid frame rate is rejected

- **WHEN** a caller supplies a zero frame-rate numerator or denominator
- **THEN** the Y4M API rejects the configuration with a typed error
- **AND** no Y4M stream header or frame bytes are written

#### Scenario: Frame payload uses visible rows

- **WHEN** a caller writes a decoded frame to Y4M
- **THEN** the frame payload serializes only cropped visible output samples
- **AND** storage stride, coded padding, reference-frame padding, and output
  metadata are excluded
- **AND** non-monochrome frames write Y bytes, then U bytes, then V bytes
- **AND** monochrome frames write only Y bytes

#### Scenario: Sample byte serialization is pinned

- **WHEN** a decoded frame has 8-bit output samples
- **THEN** each visible sample is written as one byte
- **WHEN** a decoded frame has 10-bit output samples
- **THEN** each visible sample is written as two little-endian bytes without
  normalization or scaling

#### Scenario: Stream rejects mismatched frames before payload output

- **WHEN** a caller tries to append a frame whose visible size, bit depth, or
  pixel format differs from the Y4M stream header
- **THEN** the writer returns a typed stream-parameter mismatch error
- **AND** it does not write `FRAME\n` or any frame payload bytes for the
  mismatched frame

#### Scenario: Writer errors are propagated

- **WHEN** the caller-provided output writer returns an I/O error while receiving
  the Y4M stream header, frame header, or frame payload
- **THEN** the Y4M writer returns that I/O error without panicking

#### Scenario: Runtime decode output remains unsupported

- **WHEN** a reader checks the decoder roadmap and support matrix after this
  writer is implemented
- **THEN** the `output-y4m` row states that source-backed Y4M writing exists only
  for caller-supplied decoded frames
- **AND** runtime `splot decode -o` Y4M output, byte-consuming decode,
  reconstruction algorithms, output scheduling, film-grain synthesis, AVM/dav2d
  invocation, and CI reference-tool requirements remain unsupported

### Requirement: AV2 symbol decoder foundation

The decoder support model SHALL provide a bounded `splot-core` AV2 § 8.2 symbol
decoder primitive tracked by Feature ID `AV2-8.2-SYMBOL-DECODER`, and SHALL mark
its `symbol-decoder` support row `supported`. The primitive SHALL implement the
generic symbol-decoder operations over a caller-provided tile payload byte
slice: `init_symbol(sz)` (§ 8.2.2), `read_bool()` (§ 8.2.3), `read_literal(n)`
(§ 8.2.5), `read_symbol(cdf)` arithmetic decoding and CDF adaptation (§ 8.2.6),
and the `exit_symbol()` trailing-bit/zero-padding conformance check (§ 8.2.4).
It SHALL use generated repository-owned § 9.2 conversion tables, SHALL validate
caller-supplied CDF rows before indexing or updating them, and SHALL return
typed `splot-core` errors rather than panicking on any input. The promoted row
SHALL claim only this primitive; the § 8.2.4 CDF copy/averaging and the § 8.2.2
"Tile" CDF-array copy SHALL remain owned by `tile-cdf-save-lifecycle-boundary`,
and § 8.3 syntax-element CDF selection, default § 9.3 CDF banks,
`decode_tile()`, tile syntax traversal, reconstruction, hash output, Y4M output,
reference refresh, AVM/dav2d invocation, and any new dependency/scheduler
surface SHALL remain tracked in their own rows.

#### Scenario: Initialization is tile-slice bounded

- **WHEN** a caller creates a symbol decoder over a finite tile payload byte
  slice
- **THEN** initialization follows AV2 § 8.2.2 using at most the first 15 coded
  bits, `SymbolRange = 1 << 15`, and signed `SymbolMaxBits = 8 * sz - 15`
- **AND** empty, one-byte, multi-byte, and large synthetic `sz` cases do not
  overflow or panic
- **AND** the decoder reads only from the provided tile payload slice, not from a
  parent OBU, IVF, or Annex B reader

#### Scenario: Boolean and literal reads are deterministic

- **WHEN** a caller reads pseudo-raw bits with `read_bool()` or `read_literal(n)`
- **THEN** boolean renormalization follows AV2 § 8.2.3, including implicit zero
  padding when `SymbolMaxBits` is negative
- **AND** literal reads follow AV2 § 8.2.5 by composing exactly `n`
  `read_bool()` values in MSB-first order
- **AND** literal widths outside the bounded implementation range are rejected
  with a typed error before unbounded work occurs

#### Scenario: CDF rows are validated before symbol decoding

- **WHEN** a caller passes a mutable CDF row to `read_symbol(cdf)`
- **THEN** the row is checked for a supported AV2 § 8.2.6 length, non-decreasing
  cumulative values, valid probability range, valid adaptation-rate index, and
  valid capped use count before any generated-table indexing occurs
- **AND** adjacent cumulative entries MAY be equal because AV2 § 8.2.6 adaptation
  can drive them equal, so only a strict decrease is rejected (the threshold loop
  still separates the affected symbols through `Prob_Inc`)
- **AND** invalid rows return typed CDF errors without changing decoder state or
  mutating the row

#### Scenario: Symbol reads update CDFs only when enabled

- **WHEN** `read_symbol(cdf)` decodes a symbol from a valid row
- **THEN** it follows AV2 § 8.2.6 arithmetic renormalization using the generated
  `Prob_Inc` table
- **AND** it increments the frame-symbol count by one
- **AND** it updates CDF cumulative values and caps the row count at 32 when CDF
  update is enabled
- **AND** it leaves the CDF row byte-for-byte unchanged when CDF update is
  disabled

#### Scenario: Exit validates tile padding

- **WHEN** a caller finishes symbol decoding for a tile payload
- **THEN** `exit_symbol()` enforces the AV2 § 8.2.4 `SymbolMaxBits >= -14`
  requirement
- **AND** it validates the required trailing one bit and every zero padding bit
  up to byte alignment inside the tile payload
- **AND** malformed exit state, missing trailing one bit, and nonzero padding
  return typed errors without panicking

#### Scenario: Symbol decoding is proven across all arities and update rates

- **WHEN** the symbol decoder primitive is exercised by its test suite
- **THEN** every arity `N = 2..8` is decoded, with a maximal `SymbolValue`
  selecting symbol 0 and a zero `SymbolValue` selecting symbol `N-1`
- **AND** `read_symbol(cdf)` over random valid CDF rows of every arity always
  returns a symbol in `[0, N)`, keeps post-update entries in the valid
  probability range with a count capped at 32, and is deterministic across
  fresh decoders
- **AND** the minimum and maximum § 8.2.6 adaptation rates produce the exact
  hand-verified post-update rows
- **AND** decoding many symbols past the end of a tiny payload drives
  `SymbolMaxBits` deeply negative without panicking and with deterministic
  implicit zero padding

#### Scenario: Broader symbol and CDF work stays in its own rows

- **WHEN** a reader checks decoder support after the symbol decoder primitive is
  marked supported
- **THEN** the `symbol-decoder` row states that the generic AV2 § 8.2
  primitive is complete and proven
- **AND** § 8.3 CDF selection (`tile-cdf-selection-boundary`), the § 8.2.4 CDF
  copy/averaging and Tile/Saved CDF banks (`tile-cdf-save-lifecycle-boundary`),
  default § 9.3 banks, `decode_tile()`/traversal (`tile-payload-decode`), and
  broad reconstruction, decode hashing, Y4M/raw output, and CLI decode beyond
  the already-`supported` minimal tier (plus AVM/dav2d invocation) remain
  tracked in their own rows

### Requirement: Tile payload decode boundary

The decoder support model SHALL provide a source-backed tile payload decode
boundary tracked by Feature ID `DECODE-TILE-PAYLOAD-BOUNDARY` and decoder
support matrix row `tile-payload-decode`. The boundary SHALL consume bounded
tile-group payload framing metadata derived from AV2 § 5.20.1 and SHALL hand
each eligible non-bridge tile byte slice to the AV2 § 8.2 symbol-decoder
initialization boundary before stopping at the unsupported `decode_tile()` /
§ 8.3 syntax-element CDF-selection boundary. The boundary SHALL return structured
`decode/unsupported-feature` metadata for unsupported runtime tile syntax and
SHALL initially support only deterministic planning for the minimal single-tile
closed-loop-key boundary. It SHALL NOT claim multi-tile or multi-tile-group
runtime support, § 5.20.2-§ 5.20.10 block syntax, § 8.3 CDF bank ownership,
`exit_symbol()` validation after real block syntax, CDF copyback/averaging,
reconstruction, decoded-frame hashes, runtime Y4M output, reference refresh, or
AVM/dav2d execution support.

#### Scenario: Tile boundary enforces resource limits

- **WHEN** the tile payload boundary is asked to inspect a tile group with a
  tile count or tile payload byte count above the configured `DecodeLimits`
- **THEN** it fails before unbounded iteration, allocation, or symbol-decoder
  handoff
- **AND** the failure is represented as a typed decode resource-limit error
  that can render as `decode/resource-limit`

#### Scenario: Non-bridge tile reaches unsupported decode_tile boundary

- **WHEN** the boundary receives a non-bridge tile with a valid nonzero
  `tileSize` byte slice from § 5.20.1 framing
- **THEN** it bounds the slice to the framed tile bytes and verifies the
  AV2 § 8.2 `init_symbol(tileSize)` handoff point for that slice
- **AND** it stops before block syntax with `decode/unsupported-feature`
  metadata citing spec section `5.20.2.1`, matrix row
  `tile-payload-decode`, and Feature ID `DECODE-TILE-PAYLOAD-BOUNDARY`
- **AND** it does not reconstruct pixels, compute decoded-frame hashes, write
  Y4M output, run `exit_symbol()`, update CDF banks or reference frames, or
  invoke external decoders

#### Scenario: Minimal tier yields deterministic tile work unit

- **WHEN** the boundary is invoked for a selected base-layer closed-loop-key
  frame candidate with a complete intra first tile group, one tile, one tile
  group, and a bounded nonzero payload
- **THEN** it returns one deterministic tile work unit containing source kind,
  OBU index/offset, optional IVF frame context, selected layer, tile number,
  payload byte offset, and payload byte length
- **AND** the same work-unit metadata is produced for thread policies `auto`,
  `1`, and a fixed positive worker count when reached through `DecodeContext`

#### Scenario: Unsupported bridge or inactive tile path is explicit

- **WHEN** the boundary is asked to process multiple tiles, multiple tile groups,
  bridge, BRU-inactive, inter-only, non-first tile group, missing complete frame
  facts, or otherwise non-minimal-tier tile behavior
- **THEN** it returns structured `decode/unsupported-feature` metadata instead
  of silently treating the tile as a normal intra non-bridge tile
- **AND** the diagnostic identifies the unsupported tile payload boundary rather
  than the generic CLI runtime stub

#### Scenario: Symbol exit and CDF copyback are deferred

- **WHEN** the boundary reaches the point where AV2 § 5.20.1 would run
  `decode_tile()`, `exit_symbol()`, `frame_end_update_cdf()`, or
  `decode_frame_wrapup()`
- **THEN** it records those operations as unsupported residuals rather than
  mutating CDF banks, output state, or reference state
- **AND** tests prove this deferral without requiring AVM or dav2d

#### Scenario: Runtime decode remains unsupported outside the boundary

- **WHEN** `splot decode` is run on any stream after this change
- **THEN** the CLI still follows the existing plan-only unsupported behavior
  unless a later OpenSpec change wires the tile payload boundary into a full
  runtime decode path
- **AND** no AVM, dav2d, ffmpeg, or external decoder is located or invoked by
  repo code, tests, `xtask`, or CI

### Requirement: Tile CDF selection boundary

The decoder support model SHALL provide a crate-private tile CDF selection
boundary tracked by Feature ID `DECODE-TILE-CDF-SELECTION-BOUNDARY` and decoder
support matrix row `tile-cdf-selection-boundary`. The boundary SHALL copy a
small owned tile CDF subset from generated § 9.3 default tables, including the
partition-entry rows `DoSplitCdf`, `DoSquareSplitCdf`, `RectTypeCdf`,
`DoExtPartitionCdf`, and `DoUneven4wayPartitionCdf`; expose typed row selection
for § 8.3 `S` syntax-element handoff to `SymbolDecoder::read_symbol(cdf)`;
derive bounded § 8.3.2 contexts for `do_split`, `do_square_split`,
`rect_type`, `do_ext_partition`, and `do_uneven_4way_partition`; and record the
§ 8.2 frame-end CDF copy/average policy needed by a future tile-completion row.
The boundary SHALL identify the selected CDF rows consumed by the separate
`DECODE-TILE-PARTITION-SYMBOL-READ-BOUNDARY` row, but SHALL NOT claim partition
decisions, full § 8.3 CDF selection, full Tile/Saved CDF banks, recursive
`decode_tile()` / `read_partition()` traversal, `exit_symbol()` after real
syntax, CDF copyback/averaging mutation after tile completion, reconstruction,
decoded-frame hashes, runtime Y4M output, reference refresh, public API support,
AVM/dav2d invocation, or new scheduler/dependency support.

#### Scenario: Default CDF subset is source-backed

- **WHEN** the tile CDF boundary initializes its owned frame/tile CDF subset
- **THEN** `DoSplitCdf`, `DoSquareSplitCdf`, `RectTypeCdf`,
  `DoExtPartitionCdf`, and `DoUneven4wayPartitionCdf` rows are copied from
  generated `splot-core` default CDF tables derived from AV2 § 9.3
- **AND** no CDF values are hand-transcribed from the spec mirror or a reference
  implementation

#### Scenario: Typed selectors bound CDF row access

- **WHEN** a caller requests a supported CDF row through a tile CDF selector
- **THEN** the boundary validates the selector contexts before indexing
- **AND** it returns mutable row access only for the duration of a caller
  closure suitable for `SymbolDecoder::read_symbol(cdf)`
- **AND** out-of-range selector contexts return typed errors without panicking or
  mutating CDF state

#### Scenario: Symbol decoder handoff honors CDF update policy

- **WHEN** a selected row is passed to `SymbolDecoder::read_symbol(cdf)`
- **THEN** the row is mutated when the tile work unit's CDF update mode is
  enabled
- **AND** the row remains byte-for-byte unchanged when
  `disable_cdf_update == 1` selects disabled CDF updates

#### Scenario: Copy and average policy is recorded only

- **WHEN** the boundary is asked for the frame-end CDF policy for a tile
- **THEN** it computes `copyCdf` and `avgCdf` from `enable_avg_cdf`,
  `avg_cdf_type`, `context_update_tile_id`, `TileNum`, and `TileCols * TileRows`
  according to AV2 § 8.2
- **AND** it does not apply Saved CDF mutation, CDF averaging, or
  `frame_end_update_cdf()` support until a future row wires real tile
  completion and `exit_symbol()`

#### Scenario: Left and above partition contexts are bounded

- **WHEN** the tile CDF boundary derives contexts for `do_split`, `rect_type`,
  `do_ext_partition`, or `do_uneven_4way_partition`
- **THEN** every `bSize`, `PlaneStart`, `r`, `c`, second-half extended
  partition offset, and neighbor block-size lookup is bounds-checked before use
- **AND** invalid indexes return crate-private typed errors instead of panicking
- **AND** the resulting context is checked against the selected CDF array before
  row access

#### Scenario: Square split context is bounded

- **WHEN** the tile CDF boundary derives the `do_square_split` context
- **THEN** `PlaneStart` is bounded to the AV2 § 8.3.2 square-split plane,
  `bSize` is checked against the generated § 9.2 conversion tables, `AvailU`
  gates the `MiSizes[0][r - 1][c]` lookup, and `AvailL` gates the
  `MiSizes[0][r][c - 1]` lookup
- **AND** coordinate underflow, missing grid rows, missing grid columns, and
  invalid grid block-size entries return crate-private typed errors instead of
  panicking
- **AND** the resulting context is checked against `TileDoSquareSplitCdf[0]`
  before row access

#### Scenario: Partition context derivation remains partial

- **WHEN** decoder support status is rendered
- **THEN** the tile CDF selection boundary still reports partial status
- **AND** partition decisions, `read_partition()`, and `decode_tile()` remain out
  of scope

#### Scenario: Runtime decode remains unsupported outside the boundary

- **WHEN** `splot decode` or the tile payload boundary reaches the CDF selection
  boundary after this change
- **THEN** it still reports structured `decode/unsupported-feature` metadata for
  the unimplemented `decode_tile()` / § 8.3 boundary
- **AND** it does not reconstruct pixels, compute hashes, write Y4M output,
  refresh references, locate or invoke external decoders, or bypass the
  `DecodeContext` worker-pool concurrency contract

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

### Requirement: DecodeContext tile-payload handoff

The decoder support model SHALL route the crate-private tile-payload boundary
through `DecodeContext` before any runtime tile syntax traversal or
reconstruction work is added. This handoff is tracked by Feature ID
`DECODE-CONTEXT-TILE-PAYLOAD-HANDOFF` and SHALL use the context-owned
`splot_parallel::WorkerPool` to execute the existing `tile-payload-decode`
boundary. The handoff SHALL NOT expose public tile-payload API support, bypass
`DecodeContext`, add a second worker pool, use direct Rayon/crossbeam/global
pool/thread primitives, add a `splot-decode -> splot-recon` dependency, or claim
runtime `splot decode` success.

#### Scenario: Tile boundary runs through DecodeContext

- **WHEN** crate-internal decoder code asks `DecodeContext` to plan an already
  framed minimal tile-payload boundary
- **THEN** the context executes that boundary inside its single owned
  `splot_parallel::WorkerPool`
- **AND** it returns the same deterministic tile work-unit metadata and
  structured unsupported `decode_tile()` stop as the direct crate-private
  boundary

#### Scenario: Thread policy does not change tile boundary output

- **WHEN** the same tile-payload boundary input is planned through
  `DecodeContext` configured with `auto`, `1`, and a fixed positive worker count
- **THEN** the returned plan metadata is identical across those thread policies
- **AND** no global pool, nested pool, direct Rayon/crossbeam API, ad-hoc
  thread, or queue is used outside `splot_parallel`

#### Scenario: Runtime decode remains unsupported

- **WHEN** `splot decode` is run after this handoff exists
- **THEN** it still follows the existing plan-only unsupported behavior until a
  later OpenSpec change derives tile-payload inputs from parsed frame state and
  implements tile syntax/reconstruction/output
- **AND** repo code, tests, `xtask`, and CI do not locate or invoke AVM, dav2d,
  ffmpeg, or any external decoder

### Requirement: Tile-payload input derivation

`splot-decode` SHALL provide a crate-private derivation bridge that builds
tile-payload boundary input for the minimal closed-loop-key tile tier from
source-backed parser output. The bridge SHALL validate that the selected
`DecodePlannedObu` matches the borrowed `splot-core` OBU envelope and that the
envelope payload is the exact slice of the original input bytes before using any
payload bytes. It SHALL derive the § 5.19 tile-group structure itself from that
same envelope payload, then derive the § 5.20 `tile_group_payload()` byte region
only after a complete structure parse, using checked arithmetic for
`headerBytes`, payload size, payload base, and per-tile byte spans. It SHALL
derive tile-grid, frame, quantizer, and CDF update facts from `FrameHeaderCore`,
`TileInfo`, the locally parsed `TileGroupStructure`, and `TileGroupFraming`
rather than invented values. The bridge SHALL run the resulting boundary through
the context-owned `DecodeContext` worker pool. It SHALL remain crate-private and
plan-only.

#### Scenario: Single tile candidate reaches unsupported tile syntax boundary

- **WHEN** a selected closed-loop-key OBU has matching source metadata, a
  complete intra first tile-group header, a complete § 5.19 structure, a
  one-tile § 5.20 payload region, and parser-derived tile and quantizer facts
- **THEN** the bridge derives a deterministic `DecodeTilePayloadPlan`
- **AND** the plan preserves source kind, IVF context when present, OBU index,
  OBU byte offset, selected layer, tile byte span, MI range, `CurrentQIndex`, and
  the existing unsupported `decode_tile()` boundary metadata
- **AND** no public tile-payload API, reconstruction, decoded-frame hash, Y4M
  output, reference refresh, or external decoder invocation occurs

#### Scenario: Forged parser metadata is rejected before slicing

- **WHEN** the planned OBU metadata does not match the borrowed OBU envelope, the
  envelope payload is not the exact slice from the original input bytes, or the
  borrowed payload bytes do not fit the declared OBU size and source container
  bounds
- **THEN** the bridge rejects the input with a local crate-private derivation
  error before slicing tile payload bytes
- **AND** no tile work unit is retained

#### Scenario: Tile group payload region is bounded

- **WHEN** § 5.19 parsing is truncated, `headerBytes` or `payload_size` is absent,
  or `headerBytes + payload_size` does not fit inside the OBU payload
- **THEN** the bridge rejects the input without using saturating or truncating
  slicing

#### Scenario: Unsupported paths do not guess facts

- **WHEN** the frame header is not complete intra, the selected candidate is not
  the first-and-only tile group, the frame is bridge/inter/TIP/BRU-dependent,
  required `tile_info`, quantizer, or `disable_cdf_update` facts are absent, or
  the tile range is outside the minimal tier
- **THEN** the bridge stops with a local derivation error or the existing
  structured tile-boundary unsupported metadata
- **AND** it does not infer continuation state from the most recent header or
  hardcode unexposed parser facts

#### Scenario: Thread policy does not change derived boundary output

- **WHEN** the same accepted source-backed tile input is derived through
  `DecodeContext` configured with `auto`, `1`, and a fixed positive worker count
- **THEN** the returned plan metadata or local error is identical across those
  thread policies
- **AND** no direct Rayon, crossbeam, global pool, nested pool, ad-hoc thread, or
  queue usage is introduced outside `splot_parallel`

#### Scenario: Local reference tools remain outside the repo

- **WHEN** this plan-only derivation bridge is implemented and tested
- **THEN** no AVM, dav2d, ffmpeg, or other external decoder is located, invoked,
  downloaded, built, wrapped, required by tests, or added to CI

### Requirement: Basic PAETH intra prediction primitive

The repository SHALL provide a scheduler-free `splot-recon` primitive for the
AV2 §7.13.2.2 basic intra prediction process, tracked by
`RECON-INTRA-BASIC-PAETH-PREDICTION`. The primitive SHALL predict rectangular
regions from caller-provided prepared `LeftCol[0..h)`, `AboveRow[0..w)`,
and `AboveRow[-1]` samples, validate left edge length against `h`, validate
above edge length against `w`, validate all edge samples against the active
decoded bit depth, and return typed `ReconError` values instead of panicking on
invalid inputs. The primitive SHALL implement the §7.13.2.2 candidate selection
using `base = AboveRow[j] + LeftCol[i] - AboveRow[-1]` and the three absolute
differences `pLeft`, `pTop`, and `pTopLeft`. The primitive SHALL NOT decide
AV2 edge availability, MRL, tile-boundary, superblock, CfL, directional,
smooth, DIP, transform, dequantization, residual, runtime decode, output, or
reference-refresh semantics.

#### Scenario: Basic PAETH prediction succeeds with self-contained tests

- **WHEN** `cargo test -p splot-recon --locked` runs
- **THEN** the test suite covers rectangular basic/PAETH prediction cases that
  select the left, above, and top-left candidates
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: Invalid basic PAETH prediction input is typed

- **WHEN** callers provide wrong-length edge samples, an edge sample outside the
  active bit-depth range, a sample type that cannot represent the active bit
  depth, a too-small output stride, or a too-small output buffer
- **THEN** `splot-recon` returns a structured `ReconError`
- **AND** library code does not panic, unwrap, silently clamp invalid input, or
  emit `decode/*` diagnostics

#### Scenario: Workspace supports in-storage basic PAETH writes

- **WHEN** callers request basic/PAETH intra prediction into a workspace plane
  whose top-left, left, and above neighbors are inside workspace storage
- **THEN** the workspace validates the target rectangle, uses the in-storage
  neighbors as prepared edge samples, and writes the predicted rectangle into
  workspace storage
- **AND** if the target touches the top or left storage boundary, the workspace
  returns a typed reconstruction error instead of inventing AV2 fallback
  availability samples
- **AND** the helper does not decide AV2 block availability, MRL, tile-boundary,
  superblock, CfL, directional, smooth, DIP, transform, dequantization,
  residual, runtime decode, output, or reference-refresh semantics

#### Scenario: Full intra reconstruction remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the matrix records basic/PAETH intra prediction as supported
- **AND** broad scalar intra reconstruction remains partial until full
  `predict_intra()` dispatch, directional prediction, smooth prediction, data
  driven prediction, subsampled DC, IBP, transform syntax, dequantization,
  inverse transforms, residual addition, runtime hash output, runtime Y4M
  output, and reference refresh are implemented and proven

### Requirement: Smooth intra prediction primitive

The repository SHALL provide a scheduler-free `splot-recon` primitive for the
AV2 §7.13.2.13 smooth intra prediction process, tracked by
`RECON-INTRA-SMOOTH-PREDICTION`. The primitive SHALL predict rectangular
regions for `SMOOTH_PRED`, `SMOOTH_V_PRED`, and `SMOOTH_H_PRED` from
caller-provided prepared `LeftCol[0..h]` and `AboveRow[0..w]` samples,
including the `LeftCol[h]` bottom-left and `AboveRow[w]` top-right sentinel
samples. The primitive SHALL validate left edge length against `h + 1`,
validate above edge length against `w + 1`, validate all edge samples and
computed output samples against the active decoded bit depth, validate output
stride and length, and return typed `ReconError` values instead of panicking on
invalid inputs. The primitive SHALL implement the §7.13.2.13 formulas using
AV2 §3 `BLEND_WEIGHT_MAX = 32` and AV2 §4.8 `Round2`. The primitive SHALL NOT
decide AV2 edge availability, MRL, tile-boundary, superblock, CfL,
directional, PAETH, DIP, transform, dequantization, residual, runtime decode,
output, or reference-refresh semantics.

#### Scenario: Smooth prediction succeeds with self-contained tests

- **WHEN** `cargo test -p splot-recon --locked` runs
- **THEN** the test suite covers rectangular smooth prediction for
  `SMOOTH_PRED`, `SMOOTH_V_PRED`, and `SMOOTH_H_PRED`
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: Invalid smooth prediction input is typed

- **WHEN** callers provide wrong-length edge samples, an edge sample outside the
  active bit-depth range, a sample type that cannot represent the active bit
  depth, a too-small output stride, a too-small output buffer, or a computed
  prediction outside the active bit depth
- **THEN** `splot-recon` returns a structured `ReconError`
- **AND** library code does not panic, unwrap, silently clamp invalid input, or
  emit `decode/*` diagnostics

#### Scenario: Workspace supports in-storage smooth writes

- **WHEN** callers request smooth intra prediction into a workspace plane whose
  left, above, bottom-left, and top-right prepared samples are inside workspace
  storage
- **THEN** the workspace validates the target rectangle, uses those in-storage
  samples as prepared edge inputs, and writes the predicted rectangle into
  workspace storage
- **AND** if any required prepared edge or sentinel sample is outside workspace
  storage, the workspace returns a typed reconstruction error instead of
  inventing AV2 fallback availability samples
- **AND** the helper does not decide AV2 block availability, MRL, tile-boundary,
  superblock, CfL, directional, PAETH, DIP, transform, dequantization,
  residual, runtime decode, output, or reference-refresh semantics

#### Scenario: Full intra reconstruction remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the matrix records smooth intra prediction as supported
- **AND** broad scalar intra reconstruction remains partial until full
  `predict_intra()` dispatch, directional prediction, data driven prediction,
  subsampled DC, IBP, transform syntax, dequantization, inverse transforms,
  residual addition, runtime hash output, runtime Y4M output, and reference
  refresh are implemented and proven

### Requirement: Full decoder conformance contract

The repository SHALL provide `docs/DECODER-FULL-CONFORMANCE.md` as the public
contract for the future AV2 v1.0.0 full decoder conformance claim. The document
SHALL state the current decoder status without overclaiming, define the final
conditions for claiming full conformance, distinguish raw intermediate output
from post-film-grain output, describe deterministic diagnostics and output-file
safety requirements, and preserve the local-only AVM/dav2d evidence boundary.
Tracked by `DOC-DECODER-FULL-CONFORMANCE-CONTRACT`.

#### Scenario: Reader checks current decoder status

- **WHEN** a reader opens `docs/DECODER-FULL-CONFORMANCE.md`
- **THEN** the document says that `splot decode` is not yet a full AV2 decoder
- **AND** it points readers to the generated decoder support and decoder spec
  coverage documents for current status

#### Scenario: Reader checks the future conformance claim

- **WHEN** a reader checks the future definition of full decoder conformance
- **THEN** the document requires support for every normative AV2 v1.0.0
  decode-relevant section within configured resource limits
- **AND** it requires zero temporary `decode/unsupported-feature` diagnostics for
  conforming streams before any full-conformance claim is allowed

#### Scenario: Reader checks reference evidence boundaries

- **WHEN** a reader checks how AVM or dav2d may be used for decoder evidence
- **THEN** the document states that reference evidence is committed as
  non-executable metadata only
- **AND** repository code, tests, `xtask`, CI, setup scripts, wrappers, and
  dependencies SHALL NOT locate, build, invoke, cache, or require AVM or dav2d

### Requirement: Generated decoder spec coverage document

The repository SHALL provide a generated `docs/DECODER-SPEC-COVERAGE.md`
document that maps AV2 v1.0.0 decoder-relevant section families to current
implementation ownership and evidence. Each row SHALL include `spec_sections`,
`spec_title`, `normative_status`, `implementation_owner`,
`decoder_support_rows`, `feature_ids`, `status`, `tests`, `fuzz_targets`,
`local_reference_evidence`, `diagnostics`, and `notes`. The allowed
`normative_status` values SHALL be `normative`, `informative`, and `mixed`; rows
with `normative_status = "mixed"` SHALL include notes explaining which portion is
normative for decoder conformance. The allowed row statuses SHALL be
`unsupported`, `partial`, `supported`, `blocked`, and
`out-of-scope-nonnormative`. Tracked by
`XTASK-DECODER-CONFORMANCE-COVERAGE`.

#### Scenario: Coverage document is generated

- **WHEN** `cargo xtask decoder-conformance-coverage --format markdown --output docs/DECODER-SPEC-COVERAGE.md` runs
- **THEN** the command writes a deterministic Markdown render of the decoder
  conformance coverage rows
- **AND** the output includes every section family needed by the full decoder
  conformance contract

#### Scenario: Normative status is explicit

- **WHEN** a generated row has `normative_status = "mixed"`
- **THEN** the row notes which cited sections are normative for decoder
  conformance and which cited sections are informative context

#### Scenario: Unsupported runtime sections remain visible

- **WHEN** a decode-relevant AV2 section family has no runtime decoder owner
- **THEN** the generated coverage document records `status = "unsupported"` or
  `status = "partial"` rather than omitting the section family
- **AND** the notes explain the missing runtime owner or remaining evidence gap

#### Scenario: Supported coverage row requires proof

- **WHEN** a decoder conformance coverage row has `status = "supported"`
- **THEN** the row records at least one self-contained test or proof reference
- **AND** runtime decode support SHALL NOT be marked supported from parser-only,
  docs-only, or raw reference-output evidence alone

#### Scenario: Non-normative exclusions are explicit

- **WHEN** a row has `status = "out-of-scope-nonnormative"`
- **THEN** the row includes a note explaining why the section family is not
  required for AV2 decoder conformance

### Requirement: Decoder conformance coverage drift gate

The repository SHALL provide `cargo xtask check-decoder-conformance-coverage` as
a self-contained drift and honesty gate for `docs/DECODER-SPEC-COVERAGE.md`. The
gate SHALL be part of `cargo xtask ci`, SHALL run without AVM or dav2d, and SHALL
fail when generated coverage output or cross-links are inconsistent with
committed repository files. Tracked by `XTASK-DECODER-CONFORMANCE-COVERAGE`.

#### Scenario: Coverage document drifts

- **WHEN** the coverage rows change without regenerating
  `docs/DECODER-SPEC-COVERAGE.md`
- **THEN** `cargo xtask check-decoder-conformance-coverage` fails
- **AND** the failure names the regeneration command

#### Scenario: Coverage row has invalid status

- **WHEN** a decoder conformance coverage row uses a status outside the allowed
  status set
- **THEN** `cargo xtask check-decoder-conformance-coverage` fails and names the
  offending row

#### Scenario: Coverage row references missing evidence

- **WHEN** a decoder conformance coverage row names a decoder support row,
  Feature ID, diagnostic, or local reference evidence id that is absent from the
  committed support, implementation matrix, diagnostics, or evidence files
- **THEN** `cargo xtask check-decoder-conformance-coverage` fails and names the
  missing reference

#### Scenario: Normative owner lacks a Feature ID

- **WHEN** a normative or mixed decoder conformance coverage row names a decoder
  support row as an implementation owner
- **THEN** that support row SHALL have a non-empty Feature ID

#### Scenario: CI remains self-contained

- **WHEN** `cargo xtask ci` runs on a machine without AVM or dav2d installed
- **THEN** decoder conformance coverage checks pass or fail solely from committed
  repository files
- **AND** no external reference decoder is located or invoked

### Requirement: Decoder output equivalence contract

The decoder support model SHALL document a decoder output equivalence contract
tracked by Feature ID `DOC-DECODER-OUTPUT-EQUIVALENCE-CONTRACT`. The contract
SHALL define future runtime output identity for `splot decode` without claiming
runtime decode support. It SHALL cite AV2 v1.0.0 § 5.17.12 and § 6.16.13 for
decoded-frame-hash metadata, § 6.4.1, § 6.17.4.1, and § 6.17.4.4 for output
format and crop-derived geometry, § 7.21.1 through § 7.21.7 for output events,
intermediate output, implicit output, flush output, output frame buffers, and
film grain, and § 7.22-§ 7.23 for the distinction between output events and
reference-frame state. The contract SHALL keep runtime decode, runtime hash
output, raw output, Y4M output, film-grain synthesis, metadata-hash
verification, and external reference-tool integration unsupported until later
source-backed changes provide implementation and tests.

#### Scenario: Output variants are named

- **WHEN** a reader checks the decoder output equivalence contract
- **THEN** it defines `raw_intermediate_output` as the § 7.21.2 intermediate
  output sample set before film grain synthesis
- **AND** it defines `post_film_grain_output` as the sample set after the
  § 7.21.7 film-grain synthesis process when that process applies
- **AND** it states that no-grain streams may produce identical sample bytes for
  both variants while the variant identifier remains part of artifact identity

#### Scenario: Raw intermediate hash contract remains stable

- **WHEN** a reader checks the hash identity for `raw_intermediate_output`
- **THEN** the contract keeps `splot-dfh-sha256-v1` over the
  `av2-output-samples-v1` byte stream as the stable raw intermediate hash
- **AND** it requires hash results to name the output variant, algorithm
  identifier, and byte-stream identifier
- **AND** it states that any future post-film-grain hash result MUST carry the
  `post_film_grain_output` variant identifier and cannot be supported before
  film-grain synthesis is implemented and tested

#### Scenario: Output event order is pinned

- **WHEN** future runtime decode emits output frames
- **THEN** output indices are assigned in AV2 output-process order after the
  selected operating point or layer is applied
- **AND** show-existing output events receive distinct output indices even when
  they reuse stored frame samples
- **AND** a show-existing output reached through `output_process(-1)` with
  `ShowExistingFrame == 1` does not mark the referenced frame as already output
  for later implicit-output or flush eligibility
- **AND** implicit output and flush events are appended according to § 7.21.4
  and § 7.21.5
- **AND** output ordering MUST NOT depend on decode order, OBU order, reference
  slot index, hash completion order, file-write completion order, or worker
  completion order

#### Scenario: Visible sample bytes are canonical

- **WHEN** future runtime output serializes hash, raw, or Y4M sample payloads
- **THEN** luma output uses the visible `w` by `h` sample rectangle produced by
  the AV2 output process
- **AND** non-monochrome chroma output uses
  `((w + subX) >> subX)` by `((h + subY) >> subY)` samples for U and V
- **AND** monochrome output omits U and V planes
- **AND** sample traversal is Y, then U, then V, in raster order within each
  present plane
- **AND** 8-bit output samples serialize as one byte and greater-than-8-bit
  output samples serialize as two little-endian bytes
- **AND** stride padding, backing allocation padding, reference-store metadata,
  OBU bytes, container bytes, and decoded-frame-hash metadata are excluded from
  the sample byte stream

#### Scenario: Hash JSON success schema is separate from diagnostics

- **WHEN** future `splot decode --output-format hash --json` succeeds
- **THEN** stdout is a success artifact with
  `contract_id = "splot.decode.hash_report"` and `contract_version = 1`
- **AND** it contains the selected output variant or variants, the selected
  thread policy, and an array of frames sorted by output index
- **AND** each frame entry records output index, visible luma crop origin,
  visible luma dimensions, chroma crop origin and dimensions when present, bit
  depth, pixel format, and one or more hash entries with `variant`,
  `algorithm_id`, `byte_stream_id`, and 64-character lowercase hexadecimal
  `digest_hex`
- **AND** monochrome frame entries omit chroma origin and dimension fields
- **AND** failure paths continue to emit decoder diagnostic JSON instead of a
  partial hash report

#### Scenario: Raw and Y4M output contracts are distinct

- **WHEN** runtime raw output is implemented for any tier
- **THEN** raw output is defined as concatenated canonical sample bytes for each
  output event in output-index order for the selected variant, with no header or
  metadata bytes
- **AND** the current `--output-format raw` CLI mode is limited to the
  `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT` minimal tier until broader runtime raw
  output is separately implemented and evidenced
- **WHEN** runtime Y4M output is implemented for any tier
- **THEN** Y4M output represents the AV2 output-frame sample set for the chosen
  variant, using repository-owned Y4M container policy
- **AND** Y4M container bytes remain repository output policy rather than AV2
  syntax
- **AND** the current Y4M CLI output path is limited to the
  `DECODE-Y4M-RUNTIME-OUTPUT` minimal tier until broader runtime Y4M output is
  separately implemented and evidenced

#### Scenario: Output-file publication is atomic

- **WHEN** a current or future successful `splot decode -o <path>` mode writes
  raw, Y4M, or another output artifact
- **THEN** it writes to a temporary file in the final path's directory,
  completes serialization, flushes user-space buffers, syncs the temporary
  file's contents and metadata, renames the temporary file as the final publish
  step, and attempts best-effort parent-directory sync after rename where the
  platform supports it
- **AND** if decode, reconstruction, hash serialization, raw/Y4M serialization,
  validation, temporary-file write, flush, temporary-file sync, rename, or any
  other pre-rename publication step fails, an absent final path remains absent
  and an existing final path remains byte-for-byte unchanged
- **AND** if rename succeeds, unsupported or failed parent-directory sync does
  not convert the completed publication into a failed decode, and the final path
  MUST NOT contain a partially serialized payload
- **AND** output path creation, temporary-file write, flush, sync, rename,
  cleanup, or serialization failures before the completed rename are emitted as
  a registered `decode/output-error` diagnostic rather than as partial success
  artifacts
- **AND** output-derived counts and byte sizes are computed with checked
  arithmetic and checked against `DecodeLimits` before allocation, indexing, or
  output publication

#### Scenario: Metadata hashes remain separate

- **WHEN** decoded-frame-hash metadata is present in a future supported stream
- **THEN** metadata verification uses the AV2 § 5.17.12 and § 6.16.13 metadata
  contract for conformance checking
- **AND** metadata verification is reported separately from repository
  `splot-dfh-sha256-v1` success artifacts
- **AND** the decoder support matrix does not treat AVM/dav2d raw MD5 metadata
  or decoded-frame-hash metadata verification as proof of repository SHA-256
  runtime output support

#### Scenario: Reference tools remain metadata only

- **WHEN** local AVM or dav2d evidence is recorded for output equivalence
- **THEN** committed evidence is portable metadata such as tool name, revision,
  sanitized command summary, input hash, output hash, date, and agreement notes
- **AND** the repository does not add AVM/dav2d source, binaries, submodules,
  dependencies, wrappers, setup scripts, Docker images, caches, CI jobs, runtime
  process execution, or `xtask` commands that locate, build, invoke, or require
  AVM or dav2d

### Requirement: Minimal-tier runtime hash success

The decoder support model SHALL define a supported
`decode-minimal-tier-runtime-success` row tracked by Feature ID
`DECODE-MINIMAL-TIER-RUNTIME-SUCCESS` only when `splot decode` can verify the
documented minimal intra fixture trace and emit a hash success artifact. The
supported scope SHALL be limited to the `minimal-intra-8bit420-hash-v1`
fixture-trace tier and SHALL cite
the relevant AV2 v1.0.0 Annex B length-delimited input rules, § 5.2/§ 6.2 OBU
front-door rules, minimal-tier frame/tile syntax sections, § 8.2 symbol parsing,
§ 7.1 decode process, § 7.21.1-§ 7.21.2 output sample preparation, and
§ 5.17.12/§ 6.16.13 only for metadata-hash separation and sample-byte-order
context. The row SHALL NOT claim
full AV2 decoder conformance, Annex A level/tier conformance, Y4M/raw output,
film-grain output, metadata hash verification, full tile/CDF/intra support,
reference-refresh completeness, or AVM/dav2d integration.

#### Scenario: Minimal-tier hash JSON succeeds

- **WHEN** `splot decode --output-format hash --json` runs on the committed
  minimal-tier intra IVF fixture
- **THEN** the command exits with code 0
- **AND** stdout is a `splot.decode.hash_report` success artifact with
  `contract_version = 1`
- **AND** the report contains one or more frames sorted by `output_index`
- **AND** each hash entry names `raw_intermediate_output`,
  `splot-dfh-sha256-v1`, `av2-output-samples-v1`, and a 64-character lowercase
  hexadecimal digest
- **AND** hash output does not require nonzero IVF timebase fields because it
  does not serialize frame-rate metadata
- **AND** stderr is empty

#### Scenario: Hash mode does not touch output paths

- **WHEN** hash output succeeds with no `-o` path
- **THEN** the command creates no implicit output file in the working directory
- **WHEN** hash output succeeds with `-o <path>` pointing to an existing file
- **THEN** the command leaves that file byte-for-byte unchanged
- **AND** no temporary or recovery file is left in the output directory

#### Scenario: Thread policies produce identical decoded frame hashes

- **WHEN** the same minimal-tier fixture is decoded with `--threads 1`,
  `--threads auto`, and a fixed positive `--threads N`
- **THEN** every run emits the same ordered `output_index` sequence
- **AND** every run emits the same visible dimensions, pixel format, bit depth,
  output variant, byte-stream identifier, and digest value for each frame
- **AND** any selected-thread-policy metadata difference does not change the
  decoded frame hash identity

#### Scenario: Malformed input remains diagnostic JSON

- **WHEN** malformed Annex B or IVF input is decoded with
  `--output-format hash --json`
- **THEN** the command exits nonzero
- **AND** stdout is a decoder diagnostic JSON object with
  `rule_id = "decode/malformed-source"`
- **AND** stdout is not a partial `splot.decode.hash_report`
- **AND** no output path is created or modified

#### Scenario: Outside-tier valid streams remain unsupported

- **WHEN** a valid AV2 stream is outside `minimal-intra-8bit420-hash-v1`
- **THEN** the command exits nonzero
- **AND** stdout or stderr reports `decode/unsupported-feature`
- **AND** the diagnostic names the blocking matrix row or feature metadata
- **AND** no hash success artifact is emitted

#### Scenario: Runtime resource limits fail before allocation or output

- **WHEN** bitstream-derived dimensions, tile payloads, decoded-frame bytes,
  output frame counts, output byte counts, or hash report sizes exceed
  `DecodeLimits` or checked arithmetic overflows
- **THEN** the command exits nonzero with `decode/resource-limit`
- **AND** the diagnostic includes the limit name, unit, and measured value when
  available
- **AND** no decoded-frame allocation, hash report construction, or output path
  publication occurs before the limit check

#### Scenario: Reference evidence remains portable metadata only

- **WHEN** local AVM or dav2d evidence is recorded for the minimal-tier fixture
- **THEN** it is committed only as portable metadata such as tool name,
  revision, sanitized command summary, fixture hash, output digest, date, and
  agreement notes
- **AND** repository code, tests, CI, xtask commands, scripts, wrappers,
  submodules, binaries, caches, and runtime execution do not locate, build,
  invoke, or require AVM or dav2d

#### Scenario: Status updates remain narrow

- **WHEN** the runtime hash path is implemented
- **THEN** `docs/DECODER-SUPPORT-MATRIX.toml`,
  `docs/DECODER-SUPPORT-STATUS.md`, `docs/IMPLEMENTATION-MATRIX.toml`, and
  generated feature/spec coverage docs mark only the proven minimal hash runtime
  scope as supported
- **AND** broad rows for full decode, tile payload decode, CDF lifecycle,
  intra/inter reconstruction, Y4M/raw output, film grain, reference update,
  layers, and decoder-model constraints remain partial or unsupported until
  their own source-backed implementation and tests land

### Requirement: Minimal-tier runtime Y4M output
For Feature ID `DECODE-Y4M-RUNTIME-OUTPUT`, the decoder support model SHALL provide a narrow `splot decode` Y4M success path for the existing `minimal-intra-8bit420-hash-v1` tier, using the same byte-consuming validation, tile trace, output sample values, visible geometry, bit depth, and pixel format already required for `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS`.

#### Scenario: Explicit Y4M output succeeds for the minimal fixture
- **WHEN** `splot decode --output-format y4m <minimal-ivf-fixture> -o <output.y4m>` is run for the committed minimal 64x64 IVF fixture
- **THEN** the command exits successfully
- **AND** stdout and stderr are empty
- **AND** `<output.y4m>` contains a complete Y4M stream for one 64x64 8-bit 4:2:0 raw-intermediate-output frame
- **AND** the frame payload contains the same flat sample values used by the minimal hash runtime

#### Scenario: Zero IVF timebase fails before Y4M serialization
- **WHEN** `splot decode --output-format y4m <minimal-ivf-fixture> -o <output.y4m>` is run for the committed minimal fixture shape with a zero IVF timebase numerator or denominator
- **THEN** the command emits a structured `decode/unsupported-feature` diagnostic for `invalid_ivf_timebase`
- **AND** it does not create, truncate, or replace `<output.y4m>`

#### Scenario: Implicit Y4M output remains the compatibility form
- **WHEN** `splot decode <minimal-ivf-fixture> -o <output.y4m>` is run without `--output-format`
- **THEN** the command selects Y4M output
- **AND** it writes the same bytes as explicit `--output-format y4m`

#### Scenario: Out-of-tier Y4M inputs fail closed
- **WHEN** `splot decode --output-format y4m <input> -o <output.y4m>` is run for a malformed, resource-limited, or out-of-tier source
- **THEN** the command emits the existing structured `decode/malformed-source`, `decode/resource-limit`, or `decode/unsupported-feature` diagnostic as appropriate
- **AND** it does not create, truncate, or replace `<output.y4m>`

### Requirement: Atomic runtime Y4M publication
The CLI SHALL publish runtime Y4M output atomically: all Y4M bytes MUST be written to a same-directory temporary file first, and the requested output path MUST be replaced only after successful decode, serialization, flush, and file sync.

#### Scenario: Existing output is replaced only after success
- **WHEN** the requested Y4M output path already contains bytes
- **AND** the minimal runtime Y4M decode succeeds
- **THEN** the requested path is replaced by the complete Y4M stream
- **AND** no temporary output file remains in the output directory

#### Scenario: Failure preserves existing output
- **WHEN** the requested Y4M output path already contains bytes
- **AND** decode, serialization, flush, sync, rename, or cleanup fails before publication
- **THEN** the requested path remains byte-for-byte unchanged
- **AND** no partial Y4M stream is visible at the requested path

#### Scenario: Hash output remains no-touch
- **WHEN** `splot decode --output-format hash <input> -o <path>` is run
- **THEN** hash mode does not create, truncate, or replace `<path>`
- **AND** this remains true for both hash success and hash diagnostic paths

### Requirement: Decode output error diagnostics
The decoder support model SHALL expose `decode/output-error` for raw or Y4M
serialization and CLI publication failures that are not malformed-source,
resource-limit, or unsupported-feature conditions.

#### Scenario: Output path cannot be published
- **WHEN** runtime raw or Y4M decode reaches output publication but the output
  path cannot be created, written, flushed, synced, renamed, or cleaned up
- **THEN** `splot decode` emits a structured `decode/output-error` diagnostic
- **AND** the diagnostic includes a stable operation identifier
- **AND** it does not include nondeterministic temporary filename suffixes

#### Scenario: Output error is separate from AV2 conformance
- **WHEN** the failure is a filesystem or writer publication failure
- **THEN** the diagnostic is not reported as AV2 malformed source or unsupported feature
- **AND** any spec section field is omitted unless the failure is tied to AV2 output-sample semantics rather than filesystem publication

### Requirement: Runtime Y4M byte accounting
The runtime Y4M output path SHALL check `DecodeLimitName::MaxOutputBytes` against the complete Y4M stream length, including the Y4M stream header, per-frame header, and visible sample payload bytes, before publishing the file.

#### Scenario: Output byte limit rejects before publication
- **WHEN** the configured `max_output_bytes` is smaller than the complete minimal Y4M stream length
- **THEN** runtime Y4M output fails with `decode/resource-limit`
- **AND** the requested output path is not created, truncated, or replaced

#### Scenario: Output is deterministic across thread policies
- **WHEN** the same minimal fixture is decoded to Y4M with `--threads 1`, `--threads auto`, and a fixed positive thread count
- **THEN** each successful command writes byte-identical Y4M output

### Requirement: Decoder support tracks tile partition decision boundary

The decoder support model SHALL track `DECODE-TILE-PARTITION-DECISION-BOUNDARY` as a distinct crate-private row named `tile-partition-decision-boundary`. The row SHALL mark only the AV2 §5.20.3.2 partition decision boundary over caller-provided facts as supported, SHALL link it to the existing `tile-partition-symbol-read-boundary` and `tile-cdf-selection-boundary` rows, and SHALL keep broader `tile-payload-decode`, `tile-cdf-selection-boundary`, and traversal/output rows honest when they remain partial.

#### Scenario: Support matrix records narrow partition decision support
- **WHEN** `cargo xtask check-decoder-support` renders decoder support status
- **THEN** `tile-partition-decision-boundary` appears as its own row with Feature ID `DECODE-TILE-PARTITION-DECISION-BOUNDARY`
- **AND** its notes state that support is limited to one partition decision from caller-provided allowed/implied facts
- **AND** it does not claim allowed-partition derivation, recursive `read_partition()`, `decode_tile()`, reconstruction, output, reference refresh, or external decoder use

#### Scenario: Existing broader rows remain honest
- **WHEN** decoder support status is rendered after the decision boundary lands
- **THEN** `tile-payload-decode` and `tile-cdf-selection-boundary` remain partial until their broader residual work is implemented
- **AND** `tile-partition-symbol-read-boundary` remains limited to individual `S()` reads
- **AND** the new row is cited from those notes as the separate decision consumer rather than broadening their claims

### Requirement: Minimal Runtime Partition Frontier Integration
The decoder support model SHALL record that the
`DECODE-MINIMAL-TIER-RUNTIME-SUCCESS` minimal hash/Y4M runtime consumes the
`DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY` root partition frontier before the
remaining traced flat-tile symbols. The support evidence SHALL keep
`tile-payload-decode`, broad `symbol-decoder`, CDF lifecycle, `decode_block()`,
`MiSizes` mutation, reconstruction, and reference-refresh work partial while
they remain outside the supported tier.

#### Scenario: Runtime bridge is recorded without broad decode overclaim
- **WHEN** decoder support/status checks run after this change
- **THEN** the minimal runtime support row names the partition-frontier bridge
  as evidence
- **AND** `tile-partition-traversal-boundary` remains the supported row for the
  first `decode_block()` frontier
- **AND** `tile-payload-decode` remains partial for full `decode_tile()`,
  `decode_block()` syntax, `MiSizes` mutation, reconstruction, output expansion,
  CDF lifecycle, and reference refresh work

#### Scenario: Public outputs stay in the same minimal tier
- **WHEN** the committed minimal fixture is decoded through hash and Y4M runtime
  entry points
- **THEN** output bytes remain unchanged
- **AND** the only public success tier remains
  `minimal-intra-8bit420-hash-v1`

### Requirement: Decoder support matrix tracks tile CDF lifecycle boundaries

The decoder support matrix SHALL include a row for
`tile-cdf-save-lifecycle-boundary`, tracked by Feature ID
`DECODE-TILE-CDF-SAVE-LIFECYCLE-BOUNDARY`, covering the crate-private
Tile-to-Saved and Saved-to-Frame CDF lifecycle behavior for the currently
supported subset only.

#### Scenario: lifecycle row is scoped and test-backed

- **GIVEN** the generated decoder support status
- **WHEN** the `tile-cdf-save-lifecycle-boundary` row is rendered
- **THEN** it lists AV2 § 5.20.1, § 6.19.1, § 7.5, § 8.2.2, § 8.2.4,
  § 8.2.6, § 8.3.1, and § 8.3.2 as scoped references
- **AND** it records tests for copy, average, frame-end count scaling,
  transaction rollback, and minimal runtime hash/Y4M identity
- **AND** it does not mark broad § 8.3 CDF selection, full § 9.3 CDF banks,
  multi-tile scheduling, or full `decode_tile()` traversal supported

### Requirement: Decoder support matrix tracks runtime hash fuzz coverage

The decoder support matrix SHALL include a row named
`decode-runtime-hash-fuzz`, tracked by Feature ID
`CONF-DECODE-RUNTIME-HASH-FUZZ`, covering no-panic fuzz coverage for the current
minimal `DecodeContext::decode_hash_report_bytes` byte-consuming API.

#### Scenario: runtime hash fuzz row is scoped and test-backed

- **GIVEN** the generated decoder support status
- **WHEN** the `decode-runtime-hash-fuzz` row is rendered
- **THEN** it records `fuzz/fuzz_targets/decode_runtime_hash_bytes.rs` as
  evidence
- **AND** it records finite-limit behavior and the commands that compile or
  enumerate the fuzz target
- **AND** it does not mark broad runtime decode, full tile syntax, full
  reconstruction, AVM/dav2d differential testing, or Y4M/raw output fuzzing as
  supported

### Requirement: Decoder support matrix tracks Y4M serialization fuzz coverage

The decoder support matrix SHALL include a row named
`recon-y4m-output-fuzz`, tracked by Feature ID
`CONF-RECON-Y4M-OUTPUT-FUZZ`, covering no-panic fuzz coverage for
source-backed `splot-recon` Y4M serialization over bounded caller-supplied
decoded frames.

#### Scenario: Y4M fuzz row is scoped and test-backed

- **GIVEN** the generated decoder support status
- **WHEN** the `recon-y4m-output-fuzz` row is rendered
- **THEN** it records `fuzz/fuzz_targets/recon_y4m_output_bytes.rs` as evidence
- **AND** it records bounded structured frame generation and fuzz target
  enumeration commands
- **AND** it does not mark broad runtime decode, byte-consuming runtime Y4M
  decode, raw output, AVM/dav2d differential testing, or filesystem
  publication as supported

### Requirement: Decoder support matrix tracks intra prediction fuzz coverage

The decoder support matrix SHALL include a row named
`recon-intra-prediction-fuzz`, tracked by Feature ID
`CONF-RECON-INTRA-PREDICTION-FUZZ`, covering no-panic fuzz coverage for
source-backed `splot-recon` intra prediction and current-frame workspace
primitives over bounded structured inputs.

#### Scenario: intra prediction fuzz row is scoped and test-backed

- **GIVEN** the generated decoder support status
- **WHEN** the `recon-intra-prediction-fuzz` row is rendered
- **THEN** it records `fuzz/fuzz_targets/recon_intra_prediction_bytes.rs` as
  evidence
- **AND** it records bounded structured prediction/workspace generation and
  fuzz target enumeration commands
- **AND** it does not mark broad runtime decode, full §7.13 intra
  reconstruction, directional prediction, data driven intra prediction, IBP,
  filter intra, CfL/CCTX, palette, residual, transform, quantization,
  loop-filter, AVM/dav2d differential testing, filesystem publication, or
  output scheduling as supported

### Requirement: Decoder support matrix tracks runtime Y4M fuzz coverage
The decoder support matrix SHALL include a row named
`decode-runtime-y4m-fuzz`, tracked by Feature ID
`CONF-DECODE-RUNTIME-Y4M-FUZZ`, covering no-panic fuzz coverage for the current
minimal-tier `DecodeContext::decode_y4m_bytes` byte-consuming API over bounded
raw input and minimal-fixture mutation inputs.

#### Scenario: runtime Y4M fuzz row is scoped and test-backed
- **GIVEN** the generated decoder support status
- **WHEN** the `decode-runtime-y4m-fuzz` row is rendered
- **THEN** it records `fuzz/fuzz_targets/decode_runtime_y4m_bytes.rs` as
  evidence
- **AND** it records fuzz target enumeration, focused runtime Y4M tests, and a
  local nightly fuzz smoke command
- **AND** it does not mark broad AV2 runtime decode, full Y4M output
  conformance, CLI filesystem publication, raw output, hash report output,
  post-film-grain output, show-existing/flush scheduling, reference refresh,
  metadata MD5 verification, AVM/dav2d differential testing, or support beyond
  the committed minimal IVF tier as supported

### Requirement: Decoder support matrix tracks symbol decoder fuzz coverage
The decoder support matrix SHALL include a supported row named
`symbol-decoder-fuzz`, tracked by Feature ID `CONF-SYMBOL-DECODER-FUZZ`, for
scoped no-panic fuzz coverage of the existing public
`splot_core::symbol::SymbolDecoder` API. This robustness row is independent of
the `symbol-decoder` row's own support status; it does not by itself promote or
demote §8.3 syntax-element CDF selection or runtime tile-decode behavior.

#### Scenario: symbol decoder fuzz evidence is scoped and test-backed
- **GIVEN** the generated decoder support status
- **WHEN** the `symbol-decoder-fuzz` row is rendered
- **THEN** it records `fuzz/fuzz_targets/symbol_decoder_bytes.rs` as evidence
- **AND** it records fuzz target enumeration, fuzz crate compilation, focused
  symbol decoder tests, and a local nightly fuzz smoke command
- **AND** it keeps broad §8.3 CDF selection, default Tile or Saved CDF banks,
  runtime tile payload traversal, reconstruction, output, reference refresh,
  AVM differential testing, dav2d differential testing, and support beyond the
  public §8.2 symbol decoder primitive out of scope

### Requirement: Tile payload fuzz support evidence

The decoder support matrix SHALL record `CONF-TILE-PAYLOAD-DECODE-FUZZ` as
self-contained fuzz evidence for the current minimal tile-payload runtime byte
frontier. The evidence SHALL reference the cargo-fuzz target, the
`splot-decode` `fuzzing` harness used by the target, focused
tile-payload/runtime tests, and the required
fuzz/check commands.

#### Scenario: Decoder support records the fuzz target

- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** a support row for `tile-payload-decode-fuzz` links Feature ID
  `CONF-TILE-PAYLOAD-DECODE-FUZZ` to `fuzz/fuzz_targets/tile_payload_decode_bytes.rs`
  and the feature-gated fuzzing harness

#### Scenario: Broad tile decode remains partial

- **WHEN** decoder support status is regenerated after adding the fuzz target
- **THEN** `tile-payload-decode` remains `partial` and its notes continue to
  exclude full `decode_tile()`, broad §8.3 CDF selection, recursive
  partition/block syntax, reconstruction expansion, reference refresh, and
  external decoder integration

### Requirement: Decoder support matrix tracks frame hash fuzz coverage

The decoder support matrix SHALL include a row named
`recon-frame-hash-fuzz`, tracked by Feature ID
`CONF-RECON-FRAME-HASH-FUZZ`, covering no-panic fuzz coverage for source-backed
`splot-recon` decoded-frame hash input serialization and digest computation over
bounded caller-supplied decoded frames.

#### Scenario: frame hash fuzz row is scoped and test-backed

- **GIVEN** the generated decoder support status
- **WHEN** the `recon-frame-hash-fuzz` row is rendered
- **THEN** it records `fuzz/fuzz_targets/recon_frame_hash_bytes.rs` as evidence
- **AND** it records bounded structured frame generation and fuzz target
  enumeration commands
- **AND** it does not mark broad runtime decode, AV2 decoded-frame-hash metadata
  verification, output ordering, film grain, reference refresh, AVM/dav2d
  differential testing, or filesystem publication as supported

### Requirement: Decoder support matrix tracks reference-frame store fuzz coverage

The decoder support matrix SHALL include a row named
`recon-reference-frame-store-fuzz`, tracked by Feature ID
`CONF-RECON-REFERENCE-FRAME-STORE-FUZZ`, covering no-panic fuzz coverage for the
source-backed `splot-recon` reference-frame store storage API over bounded
operation sequences.

#### Scenario: reference-frame store fuzz row is scoped and test-backed

- **GIVEN** the generated decoder support status
- **WHEN** the `recon-reference-frame-store-fuzz` row is rendered
- **THEN** it records
  `fuzz/fuzz_targets/recon_reference_frame_store_bytes.rs` as evidence
- **AND** it records bounded operation-sequence generation and fuzz target
  enumeration commands
- **AND** it does not mark byte-consuming decode, AV2 reference refresh
  semantics, `RefValid`, `refresh_frame_flags`, output scheduling,
  motion-field storage, resource diagnostics, AVM/dav2d differential testing,
  or filesystem publication as supported

### Requirement: Decoder support matrix tracks frame and plane model fuzz coverage

The decoder support matrix SHALL include a row named
`recon-frame-plane-types-fuzz`, tracked by Feature ID `CONF-RECON-FRAME-PLANE-TYPES-FUZZ`,
covering no-panic fuzz coverage for the source-backed `splot-recon`
decoded-frame and plane runtime type validators and accessors.

#### Scenario: frame and plane fuzz row is scoped and test-backed

- **GIVEN** the generated decoder support status
- **WHEN** the `recon-frame-plane-types-fuzz` row is rendered
- **THEN** it records `fuzz/fuzz_targets/recon_frame_plane_types_bytes.rs` as
  evidence
- **AND** it records bounded frame/plane model generation, typed invalid-case
  coverage, and fuzz target enumeration commands
- **AND** it does not mark byte-consuming decode, reconstruction, output
  scheduling, reference refresh, film grain, metadata MD5 verification,
  resource diagnostics, AVM/dav2d differential testing, or filesystem
  publication as supported

### Requirement: Cardinal Directional Prediction Support Row
The decoder support model SHALL track
`RECON-INTRA-CARDINAL-DIRECTIONAL-PREDICTION` as a distinct `splot-recon`
source-backed row named `intra-cardinal-directional-prediction`. The row SHALL
mark only H/V pAngle 90/180 scalar prediction and workspace handoff as
supported, SHALL cite AV2 v1.0.0 §7.13.2.1, §7.13.2.7, §7.13.2.8, and §9.2,
and SHALL keep broad intra reconstruction, broad directional prediction,
runtime decode, transform/residual, loop-filter, and reference-refresh rows
honestly partial or unsupported.

#### Scenario: Matrix records narrow support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `intra-cardinal-directional-prediction` appears with Feature ID
  `RECON-INTRA-CARDINAL-DIRECTIONAL-PREDICTION`
- **AND** it names focused unit/workspace tests plus the extended
  `recon_intra_prediction_bytes` fuzz target
- **AND** it does not claim general directional angles, IDIF, MRL, IBP,
  wide-angle mapping, CfL/CCTX/MHCCP, palette, residuals, transforms, loop
  filters, reference refresh, film grain, AVM/dav2d evidence, or full decoder
  conformance

#### Scenario: Broad rows remain partial
- **WHEN** decoder support and conformance coverage status documents are
  regenerated
- **THEN** `intra-reconstruction`, `prediction-process`, and other broad runtime
  decoder rows remain partial or unsupported until separately implemented with
  runtime evidence

### Requirement: IBP DC Prediction Support Row
The decoder support model SHALL track `RECON-INTRA-IBP-DC-PREDICTION` as a
distinct `splot-recon` source-backed row named `intra-ibp-dc-prediction`. The
row SHALL mark only AV2 §7.13.2.12 prepared-edge scalar prediction and
workspace handoff as supported, SHALL cite AV2 v1.0.0 §3, §4.8, §7.13.2.1,
§7.13.2.10, and §7.13.2.12, and SHALL keep broad intra reconstruction, full
`predict_intra()` dispatch, general directional IBP, runtime decode,
transform/residual, loop-filter, and reference-refresh rows honestly partial or
unsupported.

#### Scenario: Matrix records narrow support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `intra-ibp-dc-prediction` appears with Feature ID
  `RECON-INTRA-IBP-DC-PREDICTION`
- **AND** it names focused unit/workspace tests plus the extended
  `recon_intra_prediction_bytes` fuzz target
- **AND** it does not claim full edge preparation, full dispatcher support,
  directional IBP, data-driven prediction, CfL/CCTX/MHCCP, residuals,
  transforms, loop filters, reference refresh, film grain, AVM/dav2d evidence,
  or full decoder conformance

#### Scenario: Broad rows remain partial
- **WHEN** decoder support and conformance coverage status documents are
  regenerated
- **THEN** `intra-reconstruction`, `prediction-process`, and other broad runtime
  decoder rows remain partial or unsupported until separately implemented with
  runtime evidence

### Requirement: One-Sided Directional Angle Support Row
The decoder support model SHALL track
`RECON-INTRA-ONE-SIDED-DIRECTIONAL-ANGLE-PREDICTION` as a distinct
`splot-recon` source-backed row named
`intra-one-sided-directional-angle-prediction`. The row SHALL mark only AV2
§7.13.2.8 prepared-edge non-IDIF one-sided pAngles `45`, `67`, and `203` as
supported, SHALL cite AV2 v1.0.0 §7.13.2.1, §7.13.2.7, §7.13.2.8, and §9.2,
and SHALL keep broad intra reconstruction, full directional dispatch, middle
angles, luma IDIF, MRL, directional IBP, runtime decode, transform/residual,
loop-filter, and reference-refresh rows honestly partial or unsupported.

#### Scenario: Matrix records narrow directional-angle support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `intra-one-sided-directional-angle-prediction` appears with Feature
  ID `RECON-INTRA-ONE-SIDED-DIRECTIONAL-ANGLE-PREDICTION`
- **AND** it names focused direct primitive tests plus the extended
  `recon_intra_prediction_bytes` fuzz target
- **AND** it does not claim full edge preparation, pAngles outside `45`, `67`,
  and `203`, luma IDIF, MRL, directional IBP, workspace synthesis, runtime
  decode, residuals, transforms, loop filters, reference refresh, film grain,
  AVM/dav2d evidence, or full decoder conformance

#### Scenario: Broad rows remain partial
- **WHEN** decoder support and conformance coverage status documents are
  regenerated
- **THEN** `intra-reconstruction`, `prediction-process`, and other broad runtime
  decoder rows remain partial or unsupported until separately implemented with
  runtime evidence

### Requirement: Middle Directional Angle Support Row
The decoder support model SHALL track
`RECON-INTRA-MIDDLE-DIRECTIONAL-ANGLE-PREDICTION` as a distinct `splot-recon`
source-backed row named `intra-middle-directional-angle-prediction`. The row
SHALL mark only AV2 v1.0.0 7.13.2.8 non-IDIF pAngles `113`, `135`, and `157`
over caller-prepared logical edge ranges as supported, SHALL cite AV2 4.8,
7.13.2.1, 7.13.2.7, 7.13.2.8, and 9.2, and SHALL keep broad intra
reconstruction, edge preparation, IDIF, MRL, directional IBP, runtime decode,
transform/residual, loop-filter, and reference-refresh rows honestly partial
or unsupported.

#### Scenario: Matrix records narrow middle-angle support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `intra-middle-directional-angle-prediction` appears with Feature ID
  `RECON-INTRA-MIDDLE-DIRECTIONAL-ANGLE-PREDICTION`
- **AND** it names focused unit tests plus the extended
  `recon_intra_prediction_bytes` fuzz target
- **AND** it does not claim full edge preparation, IDIF, MRL, data-driven
  prediction, directional IBP, residuals, transforms, loop filters, reference
  refresh, film grain, AVM/dav2d evidence, or full decoder conformance

#### Scenario: Broad directional rows remain partial
- **WHEN** decoder support and conformance coverage status documents are
  regenerated
- **THEN** `intra-reconstruction`, `prediction-process`, and other broad runtime
  decoder rows remain partial or unsupported until separately implemented with
  runtime evidence

### Requirement: Workspace Directional Angle Support Row
The decoder support model SHALL track `RECON-WORKSPACE-DIRECTIONAL-ANGLE-PREDICTION` as a distinct `splot-recon` source-backed row named `workspace-directional-angle-prediction`. The row SHALL mark only current-frame chroma/no-IDIF workspace handoff for fully available in-storage one-sided pAngles `45`, `67`, and `203`, and middle pAngles `113`, `135`, and `157` as supported; SHALL record that `PlaneId::Y` is rejected until luma IDIF is implemented; SHALL cite AV2 v1.0.0 §4.8, §7.13.2.1, §7.13.2.7, §7.13.2.8, and §9.2; and SHALL keep broad intra reconstruction, fallback edge preparation, runtime decode, transform/residual, loop-filter, and reference-refresh rows honestly partial or unsupported.

#### Scenario: Matrix records workspace directional-angle support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support matrix
- **THEN** `workspace-directional-angle-prediction` appears with Feature ID `RECON-WORKSPACE-DIRECTIONAL-ANGLE-PREDICTION`
- **AND** it names focused workspace tests plus the extended `recon_intra_prediction_bytes` fuzz target
- **AND** it does not claim full edge preparation, pAngles outside the modeled one-sided and middle subsets, luma IDIF, MRL, directional IBP, runtime decode, residuals, transforms, loop filters, reference refresh, film grain, AVM/dav2d evidence, or full decoder conformance

#### Scenario: Broad workspace and decoder rows remain partial
- **WHEN** decoder support and conformance coverage status documents are regenerated
- **THEN** `intra-reconstruction`, `prediction-process`, runtime decode, and other broad decoder rows remain partial or unsupported until separately implemented with runtime evidence

### Requirement: Minimal-tier runtime raw output
The decoder support model SHALL provide Feature ID `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT`
as a narrow `splot decode` raw output success path for the existing
`minimal-intra-8bit420-hash-v1` tier. The raw path MUST use the same
byte-consuming validation, tile trace, output sample values, visible geometry,
bit depth, and pixel format already required for
`DECODE-MINIMAL-TIER-RUNTIME-SUCCESS`.

#### Scenario: Explicit raw output succeeds for the minimal fixture
- **WHEN** `splot decode --output-format raw <minimal-ivf-fixture> -o <output.raw>` is run for the committed minimal 64x64 IVF fixture
- **THEN** the command exits successfully
- **AND** stdout and stderr are empty
- **AND** `<output.raw>` contains exactly one headerless `raw_intermediate_output` event encoded as `av2-output-samples-v1`
- **AND** the bytes are visible Y samples, then visible U samples, then visible V samples, with the same flat sample values used by the minimal hash runtime

#### Scenario: Raw output does not require an IVF timebase
- **WHEN** `splot decode --output-format raw <minimal-ivf-fixture> -o <output.raw>` is run for the committed minimal fixture shape with a zero IVF timebase numerator or denominator
- **THEN** the command exits successfully
- **AND** `<output.raw>` contains the same raw sample bytes as the nonzero-timebase fixture

#### Scenario: Out-of-tier raw inputs fail closed
- **WHEN** `splot decode --output-format raw <input> -o <output.raw>` is run for a malformed, resource-limited, or out-of-tier source
- **THEN** the command emits the existing structured `decode/malformed-source`, `decode/resource-limit`, or `decode/unsupported-feature` diagnostic as appropriate
- **AND** it does not create, truncate, or replace `<output.raw>`

### Requirement: Atomic runtime raw publication
The CLI SHALL publish runtime raw output atomically: all raw sample bytes MUST
be decoded and serialized before opening output paths, then written to a
same-directory temporary file, and the requested output path MUST be replaced
only after successful decode, serialization, temp-file write, flush, and file
sync.

#### Scenario: Existing raw output is replaced only after success
- **WHEN** the requested raw output path already contains bytes
- **AND** the minimal runtime raw decode succeeds
- **THEN** the requested path is replaced by the complete raw sample byte stream
- **AND** no temporary output file remains in the output directory

#### Scenario: Raw failure preserves existing output
- **WHEN** the requested raw output path already contains bytes
- **AND** decode, serialization, write, flush, sync, rename, or cleanup fails before publication
- **THEN** the requested path remains byte-for-byte unchanged
- **AND** no partial raw stream is visible at the requested path

#### Scenario: Raw source diagnostics win before output publication
- **WHEN** `splot decode --output-format raw <input> -o <output.raw>` is run for a malformed or out-of-tier source whose output parent cannot be created
- **THEN** the command emits the source diagnostic rather than `decode/output-error`
- **AND** it does not create the missing output parent or requested output path

### Requirement: Runtime raw byte accounting
The runtime raw output path SHALL check `DecodeLimitName::MaxOutputBytes`
against the complete raw visible sample byte stream length before publishing the
file.

#### Scenario: Raw output byte limit rejects before publication
- **WHEN** the configured `max_output_bytes` is smaller than the complete minimal raw sample byte stream length
- **THEN** runtime raw output fails with `decode/resource-limit`
- **AND** the requested output path is not created, truncated, or replaced

#### Scenario: Raw output is deterministic across thread policies
- **WHEN** the same minimal fixture is decoded to raw with `--threads 1`, `--threads auto`, and a fixed positive thread count
- **THEN** each successful command writes byte-identical raw output

### Requirement: Decode runtime raw fuzz entry point
For Feature ID `CONF-DECODE-RUNTIME-RAW-FUZZ`, the fuzz corpus SHALL include a
self-contained `decode_runtime_raw_bytes` target that feeds arbitrary bytes and
bounded mutations of the committed minimal IVF fixture through the runtime raw
byte API without filesystem I/O or external decoder invocation.

#### Scenario: Raw runtime fuzz target is registered
- **WHEN** `cargo xtask check-fuzz-targets` runs
- **THEN** `fuzz/fuzz_targets/decode_runtime_raw_bytes.rs` has a matching `[[bin]]` entry in `fuzz/Cargo.toml`

#### Scenario: Raw runtime fuzz accepts typed outcomes
- **WHEN** `decode_runtime_raw_bytes` runs on arbitrary input
- **THEN** successful cases satisfy only the stable minimal raw output shape
- **AND** malformed, unsupported, resource-limit, and writer-failure paths return typed `DecodeError` values rather than panicking

### Requirement: Tile MI-size state boundary

The decoder support model SHALL track `DECODE-TILE-MI-SIZE-STATE-BOUNDARY` as a
crate-private `splot-decode` boundary for AV2 v1.0.0 § 5.20.4.1 MI-size state
updates over `MiSizes`, `LeftMiSizes`, and `AboveMiSizes`, with § 6.19.2.1
superblock-padded context extents.

#### Scenario: State initializes with clear-context sentinels
- **WHEN** the tile MI-size state boundary is constructed for finite frame MI
  dimensions
- **THEN** both luma and chroma `MiSizes` planes are initialized to the
  clear-context block-size sentinel used by the current minimal runtime frontier
  over superblock-padded row and column extents
- **AND** both luma and chroma `LeftMiSizes` and `AboveMiSizes` lines are
  initialized to the same sentinel over the corresponding padded extent
- **AND** zero dimensions or allocation arithmetic overflow fail with a typed
  crate-private error rather than panicking

#### Scenario: Runtime charges padded state before allocation
- **WHEN** the minimal runtime constructs tile MI-size state from parsed frame
  MI dimensions
- **THEN** it charges the superblock-padded grid cell count to `DecodeLimits`
  before allocation
- **AND** it charges the total MI-state `usize` entry storage bytes to
  `DecodeLimits` before allocation
- **AND** visible dimensions that fit a limit but require larger padded state
  fail with `decode/resource-limit` rather than allocating first

#### Scenario: Luma block update writes MI-size footprint
- **WHEN** a caller applies a checked luma block update with a validated AV2
  block size and in-frame `r`/`c` coordinates
- **THEN** every covered `MiSizes[0][r + y][c + x]` entry is set to that block
  size for the block's `Num_4x4_Blocks_High` by `Num_4x4_Blocks_Wide`
  footprint
- **AND** every covered `LeftMiSizes[0][r + y]` and `AboveMiSizes[0][c + x]`
  entry is set to that block size
- **AND** out-of-bounds or overflowing footprints fail before mutating state

#### Scenario: Luma edge block update may extend into padded context
- **WHEN** a caller applies a checked luma block update whose `r` and `c`
  coordinates are inside visible `MiRows` and `MiCols`
- **AND** the block's full footprint extends beyond visible `MiRows` or `MiCols`
  but remains inside the § 6.19.2.1 superblock-padded context extent
- **THEN** every covered padded `MiSizes[0]`, `LeftMiSizes[0]`, and
  `AboveMiSizes[0]` entry is set to that block size
- **AND** a block whose start coordinate is outside visible dimensions, or whose
  footprint exceeds the padded extent, fails before mutating state

#### Scenario: Chroma block update writes caller-supplied chroma footprint
- **WHEN** a caller applies a checked chroma block update with caller-supplied
  `ChromaMiRow`, `ChromaMiCol`, and `ChromaMiSize`
- **THEN** every covered `MiSizes[1][ChromaMiRow + y][ChromaMiCol + x]` entry
  is set to `ChromaMiSize`
- **AND** every covered `LeftMiSizes[1][ChromaMiRow + y]` and
  `AboveMiSizes[1][ChromaMiCol + x]` entry is set to `ChromaMiSize`
- **AND** out-of-bounds or overflowing chroma footprints fail before mutating
  state

#### Scenario: Existing partition context readers consume state views
- **WHEN** partition traversal or tests request partition-context state
- **THEN** the MI-size state boundary exposes read-only plane and neighbor-line
  views compatible with the existing `TilePartitionContextState` consumer
- **AND** those views reflect successful block updates
- **AND** this does not expose a public API or add scheduler ownership to
  `splot-recon`

#### Scenario: Broad tile decode remains partial
- **WHEN** decoder support and coverage documents are regenerated
- **THEN** `tile-payload-decode`, `tile-cdf-selection-boundary`,
  `intra-reconstruction`, runtime decode, and broad output rows remain partial
  or unsupported until separately implemented with runtime evidence
- **AND** this boundary does not claim full `decode_block()`, recursive
  `read_partition()`, broad `decode_tile()`, transform/residual parsing,
  reconstruction expansion, reference refresh, AVM/dav2d invocation, or
  external decoder integration

### Requirement: Coefficient scan order get_scan

The repository SHALL provide a scheduler-free `splot-recon` primitive for the AV2 § 5.20.7.30 `get_scan` coefficient scan order, tracked by `RECON-COEFFICIENT-SCAN-ORDER`. The `coefficient_scan_order` function SHALL write, for a `w * h` transform block and a `TransformClass`, the order in which transform coefficients are scanned — each `out[c]` being the flattened `y * w + x` position of the c-th scanned coefficient — implementing the three spec classes: `TX_CLASS_VERT` as row-major raster order, `TX_CLASS_HORIZ` as column-major (transpose) order, and `TX_CLASS_2D` as the anti-diagonal scan (each anti-diagonal `x + y` traversed from high `y` / low `x` to low `y` / high `x`). The block shape SHALL be caller-resolved (`w` / `h` each 4, 8, 16, or 32), and the function SHALL return a typed `ReconError` for an unsupported shape or a wrong-length output buffer, total and panic-free. The output for every supported shape and class SHALL be a permutation of `0..w*h`. The primitive SHALL NOT implement `get_tx_class`, the coefficient decode loop, the wiring of the scan into a decode path, the § 7.15.3 secondary-transform scan, or runtime decode output.

#### Scenario: get_scan succeeds with self-contained tests

- **WHEN** `cargo test -p splot-recon coefficient_scan --locked` runs
- **THEN** the test suite covers the hand-traced 4x4 `TX_CLASS_2D` order, the
  `TX_CLASS_VERT` identity and `TX_CLASS_HORIZ` transpose orders, and that the
  output is a valid permutation of `0..w*h` for all 4/8/16/32 shapes and all three
  classes
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: Invalid scan shape or length is typed

- **WHEN** callers request a `w` / `h` outside 4/8/16/32, or an output buffer not
  exactly `w * h` long
- **THEN** `coefficient_scan_order` returns a structured `ReconError`
- **AND** library code does not panic, overflow, or unwrap

#### Scenario: Coefficient decode remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the matrix records the `get_scan` coefficient scan order as supported
- **AND** the coefficient decode loop and broader reconstruction remain partial
  until `get_tx_class`, the decode loop, and the runtime wiring are implemented
  and proven

### Requirement: Coefficient scan class get_tx_class

The repository SHALL provide a scheduler-free `splot-recon` primitive for the AV2 § 8.3.2 `get_tx_class` transform-class derivation, tracked by `RECON-GET-TX-CLASS`. The `tx_class` function SHALL return, for a `PlaneTxType`, the `TransformClass` that selects the § 5.20.7.30 coefficient scan: the vertical-only transforms `V_DCT`, `V_ADST`, and `V_FLIPADST` to the vertical class, the horizontal-only transforms `H_DCT`, `H_ADST`, and `H_FLIPADST` to the horizontal class, and — per the spec `else` branch — every other value, including all 2D and identity transforms and any out-of-range input, to the 2D class. The function SHALL be total and panic-free over all inputs and SHALL reuse the existing `TransformClass` enum without adding an error variant. The primitive SHALL NOT implement the § 5.20.7.29 `compute_tx_type` transform-type computation that produces `PlaneTxType`, the coefficient decode loop, the wiring of the class into a decode path, or runtime decode output.

#### Scenario: get_tx_class succeeds with self-contained tests

- **WHEN** `cargo test -p splot-recon coefficient_scan --locked` runs
- **THEN** the test suite covers each vertical transform type (10, 12, 14)
  mapping to the vertical class, each horizontal transform type (11, 13, 15)
  mapping to the horizontal class, every `0..=9` value mapping to the 2D class,
  and out-of-range inputs mapping to the 2D class
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: get_tx_class is total and panic-free

- **WHEN** callers pass any `PlaneTxType`, including values outside the named
  `TX_TYPE` range
- **THEN** `tx_class` returns a `TransformClass` via the spec `else` branch
- **AND** library code does not panic, overflow, or unwrap

#### Scenario: Coefficient decode remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the matrix records the `get_tx_class` transform-class derivation as
  supported
- **AND** the coefficient decode loop and broader reconstruction remain partial
  until the § 5.20.7.29 `compute_tx_type` transform-type computation, the decode
  loop, and the runtime wiring are implemented and proven

### Requirement: Inverse transform Transform_Shift lookup

The repository SHALL provide a scheduler-free `splot-recon` primitive for the AV2
§ 7.15.4 `Transform_Shift` row and column down-shift lookup, tracked by
`RECON-TRANSFORM-SHIFT-LOOKUP`. The `transform_shift` function SHALL return the
pair `(rowShift, colShift) = (Transform_Shift[txSz][0], Transform_Shift[txSz][1])`
for the `txSz` whose `(Tx_Width_Log2, Tx_Height_Log2)` equals the requested
`(log2_width, log2_height)` shape, transcribed verbatim from the § 7.15.4
constant table. Because `Transform_Shift` is a § 7.15.4 process-body constant
absent from the generated `all_tables.h` § 9 attachment, it SHALL be a
hand-written, spec-cited `splot-recon` constant rather than a `gen-tables`
output. The function SHALL return a typed `ReconError` for a `(log2_width,
log2_height)` pair that is not one of the 25 AV2 `TX_SIZES_ALL` transform shapes,
and SHALL be total and panic-free. The primitive SHALL read no frame, segment, or
tile state and SHALL NOT implement the `get_transform_1d_type` derivation, the
DPCM-direction selection, the wiring of `Transform_Shift` into the runtime decode
path, the § 7.15.3 secondary transform, or the coefficient entropy decode that
produces `Quant`.

#### Scenario: Transform_Shift lookup succeeds with self-contained tests

- **WHEN** `cargo test -p splot-recon transform_params --locked` runs
- **THEN** the test suite covers every `TX_SIZES_ALL` shape against the verbatim
  spec table, the `(log2W, log2H)` key uniqueness invariant, independently
  transcribed spec spot values, and transpose symmetry
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: Non-AV2 transform shape is typed

- **WHEN** callers request a `(log2_width, log2_height)` pair that is not one of
  the 25 AV2 `TX_SIZES_ALL` transform shapes
- **THEN** `transform_shift` returns a structured `ReconError`
- **AND** library code does not panic, overflow, or unwrap

#### Scenario: Full reconstruction remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the matrix records the `Transform_Shift` lookup as supported
- **AND** broader reconstruction remains partial until the `get_transform_1d_type`
  derivation, the coefficient entropy decode, and the runtime transform wiring are
  implemented and proven

### Requirement: Inverse transform get_transform_1d_type derivation

The repository SHALL provide a scheduler-free `splot-recon` primitive for the AV2
§ 7.15.4 `get_transform_1d_type` row and column transform-type derivation, tracked
by `RECON-GET-TRANSFORM-1D-TYPE`. The `get_transform_1d_type` function SHALL
return `Transform_1d_Type[PlaneTxType][dir]` (as the `InverseTransform2dDim` the
2D inverse transform consumes, with `IDT` mapped to `Identity` and the kernel
types to `Kernel`), and SHALL apply the spec `useDdt` substitution: when the
caller-resolved `use_ddt` is set, the base type is `ADST` or `FDST`, and the pass
size is not 4, the type SHALL be replaced by `DDTX` or `FDDT` respectively.
Because `Transform_1d_Type` is a § 7.15.4 process-body constant absent from the
generated `all_tables.h` § 9 attachment, it SHALL be a hand-written, spec-cited
`splot-recon` constant rather than a `gen-tables` output. The function SHALL
return a typed `ReconError` for a `PlaneTxType` outside the `TX_TYPES` range
(`0..16`), and SHALL be total and panic-free. The primitive SHALL read no frame,
segment, or tile state beyond its caller-resolved arguments and SHALL NOT
implement the DPCM-direction selection, the combined transform-parameter resolve
helper, the wiring of `get_transform_1d_type` into the runtime decode path, the
§ 7.15.3 secondary transform, or the coefficient entropy decode that produces
`Quant`.

#### Scenario: get_transform_1d_type succeeds with self-contained tests

- **WHEN** `cargo test -p splot-recon transform_params --locked` runs
- **THEN** the test suite covers every `PlaneTxType` row against the verbatim spec
  `Transform_1d_Type` table for both passes, the `useDdt` substitution (eligible
  and ineligible cases), and the out-of-range rejection
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: Invalid PlaneTxType is typed

- **WHEN** callers request a `PlaneTxType` outside the `TX_TYPES` range
- **THEN** `get_transform_1d_type` returns a structured `ReconError`
- **AND** library code does not panic, overflow, or unwrap

#### Scenario: Full reconstruction remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the matrix records the `get_transform_1d_type` derivation as supported
- **AND** broader reconstruction remains partial until the DPCM-direction
  selection, the runtime transform wiring, and the coefficient entropy decode are
  implemented and proven

### Requirement: eob_extra coefficient CDF bank

The `splot-decode` tile CDF selection subset SHALL include the AV2 `TileEobExtraCdf` coefficient CDF bank, tracked by `DECODE-TILE-CDF-SELECTION-BOUNDARY`. The bank SHALL be copied from the generated AV2 § 9.3 `Default_Eob_Extra_Cdf` defaults, and SHALL be selectable by `coeff_cdf_q_ctx` with no per-symbol context (AV2 § 8.3.2: the cdf for `eob_extra` is given directly by `TileEobExtraCdf`). A `coeff_cdf_q_ctx`
outside the valid range SHALL return a typed `SelectorOutOfRange` error naming the
`TileEobExtraCdf` array, never panicking. The bank SHALL participate in the
supported-subset tile copy/average and frame-end count-scaling paths. The bank is
loaded but not consumed by a decode loop in this change (the § 5.20.7.27
`coeffs()` syntax that reads it is not wired), so the minimal-fixture decode
output SHALL be unchanged. Broader § 8.3 coefficient CDF selection and the
coefficient decode loop remain partial.

#### Scenario: eob_extra bank loads the generated defaults and selects by q-context

- **WHEN** `cargo test -p splot-decode tile_payload --locked` runs
- **THEN** the frame CDF subset copies `Default_Eob_Extra_Cdf` into the
  `eob_extra` bank without aliasing, and the `EobExtra { coeff_cdf_q_ctx }`
  selector returns the matching row for each valid `coeff_cdf_q_ctx`
- **AND** an out-of-range `coeff_cdf_q_ctx` returns a typed `SelectorOutOfRange`
  naming `TileEobExtraCdf`, and library code does not panic

#### Scenario: Adding the bank does not change decode output

- **WHEN** the minimal flat-intra fixture is decoded to a hash, raw, or Y4M output
- **THEN** the output bytes are identical to before the bank was added (the bank
  is loaded but not read by any decode path)

#### Scenario: Coefficient CDF selection remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the tile CDF selection boundary records the `eob_extra` bank
- **AND** broader § 8.3 coefficient CDF selection (the remaining banks and the
  coefficient decode loop) remains partial

### Requirement: eob_pt coefficient CDF family

The `splot-decode` tile CDF selection subset SHALL include the AV2 `eob_pt` coefficient CDF family — the seven transform-size class banks `TileEobPt16Cdf` through `TileEobPt1024Cdf` — tracked by `DECODE-TILE-CDF-SELECTION-BOUNDARY`. Each bank SHALL be copied from its generated AV2 § 9.3 `Default_Eob_Pt_<size>_Cdf` defaults, and SHALL be selectable by an `EobPtSize` transform-size class together with `coeff_cdf_q_ctx` and `eobCtx` (AV2 § 8.3.2: `eob_pt_<size>` reads `TileEobPt<size>Cdf[eobCtx]`, with `eobCtx = (plane > 0) ? 2 : is_inter`). A `coeff_cdf_q_ctx` or `eob_ctx` outside its valid range SHALL return a typed `SelectorOutOfRange` error naming the `eob_pt` family, never panicking. The family SHALL participate in the supported-subset tile copy/average and frame-end count-scaling paths. The family is loaded but not consumed by a decode loop in this change (the § 5.20.7.27 `coeffs()` syntax that reads it is not wired), so the minimal-fixture decode output SHALL be unchanged. Broader § 8.3 coefficient CDF selection and the coefficient decode loop remain partial.

#### Scenario: eob_pt banks load defaults and select by size and context

- **WHEN** `cargo test -p splot-decode tile_payload --locked` runs
- **THEN** each of the seven `eob_pt` banks copies its `Default_Eob_Pt_<size>_Cdf`
  table, and the `EobPt { size, coeff_cdf_q_ctx, eob_ctx }` selector returns the
  matching row for every valid size, `coeff_cdf_q_ctx`, and `eob_ctx`
- **AND** an out-of-range `coeff_cdf_q_ctx` or `eob_ctx` returns a typed
  `SelectorOutOfRange` naming the `eob_pt` family, and library code does not panic

#### Scenario: Adding the family does not change decode output

- **WHEN** the minimal flat-intra fixture is decoded to a hash, raw, or Y4M output
- **THEN** the output bytes are identical to before the family was added (the banks
  are loaded but not read by any decode path)

#### Scenario: Coefficient CDF selection remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the tile CDF selection boundary records the `eob_pt` family
- **AND** broader § 8.3 coefficient CDF selection (the remaining banks and the
  coefficient decode loop) remains partial

### Requirement: dc_sign coefficient CDF bank

The `splot-decode` tile CDF selection subset SHALL include the AV2 `TileDcSignCdf` coefficient CDF bank, tracked by `DECODE-TILE-CDF-SELECTION-BOUNDARY`. The bank SHALL be copied from the generated AV2 § 9.3 `Default_Dc_Sign_Cdf` defaults, and SHALL be selectable by `coeff_cdf_q_ctx`, `plane_type`, the `isHidden` group, and the DC-sign `ctx` (AV2 § 8.3.2: `dc_sign` reads `TileDcSignCdf[ptype][isHidden][ctx]`). Each of the four selector index axes SHALL be bounds-checked and return a typed `SelectorOutOfRange` error naming the `dc_sign` bank, never panicking. The bank SHALL participate in the supported-subset tile copy/average and frame-end count-scaling paths. The § 8.3.2 `ctx` derivation from the Above/Left DC-context buffers is not implemented in this change (those buffers do not exist yet), so the bank is loaded but not consumed by a decode loop, and the minimal-fixture decode output SHALL be unchanged. Broader § 8.3 coefficient CDF selection and the coefficient decode loop remain partial.

#### Scenario: dc_sign bank loads defaults and selects across all indices

- **WHEN** `cargo test -p splot-decode tile_payload --locked` runs
- **THEN** the `dc_sign` bank copies `Default_Dc_Sign_Cdf`, and the
  `DcSign { coeff_cdf_q_ctx, plane_type, group, ctx }` selector returns the
  matching row for every valid combination of the four indices
- **AND** an out-of-range value on any of the four axes returns a typed
  `SelectorOutOfRange` naming the `dc_sign` bank, and library code does not panic

#### Scenario: Adding the bank does not change decode output

- **WHEN** the minimal flat-intra fixture is decoded to a hash, raw, or Y4M output
- **THEN** the output bytes are identical to before the bank was added (the bank
  is loaded but not read by any decode path)

#### Scenario: Coefficient CDF selection remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the tile CDF selection boundary records the `dc_sign` bank
- **AND** broader § 8.3 coefficient CDF selection (the `ctx` derivation, the
  remaining banks, and the coefficient decode loop) remains partial

### Requirement: Coefficient base position CDF contexts

The `splot-decode` tile CDF selection subset SHALL derive the two position-only AV2 § 8.3.2 coefficient base CDF contexts, tracked by `DECODE-TILE-CDF-SELECTION-BOUNDARY`. `coeff_base_eob_ctx` SHALL compute the `coeff_base_eob` context by partitioning the scan position `c` against the adjusted transform block's coefficient count `Tx_Height[adjTxSz] << Tx_Width_Log2[adjTxSz]`: `0` when `c` is `0`, `1` when `c` is at most one eighth of the count, `2` when `c` is at most one quarter, and `3` otherwise (the `SIG_COEF_CONTEXTS_EOB - 4 ..= SIG_COEF_CONTEXTS_EOB - 1` contexts). `coeff_base_bob_ctx` SHALL compute the `coeff_base_bob` context by partitioning the begin position `bob` against the segment end-of-block `seg_eob`: `0` when `bob` is at most `seg_eob >> 3`, `1` when at most `seg_eob >> 2`, and `2` otherwise. Both SHALL be pure functions of caller-supplied scan and segment scalars plus caller-resolved adjusted geometry (needing no `Level[]` magnitude buffer), SHALL be total and panic-free (including an out-of-range shift width), and SHALL NOT be read by any decode path in this change, so the minimal-fixture decode output SHALL be unchanged. The `Level[]`-dependent coefficient contexts, the sign contexts, the per-transform-block level and sign buffers, and the coefficient decode loop remain partial.

#### Scenario: Coefficient base position contexts partition the position

- **WHEN** `cargo test -p splot-decode tile_payload --locked` runs
- **THEN** `coeff_base_eob_ctx` returns the four contexts across the
  `numCoeffs / 8` and `numCoeffs / 4` boundaries for TX_32X32 and TX_4X4
  geometry, and `coeff_base_bob_ctx` returns contexts 0/1/2 across the
  `seg_eob >> 3` and `seg_eob >> 2` boundaries
- **AND** an out-of-range shift width does not panic, and library code does not
  panic, overflow, or unwrap

#### Scenario: Adding the contexts does not change decode output

- **WHEN** the minimal flat-intra fixture is decoded to a hash, raw, or Y4M output
- **THEN** the output bytes are identical to before the contexts were added (the
  derivations are not read by any decode path)

#### Scenario: Coefficient CDF selection remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the tile CDF selection boundary records the two position-only
  coefficient base contexts
- **AND** broader § 8.3 coefficient CDF selection (the `Level[]`-dependent
  contexts, the sign contexts, and the coefficient decode loop) remains partial

### Requirement: coeff_br coefficient base-range CDF context

The `splot-decode` tile CDF selection subset SHALL derive the AV2 § 8.3.2 `coeff_br` coefficient base-range CDF context, tracked by `DECODE-TILE-CDF-SELECTION-BOUNDARY`. `CoeffBrContext::ctx` SHALL, for a coefficient at scan position `pos` in an adjusted transform block of caller-resolved geometry (`bwl`, `txw`, `txh`) and a caller-provided row-major `Level[]` magnitude slice, compute the context by: deriving `row`/`col` from `pos` and `bwl`; summing up to three neighbour magnitudes at the § 8.3.2 `Mag_Ref_Offset_With_Tx_Class` offsets for the transform class (only the first two offsets when the transform class is not 2D and the plane is chroma), each clamped to `MAX_BASE_BR_RANGE - 1`; halving and clamping the sum as `Min((mag + 1) >> 1, 6)`; and offsetting it by plane (chroma `Min(mag, 3)`), DC position (non-2D `mag + 7`), or low-frequency (`mag + 7`). It SHALL read the level magnitudes over the caller-provided slice with the spec `refRow < txh && refCol < txw` bound (and a slice-length guard), so out-of-bounds and short-slice neighbour reads contribute `0` and the function is total and panic-free. It SHALL NOT be read by any decode path in this change, so the minimal-fixture decode output SHALL be unchanged. The remaining `Level[]`-dependent contexts, the sign contexts, the full per-transform-block level/sign buffers, and the coefficient decode loop remain partial.

#### Scenario: coeff_br sums and offsets the context per the spec

- **WHEN** `cargo test -p splot-decode tile_payload --locked` runs
- **THEN** `CoeffBrContext::ctx` sums the clamped neighbour magnitudes, halves and
  clamps to 6, and applies the plane / DC-position / low-frequency offsets, with
  tests pinning the chroma `Min(mag, 3)` clamp, the non-2D `mag + 7` offset, and
  the non-2D-chroma two-neighbour case (distinguished from the three-neighbour
  case)
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: coeff_br is total over out-of-bounds and short slices

- **WHEN** neighbour offsets leave the transform block, or the `Level[]` slice is
  shorter than the block
- **THEN** those reads contribute `0` and `ctx` returns without panicking
- **AND** library code does not panic, overflow, or unwrap

#### Scenario: Adding the context does not change decode output

- **WHEN** the minimal flat-intra fixture is decoded to a hash, raw, or Y4M output
- **THEN** the output bytes are identical to before the context was added (the
  derivation is not read by any decode path)

#### Scenario: Coefficient CDF selection remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the tile CDF selection boundary records the `coeff_br` context
- **AND** broader § 8.3 coefficient CDF selection (the remaining `Level[]`-dependent
  contexts, the sign contexts, and the coefficient decode loop) remains partial

### Requirement: IDTX coefficient magnitude CDF contexts

The `splot-decode` tile CDF selection subset SHALL derive the two AV2 § 8.3.2 identity-transform coefficient magnitude CDF contexts, tracked by `DECODE-TILE-CDF-SELECTION-BOUNDARY`. `coeff_base_idtx_ctx` SHALL compute the `coeff_base_idtx` context as `Min(3, Level[row][col-1]) + Min(3, Level[row-1][col])` (each neighbour included only when in range), and `coeff_br_idtx_ctx` SHALL compute the `coeff_br_idtx` context the same way with the `MAX_BASE_BR_RANGE - 1` per-neighbour clamp followed by `Min(mag, 6)`. Both SHALL read a caller-provided row-major `txw`-wide `Level[]` slice (`level[row * txw + col]`), with saturating flat-index geometry and a slice-length guard so out-of-range or short-slice reads contribute `0` and the functions are total and panic-free. Both results SHALL be the spec `mag`, used directly as the inner CDF index. They SHALL NOT be read by any decode path in this change, so the minimal-fixture decode output SHALL be unchanged. The remaining `Level[]`-dependent context, the sign contexts, the full level/sign buffers, and the coefficient decode loop remain partial.

#### Scenario: IDTX magnitude contexts sum the clamped neighbours

- **WHEN** `cargo test -p splot-decode tile_payload --locked` runs
- **THEN** `coeff_base_idtx_ctx` sums the left and above neighbours clamped to 3,
  and `coeff_br_idtx_ctx` sums them clamped to `MAX_BASE_BR_RANGE - 1` then clamps
  the total to 6, with tests pinning the col==0 / row==0 missing-neighbour skips
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: IDTX contexts are total over short slices and bad geometry

- **WHEN** the `Level[]` slice is shorter than the block or the geometry is
  malformed
- **THEN** out-of-range reads contribute `0` and the functions return without
  panicking
- **AND** library code does not panic, overflow, or unwrap

#### Scenario: Adding the contexts does not change decode output

- **WHEN** the minimal flat-intra fixture is decoded to a hash, raw, or Y4M output
- **THEN** the output bytes are identical to before the contexts were added (the
  derivations are not read by any decode path)

#### Scenario: Coefficient CDF selection remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the tile CDF selection boundary records the two IDTX magnitude contexts
- **AND** broader § 8.3 coefficient CDF selection (the `coeff_base` context, the
  sign contexts, and the coefficient decode loop) remains partial

### Requirement: coeff_base significant-coefficient CDF context

The `splot-decode` tile CDF selection subset SHALL derive the AV2 § 8.3.2 `coeff_base` significant-coefficient CDF context, tracked by `DECODE-TILE-CDF-SELECTION-BOUNDARY`. `CoeffBaseContext::select` SHALL sum the significant-neighbour `Level[]` magnitudes at the generated § 9.2 `Sig_Ref_Diff_Offset[txClass]` offsets — over `SIG_REF_DIFF_OFFSET_NUM` samples for luma, 3 for chroma 2D, and 2 for chroma non-2D — each clamped by the position-dependent `magLimit` (5 for the low-frequency near-DC samples unless the coefficient is the parity-hidden DC, else 3), form `ctx = (mag + 1) >> 1`, and return a `CoeffBaseSelection` naming one of the five `coeff_base` banks with its bank-specific context offset: the parity-hidden DC bank (`Min(ctx, 4)`, overriding the others when `isHidden` and `c == 0`), the chroma and chroma low-frequency banks (`Min(ctx, 3)` plus the plane and 2D offsets), the luma low-frequency bank (the `c == 0` / `row + col` / horiz-col-vert-row sub-branches over `LF_SIG_COEF_CONTEXTS_2D`), and the luma high-frequency bank (`Min(ctx, 4)` plus the `row + col` position buckets, or `+ 15` for non-2D). It SHALL read a caller-provided row-major `txw`-wide `Level[]` slice with checked shifts, saturating flat-index geometry, and a slice-length guard (the spec `refRow < height && refCol < width` guard), so out-of-range or short-slice reads contribute `0` and the function is total and panic-free, and SHALL use the generated `Sig_Ref_Diff_Offset` conversion table rather than a duplicate. It SHALL NOT be read by any decode path in this change, so the minimal-fixture decode output SHALL be unchanged. The sign contexts, the full per-transform-block level/sign buffers, and the coefficient decode loop remain partial.

#### Scenario: coeff_base selects the right bank and context per branch

- **WHEN** `cargo test -p splot-decode tile_payload --locked` runs
- **THEN** `CoeffBaseContext::select` returns the parity-hidden, chroma,
  chroma-low-frequency, luma-low-frequency, and luma-high-frequency bank variants
  with the spec context offsets, with tests pinning the high-frequency `row + col`
  buckets, the non-2D `+ 15`, the low-frequency sub-branches, the chroma U-vs-V
  and 2D-vs-non-2D offsets, the clamped neighbour sum, the magLimit raise, and the
  parity-hidden override
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: coeff_base is total over short slices and bad geometry

- **WHEN** the `Level[]` slice is shorter than the block or the geometry is
  malformed
- **THEN** out-of-range neighbour reads contribute `0` and `select` returns
  without panicking
- **AND** library code does not panic, overflow, or unwrap

#### Scenario: Adding the context does not change decode output

- **WHEN** the minimal flat-intra fixture is decoded to a hash, raw, or Y4M output
- **THEN** the output bytes are identical to before the context was added (the
  derivation is not read by any decode path)

#### Scenario: Coefficient CDF selection remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the tile CDF selection boundary records the `coeff_base` context
- **AND** broader § 8.3 coefficient CDF selection (the sign contexts and the
  coefficient decode loop) remains partial

### Requirement: dc_sign sign CDF context

The `splot-decode` tile CDF selection subset SHALL derive the AV2 § 8.3.2 `dc_sign` sign CDF context, tracked by `DECODE-TILE-CDF-SELECTION-BOUNDARY`. `dc_sign_ctx` SHALL net the DC-sign votes of the block's above and left neighbours — `AboveDcContext[plane][x4 + k]` for `k` in `0..w4` and `LeftDcContext[plane][y4 + k]` for `k` in `0..h4`, where a sign value of `1` decrements and `2` increments a running `dcSign` — and return `1` when `dcSign < 0`, `2` when `dcSign > 0`, and `0` otherwise (the inner index of `TileDcSignCdf[ptype][isHidden][ctx]`). `above_dc` and `left_dc` SHALL be the caller-supplied `AboveDcContext` / `LeftDcContext` plane slices whose lengths are the spec `MiCols` / `MiRows` bounds, so reads past either slice are skipped; the loop SHALL break once the monotonic index leaves the slice, so a pathological `w4` / `h4` cannot spin and the function is total and panic-free. It SHALL NOT be read by any decode path in this change, so the minimal-fixture decode output SHALL be unchanged. The `idtx_sign` sign context, the DC-context buffers, and the coefficient decode loop remain partial.

#### Scenario: dc_sign nets above and left votes

- **WHEN** `cargo test -p splot-decode tile_payload --locked` runs
- **THEN** `dc_sign_ctx` returns context 1 for a net-negative neighbour sum, 2 for
  net-positive, and 0 for a balanced or empty sum, with tests pinning the position
  offset and the out-of-slice (`MiCols` / `MiRows`) skip
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: dc_sign is total over pathological geometry

- **WHEN** `w4` / `h4` or the position offsets are far larger than the slices
- **THEN** the loop terminates without spinning and `dc_sign_ctx` returns without
  panicking
- **AND** library code does not panic, overflow, or unwrap

#### Scenario: Adding the context does not change decode output

- **WHEN** the minimal flat-intra fixture is decoded to a hash, raw, or Y4M output
- **THEN** the output bytes are identical to before the context was added (the
  derivation is not read by any decode path)

#### Scenario: Coefficient CDF selection remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the tile CDF selection boundary records the `dc_sign` sign context
- **AND** broader § 8.3 coefficient CDF selection (the `idtx_sign` context, the
  DC-context buffers, and the coefficient decode loop) remains partial

### Requirement: idtx_sign sign CDF context

The `splot-decode` tile CDF selection subset SHALL derive the AV2 § 8.3.2 `idtx_sign` sign CDF context, tracked by `DECODE-TILE-CDF-SELECTION-BOUNDARY`. `idtx_sign_ctx` SHALL net the signs of the left (`QuantSign[row*txw + col-1]`), above (`QuantSign[(row-1)*txw + col]`), and above-left (`QuantSign[(row-1)*txw + col-1]`) coefficients into `signc` — the edge neighbours gated by `col > 0` and `row > 0` — map `signc` to a base context (`5` when `signc > 2`, `6` when `signc < -2`, `1` when `signc > 0`, `2` when `signc < 0`, else `0`), and add `2` when `Level[row][col]` exceeds `COEFF_BASE_RANGE` and the base context is non-zero (the inner index of `TileIdtxSignCdf[Min(TX_16X16, txSzCtx)][ctx]`). It SHALL read caller-provided row-major `txw`-wide `QuantSign[]` and `Level[]` slices with saturating flat-index geometry and a slice-length guard, so out-of-range reads contribute `0` and the function is total and panic-free. It SHALL NOT be read by any decode path in this change, so the minimal-fixture decode output SHALL be unchanged. The per-transform-block level/sign tile buffers and the coefficient decode loop remain partial.

#### Scenario: idtx_sign maps the neighbour sign sum and level threshold

- **WHEN** `cargo test -p splot-decode tile_payload --locked` runs
- **THEN** `idtx_sign_ctx` returns the base context for each `signc` bucket
  (5/6/1/2/0) and adds 2 only when the base context is non-zero and the current
  level exceeds `COEFF_BASE_RANGE`, with tests pinning the missing-edge-neighbour
  skips and the threshold boundary
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: idtx_sign is total over short slices and bad geometry

- **WHEN** the `QuantSign[]` / `Level[]` slices are shorter than the block or the
  geometry is malformed
- **THEN** out-of-range reads contribute `0` and `idtx_sign_ctx` returns without
  panicking
- **AND** library code does not panic, overflow, or unwrap

#### Scenario: Adding the context does not change decode output

- **WHEN** the minimal flat-intra fixture is decoded to a hash, raw, or Y4M output
- **THEN** the output bytes are identical to before the context was added (the
  derivation is not read by any decode path)

#### Scenario: Coefficient CDF selection remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the tile CDF selection boundary records the `idtx_sign` sign context
  (completing the § 8.3.2 coefficient-symbol contexts)
- **AND** broader coefficient decode (the per-transform-block level/sign tile
  buffers and the `coeffs()` loop) remains partial

### Requirement: Tile coefficient state buffers

The decoder support model SHALL track `DECODE-TILE-COEFF-STATE-BUFFERS` as a
crate-private `splot-decode` row named `tile-coeff-state-buffers`. The row SHALL
cover decode-owned state for AV2 §5.20.7.27 transform-block-local `Level[]` and
`QuantSign[]` buffers and the tile-neighbour `AboveLevelContext`,
`LeftLevelContext`, `AboveDcContext`, and `LeftDcContext` lines read by §8.3.2
coefficient contexts. The row SHALL remain partial until the §5.20.7.27
`coeffs()` loop reads symbols, fills `Quant[]`, and wires reconstruction.

#### Scenario: Transform block buffers are bounded and initialized

- **WHEN** a transform-block coefficient state is constructed for caller-resolved
  adjusted dimensions
- **THEN** it allocates zeroed row-major `Level[]` and `QuantSign[]` arrays for at
  most the §5.20.7.27 32x32 adjusted block extent
- **AND** zero dimensions, dimensions above 32x32, arithmetic overflow, or
  allocation failure return typed errors rather than panicking

#### Scenario: Coefficient context lines update like coeffs

- **WHEN** a coefficient block completes with caller-supplied `culLevel`,
  `dcCategory`, `plane`, `x4`, `y4`, `w4`, and `h4`
- **THEN** the tile state writes `culLevel` to `AboveLevelContext[plane]` and
  `LeftLevelContext[plane]` over the block's above and left ranges
- **AND** it writes `dcCategory` to `AboveDcContext[plane]` and
  `LeftDcContext[plane]` over the same ranges
- **AND** out-of-range plane or coordinate facts return typed errors rather than
  panicking or silently wrapping

#### Scenario: Coefficient context lines reset like reset_block_context

- **WHEN** block syntax requests a level/DC context reset for caller-resolved
  plane, start, size, and subsampling facts
- **THEN** the tile state zeros the matching above and left level/DC context
  ranges
- **AND** the operation is bounded by the actual owned line lengths and cannot
  spin on pathological caller counts

#### Scenario: State does not change decode output yet

- **WHEN** the minimal flat-intra fixture is decoded to hash, raw, or Y4M output
- **THEN** output bytes remain unchanged because this change does not wire the
  §5.20.7.27 `coeffs()` symbol loop or reconstruction

#### Scenario: Broader coefficient decode remains incomplete

- **WHEN** decoder support and conformance coverage are generated
- **THEN** `tile-coeff-state-buffers` appears as a partial row linked to
  `DECODE-TILE-COEFF-STATE-BUFFERS`
- **AND** `tile-payload-decode`, `tile-cdf-selection-boundary`, reconstruction,
  and full decoder conformance remain partial

### Requirement: Coeff all_zero block state handoff

The decoder support model SHALL track `DECODE-COEFF-ALL-ZERO-BLOCK-STATE` as a
crate-private `splot-decode` row named `coeff-all-zero-block-state`. The row
SHALL cover the §5.20.7.27 `all_zero == 1` coefficient-block state effects for
the currently traced minimal luma and V branches: zero coefficient state,
`eob == 0`, zero `culLevel` / `dcCategory`, and above/left context-line writes
through `TileCoeffContextState`. The row SHALL remain partial until the full
§5.20.7.27 `coeffs()` loop reads nonzero EOB and coefficient symbols, fills
nonzero `Quant[]`, and wires reconstruction.

#### Scenario: Transform block state includes Quant

- **WHEN** crate-private transform coefficient block state is initialized for a
  caller-resolved adjusted extent
- **THEN** the decoder allocates zeroed row-major `Level[]`, `QuantSign[]`, and
  `Quant[]` buffers with checked dimensions
- **AND** checked accessors reject out-of-range coordinates or positions without
  panicking

#### Scenario: All-zero block applies coefficient context writes

- **WHEN** the all-zero coefficient-block helper is applied for caller-resolved
  plane coordinates and 4x4 transform dimensions
- **THEN** it returns `eob == 0`, `culLevel == 0`, and `dcCategory == 0`
- **AND** it initializes zero `Level[]`, `QuantSign[]`, and `Quant[]` state
- **AND** it writes zero level/DC values to the covered above and left tile
  context ranges through `TileCoeffContextState`
- **AND** malformed ranges fail with typed coefficient-state errors before
  mutating context state

#### Scenario: Minimal trace writes all-zero state

- **WHEN** the minimal flat-intra block-symbol trace reads the existing luma
  `txb_skip` and V `v_txb_skip` symbols as all-zero
- **THEN** it applies the all-zero block state helper after each read
- **AND** the no-output-change symbol-frontier test remains unchanged

#### Scenario: Full coefficient decode remains incomplete

- **WHEN** decoder support and conformance coverage are generated
- **THEN** `coeff-all-zero-block-state` appears as a partial row linked to
  `DECODE-COEFF-ALL-ZERO-BLOCK-STATE`
- **AND** nonzero EOB decode, coefficient scan walk, coefficient base/br/sign
  reads, `read_quant`, dequantization, reconstruction, and full decoder
  conformance remain partial

### Requirement: Decoder support matrix tracks coefficient EOB value state

The decoder support matrix SHALL include a partial row named
`coeff-eob-value-state`, tracked by Feature ID
`DECODE-COEFF-EOB-VALUE-STATE`, for the crate-private AV2 § 5.20.7.27 helper
that derives a nonzero `eob` value from caller-decoded `eobPt`, `eob_extra`, and
packed `eob_extra_bit` refinements. The row SHALL keep broad coefficient decode
and decoded-output support partial until later changes read the `eob_pt_*` CDF
rows, walk the coefficient scan, fill nonzero coefficient state, and run
reconstruction.

#### Scenario: EOB value helper is scoped and test-backed

- **WHEN** `cargo xtask check-decoder-support` renders decoder support status
- **THEN** `coeff-eob-value-state` appears with Feature ID
  `DECODE-COEFF-EOB-VALUE-STATE`
- **AND** it records focused tests for small `eobPt`, refined `eob_extra`, max
  AV2 EOB, and invalid caller-provided EOB parts
- **AND** it cites AV2 § 5.20.7.27 through
  `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`

#### Scenario: Broad coefficient decode remains partial

- **WHEN** decoder support and conformance coverage status documents are
  regenerated after this change
- **THEN** broad tile payload, symbol/CDF, and coefficient-loop rows remain
  partial for actual `eob_pt_*` symbol reads, `eob_extra` symbol reads,
  `eob_extra_bit` literal reads, scan-order traversal, coefficient base/br/sign
  symbol reads, nonzero `Quant[]` writes, `read_quant`, dequantization, inverse
  transform, residual addition, and decoded output changes

### Requirement: Decoder support matrix tracks coefficient EOB symbol reads

The decoder support matrix SHALL include a partial row named
`coeff-eob-symbol-read`, tracked by Feature ID
`DECODE-COEFF-EOB-SYMBOL-READ`, for the crate-private AV2 § 5.20.7.27 helper
that reads the caller-selected `eob_pt_*` symbol, any size-specific
`eob_pt_*_extra` literal bits, `eob_extra`, and any `eob_extra_bit` refinement
literals before producing the checked nonzero EOB value. The row SHALL keep
broad coefficient decode and decoded-output support partial until later changes
wire the helper into the coefficient scan and coefficient state writes.

#### Scenario: EOB symbol helper is scoped and test-backed

- **WHEN** `cargo xtask check-decoder-support` renders decoder support status
- **THEN** `coeff-eob-symbol-read` appears with Feature ID
  `DECODE-COEFF-EOB-SYMBOL-READ`
- **AND** it records focused tests for EOB point CDF consumption, EOB extra CDF
  consumption, size-class extra literal handling, invalid selector rollback
  before reads, and disabled CDF update behavior
- **AND** it cites AV2 § 5.20.7.27 and § 8.3.2 through the committed spec mirror

#### Scenario: Broad coefficient decode remains partial

- **WHEN** decoder support and conformance coverage status documents are
  regenerated after this change
- **THEN** broad tile payload, symbol/CDF, and coefficient-loop rows remain
  partial for transform-size derivation, scan-order traversal, coefficient
  base/br/sign symbol reads, nonzero `Level[]` and `Quant[]` writes,
  `read_quant`, dequantization, inverse transform, residual addition, and
  decoded output changes

### Requirement: Coefficient scan walk support row
The decoder support model SHALL track `DECODE-COEFF-SCAN-WALK` as a distinct
crate-private row named `coeff-scan-walk`. The row SHALL mark only the
decode-side, caller-supplied ordinary non-FSC § 5.20.7.27 coefficient scan walk
boundary as supported, and SHALL keep scan-table derivation, transform-type
computation, coefficient base/BR/sign reads, `read_quant`, dequantization,
inverse transform, residual add, and runtime nonzero coefficient blocks partial
or unsupported until separately implemented.

#### Scenario: Matrix records narrow scan-walk support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix after this change
- **THEN** `coeff-scan-walk` appears with Feature ID `DECODE-COEFF-SCAN-WALK`
- **AND** it cites AV2 § 5.20.7.27 as the scan-walk syntax boundary
- **AND** it names focused tests for reverse order, EOB length rejection, and
  out-of-range scan-position rejection
- **AND** it does not claim runtime nonzero coefficient decode or output support

### Requirement: Coefficient base CDF row support
The decoder support model SHALL track `DECODE-COEFF-BASE-CDF-ROWS` as a
distinct crate-private row named `coeff-base-cdf-rows`. The row SHALL mark only
loaded-but-unread tile CDF row storage, selection, and lifecycle coverage for
ordinary non-IDTX coefficient base, base-EOB, and base-range symbol families as
supported, and SHALL keep coefficient symbol reads, nonzero coefficient writes,
`read_quant`, dequantization, inverse transform, residual add, and runtime
nonzero coefficient blocks partial or unsupported until separately implemented.

#### Scenario: Matrix records narrow CDF-row support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix after this change
- **THEN** `coeff-base-cdf-rows` appears with Feature ID
  `DECODE-COEFF-BASE-CDF-ROWS`
- **AND** it cites AV2 § 8.3.2 and § 9.3 as the CDF-selection/default-table
  boundary
- **AND** it names focused tests for generated-default loading, selector bounds
  errors, tile-copy non-aliasing, and mutable row handoff
- **AND** it does not claim runtime coefficient symbol reads or output support

### Requirement: Coefficient base symbol-read support
The decoder support model SHALL track `DECODE-COEFF-BASE-SYMBOL-READ` as a
distinct crate-private row named `coeff-base-symbol-read`. The row SHALL mark
only ordinary non-FSC coefficient base/base-EOB/base-range symbol-read sequencing
over caller-resolved scan and selector facts as implemented, and SHALL keep
runtime coefficient-state writes, `read_quant`, reconstruction, and broad
`decode_block()` support partial or unsupported until separately implemented.

#### Scenario: Matrix records narrow symbol-read support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix after this change
- **THEN** `coeff-base-symbol-read` appears with Feature ID
  `DECODE-COEFF-BASE-SYMBOL-READ`
- **AND** it cites AV2 §5.20.7.27 and §8.3.2 as the coefficient-loop read-order
  and CDF-selection boundary
- **AND** it names focused tests for direct-read equivalence, scan-entry
  matching, base-range conditional reads, disabled CDF updates, and
  invalid-selector no-consumption behavior
- **AND** it does not claim nonzero `Quant[]` output, `read_quant`,
  reconstruction, external decoder invocation, public API, or broad runtime
  `decode_tile()` support

### Requirement: Coefficient level state-write support
The decoder support model SHALL track `DECODE-COEFF-LEVEL-STATE-WRITE` as a
distinct crate-private row named `coeff-level-state-write`. The row SHALL mark
only ordinary non-FSC decoded level application into local `Level[]` state as
implemented, and SHALL keep sign reads, `QuantSign[]`, `Quant[]`, `read_quant`,
reconstruction, and broad `decode_block()` support partial or unsupported until
separately implemented.

#### Scenario: Matrix records narrow level-write support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix after this change
- **THEN** `coeff-level-state-write` appears with Feature ID
  `DECODE-COEFF-LEVEL-STATE-WRITE`
- **AND** it cites AV2 §5.20.7.27 as the `Level[row][col] = level`
  state-application boundary
- **AND** it names focused tests for row-major placement, untouched quantization
  state, scan-entry mismatch rejection, and mismatched geometry rejection
- **AND** it does not claim sign reads, nonzero `Quant[]` output, `read_quant`,
  reconstruction, external decoder invocation, public API, or broad runtime
  `decode_tile()` support
