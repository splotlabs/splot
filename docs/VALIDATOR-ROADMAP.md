# Validator implementation roadmap

`status: draft`  
`owner: validator`  
`scope: AV2 validator/parser/inspector, not encoder`

## Guiding rule

Every validator feature must flow through the existing five-layer workflow:

```text
OpenSpec change -> docs/IMPLEMENTATION-MATRIX.toml -> code/tests/diagnostics -> xtask proof -> generated docs/FEATURE-STATUS.md
```

Do not mark a matrix stage `done` without proof. Do not add a bare `TODO(spec)`. Use `TODO(spec: FEATURE-ID): ...` and make sure the Feature ID exists in the matrix.

## Phase 0 — matrix and OpenSpec hygiene

**Goal:** make missing validator work visible before code expands.

Tasks:

- Add this roadmap to `docs/` and link it from `docs/SPEC-MAPPING.md`, `docs/FEATURE-TRACKING.md`, and `README.md` if appropriate.
- Add or refine `openspec/changes/validator-coverage-roadmap/`.
- Expand `docs/IMPLEMENTATION-MATRIX.toml` with child rows for large features, especially sequence header and frame header.
- Regenerate `docs/FEATURE-STATUS.md`.

Acceptance:

```bash
cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md
cargo xtask check-feature-status
cargo xtask ci
```

## Phase 1 — descriptor and payload-boundary foundation

**Goal:** make `splot-core` able to parse payload syntax without panics or overreads.

Feature IDs:

- `AV2-4.11.3-UVLC`
- `AV2-4.11.5-LE`
- `AV2-4.11.8-NS`
- `AV2-5.2.3-TRAILING-BITS`
- `AV2-5.2.4-BYTE-ALIGNMENT`

Implementation shape:

```text
crates/splot-core/src/bitio.rs
  BitReader::read_bit()
  BitReader::read_bits(n)
  BitReader::read_uvlc()
  BitReader::read_ns(n)
  BitReader::read_le(n)
  BitReader::byte_align_zero()
  BitReader::remaining_bits_in(payload)

crates/splot-core/src/obu.rs
  parse_trailing_bits(nb_bits)
  parse_obu_extension_data(nb_bits)
```

Validation requirements:

- EOF returns typed `Error`, never panics.
- `trailing_bits(nbBits)` rejects `nbBits == 0` before reading `trailing_one_bit`.
- `trailing_one_bit` must be 1.
- every trailing/alignment zero bit must be 0.
- property tests cover arbitrary byte slices.

Acceptance:

```bash
cargo test -p splot-core bitio
cargo test -p splot-core trailing
cargo test -p splot-core proptests
cargo xtask ci
```

## Phase 2 — `open_bitstream_unit(sz)` payload dispatch

**Goal:** parse the OBU payload selected by `obu_type` instead of treating every payload as opaque bytes.

Feature ID:

- `AV2-5.2.1-OBU-DISPATCH`

Implementation shape:

```rust
pub enum ParsedObu<'a> {
    SequenceHeader(SequenceHeader),
    TemporalDelimiter,
    Msdo(MsdoObu),
    MultiFrameHeader(MultiFrameHeader),
    FrameHeader(FrameHeader),
    MetadataShort(MetadataShortObu<'a>),
    MetadataGroup(MetadataGroupObu),
    TileGroup(TileGroupObu<'a>),
    LayerConfigurationRecord(LayerConfigurationRecord),
    AtlasSegment(AtlasSegmentInfo),
    OperatingPointSet(OperatingPointSet),
    BufferRemovalTiming(BufferRemovalTiming),
    QuantizerMatrix(QuantizerMatrix),
    FilmGrain(FilmGrainObu),
    ContentInterpretation(ContentInterpretationObu),
    Padding(PaddingObu<'a>),
    Reserved(ReservedObu<'a>),
    Unparsed { feature: &'static str, payload: &'a [u8] },
}
```

Keep the enum `#[non_exhaustive]` and only add concrete variants when the parser exists. Until a parser exists, dispatch may return `Unparsed` plus a validator warning only if the project intentionally permits partial validation mode. Strict validation should fail on unparsed normative payloads once the feature is marked as required.

Acceptance:

- Existing envelope/header tests still pass.
- `inspect --headers` remains stable.
- `inspect --json` can include `payload_status: "opaque" | "parsed" | "unimplemented"`.
- Matrix stages are honest: dispatch can be `partial` while payload variants remain `todo`.

## Phase 3 — sequence header parser, split by §5.4 child rows

**Goal:** implement the first real OBU payload parser and unlock sequence-activated validation.

Umbrella:

- `AV2-5.4-SEQUENCE-HEADER`

Child rows to add or implement:

- `AV2-5.4.1-SEQUENCE-HEADER-GENERAL`
- `AV2-5.4.2-SEQUENCE-TILE-CONFIG`
- `AV2-5.4.3-SEQUENCE-PARTITION-CONFIG`
- `AV2-5.4.4-SEQUENCE-SEGMENT-CONFIG`
- `AV2-5.4.5-SEQUENCE-INTRA-CONFIG`
- `AV2-5.4.6-SEQUENCE-INTER-CONFIG`
- `AV2-5.4.7-SEQUENCE-SCC-CONFIG`
- `AV2-5.4.8-SEQUENCE-TQ-ENTROPY-CONFIG`
- `AV2-5.4.9-SEGMENT-INFO`
- `AV2-5.4.10-SEQUENCE-FILTER-CONFIG`
- `AV2-5.4.11-USER-QM`
- `AV2-5.4.12-TIMING-INFO`
- `AV2-5.4.13-SEQUENCE-DECODER-MODEL-INFO`
- `AV2-6.4-SEQUENCE-HEADER-SEMANTICS`

Core data shape:

```rust
pub struct SequenceHeader {
    pub seq_header_id: SequenceHeaderId,
    pub seq_profile_idc: ProfileIdc,
    pub single_picture_header_flag: bool,
    pub seq_level_idx: LevelIdx,
    pub seq_tier: Tier,
    pub chroma_format_idc: ChromaFormatIdc,
    pub bit_depth_idc: BitDepthIdc,
    pub seq_lcr_id: LcrId,
    pub still_picture: bool,
    pub max_tlayer_id: TemporalLayerId,
    pub max_mlayer_id: EmbeddedLayerId,
    pub seq_max_mlayer_count: EmbeddedLayerCount,
    pub monotonic_output_order_flag: bool,
    pub max_frame_width: NonZeroU32,
    pub max_frame_height: NonZeroU32,
    pub cropping_window: CroppingWindow,
    // Child structures follow only as implemented.
}
```

Do not add AV1 names. Every field must map directly to AV2 syntax or an AV2-derived variable.

Local checks to add first:

- `sequence-header/seq-header-id-out-of-range`
- `sequence-header/chroma-format-out-of-range`
- `sequence-header/bit-depth-out-of-range`
- `sequence-header/seq-max-mlayer-count-out-of-range`
- `sequence-header/crop-left-out-of-range`
- `sequence-header/crop-right-out-of-range`
- `sequence-header/crop-top-out-of-range`
- `sequence-header/crop-bottom-out-of-range`
- `sequence-header/timing-num-units-zero`
- `sequence-header/timing-time-scale-zero`

Acceptance:

- Positive tests for minimal still-picture and non-still sequence headers.
- Negative tests for every local range check.
- EOF tests at every field boundary that has variable width.
- Fuzz/property test: arbitrary sequence-header payload never panics.
- Matrix proof lists test modules and diagnostic IDs.

## Phase 4 — activated sequence state and remaining §6.2.2 checks

**Goal:** the validator remembers activated sequence headers and uses them to check OBU layer IDs.

Feature IDs:

- `AV2-6.2.2-OBU-HEADER-ACTIVATED-SEQUENCE-LIMITS`
- `AV2-7.3.8-HLS-AVAILABILITY`

State shape:

```rust
pub struct ValidatorContext {
    pub sequence_headers: SequenceHeaderStore,
    pub active_sequence_by_xlayer: BTreeMap<ExtendedLayerId, SequenceHeaderId>,
    pub temporal_unit: TemporalUnitState,
    pub diagnostics_mode: DiagnosticsMode,
}
```

First stateful checks:

- after activation, reject `obu_tlayer_id > max_tlayer_id`;
- after activation, reject `obu_mlayer_id > max_mlayer_id`;
- reject frame/tile OBUs before an available/activated sequence header once enough activation rules are known;
- preserve a partial-validation warning for payloads that cannot yet activate a sequence header.

Acceptance:

- Unit tests with one sequence header followed by a violating OBU.
- Unit tests with parseable prefix + later error retaining both stateful and parse diagnostics.
- No global mutable state.

## Phase 5 — OBU ordering and temporal-unit state machine

**Goal:** enforce temporal-unit and coded-extended-layer presence order enough for validator-first conformance.

Feature IDs:

- `AV2-7.3-OBU-ORDERING` umbrella, already present.
- Add child rows:
  - `AV2-7.3.2-CMVS-BOUNDARIES`
  - `AV2-7.3.3-CODED-OUTPUT-FRAME-UNIT`
  - `AV2-7.3.4-CODED-NONOUTPUT-FRAME-UNIT`
  - `AV2-7.3.5-CODED-FRAME-UNIT`
  - `AV2-7.3.6-CODED-EXTENDED-LAYER-UNIT`
  - `AV2-7.3.7-TEMPORAL-UNIT-ORDER`
  - `AV2-7.3.8-HLS-AVAILABILITY`
  - `AV2-7.3.9-LONG-TERM-REFERENCE-AVAILABILITY`

Initial checks:

- temporal unit starts with exactly one global temporal delimiter;
- global HLS OBUs precede coded extended layer units;
- coded extended layer units appear in ascending non-global `obu_xlayer_id` within a temporal unit;
- padding can appear anywhere, but outside coded extended layer units it must be global;
- global metadata prefix/suffix positions once metadata parsing exists.

Acceptance:

- Small synthetic streams for valid/invalid ordering.
- Inspector output groups OBUs by temporal unit where possible.
- Matrix child rows prevent the umbrella from pretending complete coverage.

## Phase 6 — high-level syntax OBUs

**Goal:** parse HLS OBUs referenced by sequence/frame validation and OBU ordering.

Feature IDs:

- `AV2-5.6-MSDO`
- `AV2-5.7-MULTI-FRAME-HEADER`
- `AV2-5.8-LAYER-CONFIG-RECORD`
- `AV2-5.9-ATLAS-SEGMENT`
- `AV2-5.10-OPERATING-POINT-SET`
- `AV2-5.11-OPERATING-POINT-PAYLOAD`
- `AV2-5.12-BUFFER-REMOVAL-TIMING`

Prioritize HLS availability and layer mapping:

1. MSDO: substream/xlayer map and random-access availability.
2. LCR: global/local ids, layer maps, sequence-header references.
3. OPS: maps and payload size consistency.
4. Atlas: id and LCR relationship checks.
5. Buffer removal timing: decoder-model hooks.
6. Multi-frame header: prerequisites for frame header reuse.

Acceptance:

- Each OBU has a dedicated parser module and validator checks.
- Every check has a stable diagnostic ID and spec section.
- External availability is modeled explicitly but disabled by default unless the CLI/API supplies external HLS OBUs.

## Phase 7 — non-HLS payload OBUs

**Goal:** parse and validate payload OBUs that are not the full frame/tile syntax yet.

Feature IDs:

- `AV2-5.13-QUANTIZATION-MATRIX`
- `AV2-5.14-FILM-GRAIN`
- `AV2-5.15-CONTENT-INTERPRETATION`
- `AV2-5.16-PADDING`
- `AV2-5.17-METADATA`
- metadata child rows for §5.17.1 through §5.17.13

Initial checks:

- padding payload bytes are zero where required by syntax/semantics;
- metadata type parsing and layer-specific/global rules;
- film-grain update flags and chroma idc ranges;
- quantization matrix non-zero entries and delta range once syntax exists;
- content-interpretation field bounds.

Acceptance:

- synthetic fixtures for each OBU type;
- `inspect` can show parsed fields in JSON;
- strict validation fails on unsupported payload syntax once corresponding matrix row is `partial`/`done`.

## Phase 8 — frame header child features

**Goal:** split the large frame header into implementable chunks.

Umbrella:

- `AV2-5.18-FRAME-HEADER`

Child rows:

- `AV2-5.18.1-FRAME-HEADER-GENERAL`
- `AV2-5.18.2-FRAME-HEADER-INFO`
- `AV2-5.18.3-FRAME-CONFIGURATION`
- `AV2-5.18.4-FRAME-SIZE`
- `AV2-5.18.5-FILTERING`
- `AV2-5.18.6-QUANTIZATION`
- `AV2-5.18.7-SEGMENTATION-TILING`
- `AV2-5.18.8-TRANSFORM-CODING-MODES`
- `AV2-5.18.9-GLOBAL-MOTION`
- `AV2-5.18.10-FILM-GRAIN-STRUCTURES`
- matching §6.17 semantics rows as needed.

Rules:

- Frame header implementation must depend on parsed sequence header state.
- Avoid introducing a decoder unless a check truly requires it.
- Use AVM traces/differential testing as soon as fixture streams are available.

## Phase 9 — tile group and arithmetic payload boundary validation

**Goal:** validate tile-group structure without prematurely promising a complete decoder.

Feature IDs:

- `AV2-5.19-TILE-GROUP`
- `AV2-5.20-TILE-GROUP-PAYLOAD`
- child rows for §5.20.1-§5.20.10 as needed.

Initial target:

- validate tile group header/size fields;
- validate arithmetic coder entry/exit boundaries;
- validate `exit_symbol` / trailing-bit interactions;
- leave pixel-reconstruction-dependent checks as explicit child rows.

Acceptance:

- malformed tile payloads return diagnostics, not panics;
- valid AVM-generated streams pass header/payload-boundary checks;
- incomplete decoding constraints are tracked as child rows, not hidden.

## Phase 10 — conformance vectors and AVM differential harness

**Goal:** turn validator confidence into reproducible external proof.

Feature IDs:

- `CONF-AVM-DIFF-HARNESS`
- `CONF-PUBLIC-VECTORS`
- `CONF-INSPECT-SNAPSHOTS`

Work items:

- `cargo xtask conformance --avm-bin <path> --input <stream>`;
- parser trace comparison mode;
- store failing bitstreams under an ignored local corpus and optionally add minimized redistributable fixtures;
- document where public AV2 vectors can be fetched and their license status;
- never require AVM in normal CI until the maintainer opts in.

## Done criteria for the umbrella validator goal

The validator can be called “full syntax validator” only when:

- every §5 syntax row is `parse = done` and `tests = done`;
- every locally checkable §6 semantic row is `validate = done`;
- every stateful §7.3/HLS availability row has either `validate = done` or an explicitly documented blocked dependency;
- Annex A/E conformance rows are represented and implemented to the extent their required syntax exists;
- malformed data never panics under unit tests, proptests, and fuzzing;
- AVM/public-vector proof exists for representative valid and invalid streams.
