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
  fuzzing rather than AV2 normative conformance limits
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

The repository SHALL document the first intended `splot decode` success tier as
a repository-owned implementation subset before any runtime decode path, hash
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
  output samples are the first success artifact
- **AND** Y4M output remains unsupported until the `output-y4m` row is
  implemented and tested against the same cropped visible output samples
- **AND** streams outside the tier SHALL fail with structured
  `decode/unsupported-feature` diagnostics that identify the blocking matrix row
  where possible, while limit overflow or configured-limit excess SHALL use the
  emitted `decode/resource-limit` diagnostic when surfaced through `splot decode`

#### Scenario: Contract remains non-executable until implementation

- **WHEN** `minimal-decode-tier-contract` is still contract-only
- **THEN** the decoder support matrix marks the row as `partial`
- **AND** the row records self-contained docs/OpenSpec proof commands
- **AND** the row does not claim source implementation, runtime byte
  consumption, stream traversal, layer selection, reconstruction, frame hashes,
  Y4M output, fixture support, fuzz coverage, emitted new diagnostics, or
  mandatory AVM/dav2d execution

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

The repository SHALL provide a source-backed `splot-recon` API that serializes
canonical decoded-frame hash input bytes from a validated caller-supplied
`DecodedFrame<T>` without computing a digest. The byte stream SHALL use
identifier `av2-output-samples-v1` and raw output variant
`raw_intermediate_output`. The serializer SHALL follow AV2 § 6.16.13
sample-byte conversion for the frame's modeled visible output rows: visible
samples only, Y then U then V plane order for non-monochrome frames, Y only for
monochrome frames, raster scan order within each plane, one byte per 8-bit
sample, and little-endian two-byte values for samples with bit depth greater
than 8. The serializer SHALL exclude stride padding, backing allocation padding,
output index, frame dimensions, pixel format metadata, OBU bytes, container
metadata, and decoded-frame-hash metadata from the byte stream. The serializer
SHALL expose checked byte-length calculation and a writer-based output method,
while SHA-256 digest computation, AV2 metadata MD5 verification,
byte-consuming decode, output ordering, film-grain synthesis, Y4M output,
AVM/dav2d invocation, and new dependencies remain future work.

#### Scenario: Visible rows exclude padding

- **WHEN** a decoded frame stores non-visible padding or stride samples around a
  visible crop rectangle
- **THEN** hash input serialization writes only the visible samples in raster
  order
- **AND** padding and stride samples do not appear in the output bytes

#### Scenario: Monochrome and chroma plane order

- **WHEN** a decoded frame is monochrome
- **THEN** hash input serialization writes only Y-plane bytes
- **WHEN** a decoded frame has chroma planes
- **THEN** hash input serialization writes Y bytes, then U bytes, then V bytes

#### Scenario: Sample byte width follows bit depth

- **WHEN** a decoded frame has 8-bit output samples
- **THEN** hash input serialization writes one byte per visible sample
- **WHEN** a decoded frame has greater-than-8-bit output samples
- **THEN** hash input serialization writes each visible sample as two
  little-endian bytes

#### Scenario: Byte length matches emitted bytes

- **WHEN** a caller asks for the hash input byte length and writes the same frame
  to an in-memory byte buffer
- **THEN** the checked byte length equals the number of emitted bytes

#### Scenario: Writer errors are propagated

- **WHEN** the caller-provided writer returns an error while receiving hash
  input bytes
- **THEN** serialization returns that writer error without panicking

#### Scenario: Runtime model does not claim hash computation

- **WHEN** a reader checks the decoder roadmap and support matrix
- **THEN** the deterministic-frame-hash row states that source-backed hash input
  serialization exists
- **AND** SHA-256 digest computation, AV2 metadata MD5 verification,
  byte-consuming decode, output ordering, film-grain synthesis, Y4M output,
  AVM/dav2d invocation, and CI reference-tool requirements remain unsupported

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
SHALL treat `OBU_CLOSED_LOOP_KEY` as the only frame candidate in this slice,
and SHALL reject multistream/layer-selection structures, invalid xlayer scope,
non-base layers, unsupported frame-carrying OBUs, malformed parsed sources, and
resource-limit failures transactionally.

The planner SHALL enforce only the resource limits it can derive honestly from
the parsed stream model: `max_input_bytes` before planner traversal,
`max_obus` before adding the next planned OBU, `max_ivf_frame_records` before
traversing the next IVF frame record, and `max_frames_to_decode` before
accepting the next closed-loop-key frame candidate. Raw-byte traversal is
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
  `max_ivf_frame_records`, or accepted closed-loop-key frame candidates would
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
