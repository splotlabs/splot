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
own the `decode/unsupported-feature` descriptor with severity `Error`, spec
section `7.1`, matrix row `cli-decode-entrypoint`, and Feature ID `CLI-DECODE`
until a supported decoder path replaces the intentional unsupported
implementation. The `splot decode` CLI entry point SHALL render that
library-owned descriptor without changing its text or JSON field values.

#### Scenario: Unsupported feature is documented
- **WHEN** a matrix row identifies an unsupported AV2 tool
- **THEN** the row links the unsupported behavior to a stable diagnostic code or
  planned diagnostic code and a spec section where applicable

#### Scenario: Decode crate owns the unsupported diagnostic descriptor
- **WHEN** `splot-decode` is tested
- **THEN** it exposes the `decode/unsupported-feature` descriptor with severity
  `Error`, spec section `7.1`, matrix row `cli-decode-entrypoint`, and Feature
  ID `CLI-DECODE`

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
  planned `decode/resource-limit` diagnostic

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
lands, every valid `splot decode` invocation SHALL continue to emit the existing
`decode/unsupported-feature` diagnostic, exit with code `1`, avoid input reads,
avoid output writes, and avoid external decoder invocation.

#### Scenario: Compatibility Y4M form remains valid but unsupported

- **WHEN** `splot decode <input> -o <output>` is run before runtime decode
  support is implemented
- **THEN** it remains a valid CLI invocation
- **AND** it exits with code `1` and emits `decode/unsupported-feature`
- **AND** it does not read `<input>` or modify `<output>`

#### Scenario: Explicit hash format is accepted without Y4M output

- **WHEN** `splot decode <input> --output-format hash` is run before runtime
  decode support is implemented
- **THEN** it is a valid CLI invocation
- **AND** it exits with code `1` and emits `decode/unsupported-feature`
- **AND** it does not read `<input>` or create any output file

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
  runtime decode support is implemented
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

