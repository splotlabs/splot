# Design: validator coverage roadmap

`change-id: validator-coverage-roadmap`  
`status: proposed`

## 1. Architecture overview

The validator should evolve from a stateless per-OBU checker into a parser-driven, stateful validator while preserving the current crate boundaries:

```text
splot-core
  strong AV2 types
  panic-free bit readers and syntax parsers
  no dependency on splot-validate

splot-validate
  diagnostic model
  stateless checks for individual OBUs
  stateful checks for activated sequence headers, HLS availability, OBU ordering

splot-cli
  thin file/JSON/human interface only

xtask
  matrix drift checks, feature-status, spec-coverage, conformance harness stubs
```

## 2. Parser layering

Recommended `splot-core` layering:

```text
BitReader + descriptors
  f(n), uvlc(), ns(n), le(n), alignment

Annex B envelope
  leb128 length -> declared OBU bytes

OBU header
  §5.2.2 AV2 header only

Open bitstream unit dispatch
  header + payload size + payload-specific parser

Payload syntax parsers
  sequence, HLS, metadata, frame, tile group

Parsed syntax tree
  compact, non-owning where possible, owned where state must survive validation
```

The parser should return typed `Error` for structural problems. `splot-validate` converts parser errors into diagnostics.

## 3. `ParsedObu` and payload status

The repository currently treats payload bytes as opaque. Add an explicit payload status before deep parsing:

```rust
pub enum PayloadStatus<'a, T> {
    Parsed(T),
    Opaque(&'a [u8]),
    Unimplemented { feature: &'static str, payload: &'a [u8] },
}
```

For the public API, prefer a typed enum:

```rust
#[non_exhaustive]
pub enum ParsedObu<'a> {
    SequenceHeader(SequenceHeader),
    TemporalDelimiter,
    Padding(PaddingObu<'a>),
    Unimplemented { feature: &'static str, payload: &'a [u8] },
}
```

Add variants only when the parser exists. Avoid public fields with raw integers where the spec has bounded syntax.

## 4. Stateful validator context

A stateful validator is required for checks that depend on previous OBUs.

```rust
pub struct ValidatorContext {
    pub hls: HighLevelSyntaxState,
    pub temporal_unit: TemporalUnitState,
    pub active_sequences: ActiveSequenceState,
    pub mode: ValidationMode,
}
```

Suggested substate:

```rust
pub struct HighLevelSyntaxState {
    pub sequence_headers: SequenceHeaderStore,
    pub msdo: Option<MsdoState>,
    pub global_lcr: BTreeMap<LcrId, LayerConfigurationRecord>,
    pub local_lcr: BTreeMap<(ExtendedLayerId, LcrId), LayerConfigurationRecord>,
    pub atlas: BTreeMap<AtlasSegmentId, AtlasSegmentInfo>,
    pub ops: BTreeMap<OpsId, OperatingPointSet>,
}

pub struct ActiveSequenceState {
    pub by_xlayer: BTreeMap<ExtendedLayerId, SequenceHeaderId>,
}

pub struct TemporalUnitState {
    pub index: u64,
    pub seen_temporal_delimiter: bool,
    pub phase: TemporalUnitPhase,
    pub last_non_global_xlayer: Option<ExtendedLayerId>,
}
```

Do not add all fields at once. Add only the fields needed by the feature being implemented.

## 5. Partial validation mode

The project is validator-first but not yet complete. Partial coverage must be explicit.

Recommended modes:

```rust
pub enum ValidationMode {
    LenientPartial,
    StrictPartial,
    CompleteRequired,
}
```

Initial behavior can preserve the current CLI `--strict` flag:

- default mode: validate implemented syntax and warn about unparsed normative payloads;
- strict mode: treat warnings as unacceptable;
- future complete mode: reject unparsed syntax as an error once the matrix says a feature should be parsed.

## 6. Diagnostics and offsets

Every parser should preserve enough position data to locate syntax errors.

Recommended span types:

```rust
pub struct BitSpan {
    pub byte_offset: ByteOffset,
    pub bit_offset: BitOffset,
    pub bit_len: u64,
}
```

Do not require every parsed syntax element to carry spans at first. Start with spans for parser errors and diagnostics. Add per-field spans for tricky payload syntax as needed.

## 7. Sequence header data model

The sequence header parser should be the first deep parser. Fields must map directly to AV2 names or explicitly derived AV2 variables.

Recommended module split:

```text
crates/splot-core/src/headers/
  mod.rs
  sequence.rs
  frame.rs
  hls.rs
  metadata.rs
```

The existing `headers.rs` can become a compatibility re-export if module splitting is too large for one PR.

Strong types to add:

```text
SequenceHeaderId
ProfileIdc
LevelIdx
Tier
ChromaFormatIdc
BitDepthIdc
LcrId
FrameDimensionBits
FrameWidth
FrameHeight
CroppingWindow
EmbeddedLayerCount
```

## 8. Implementation order

The first coding PR should implement:

1. descriptor support required by sequence header;
2. `trailing_bits()` / `byte_alignment()`;
3. payload dispatch skeleton;
4. `SequenceHeader` fields through §5.4.1;
5. local §6.4.1 diagnostics;
6. activated sequence max layer checks.

Later PRs implement child rows.

## 9. Testing strategy

Test layers:

```text
unit tests: exact parser behavior and exact diagnostics
property tests: arbitrary bytes never panic
CLI tests: JSON/human diagnostics and exit codes
snapshot tests: inspect output once stable
fixtures: tiny hand-built valid/invalid OBUs
AVM: optional differential oracle
```

Fixtures should be small and generated by helper builders in tests where possible. Do not vendor unclear-license bitstreams.

## 10. AV1 guardrails

rav1e and SVT-AV1 may be used for software-architecture inspiration only. Do not copy:

- AV1 OBU header fields;
- AV1 OBU type tables;
- AV1 entropy contexts/CDFs;
- AV1 syntax writers;
- code, comments, or tables.

For AV2 syntax and semantics, use only:

1. AV2 v1.0.0 specification;
2. AVM behavior/differential traces;
3. original Rust implementation work in `splot`.
