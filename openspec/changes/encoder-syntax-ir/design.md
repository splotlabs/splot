## Context

`splot-encode` now owns validated YUV420 frame input, a push/pull context
lifecycle, and the reconstruction dependency needed for future closed-loop
work. `splot-core` also provides a generic AV2 Section 8.2 symbol encoder that
is inverse to the existing symbol decoder. What is still missing is an encoder
side representation of syntax decisions before those decisions are serialized.

This change introduces a private, deterministic syntax-planning IR in
`splot-encode`. The IR is not an AV2 header writer, tile writer, transform
implementation, quantizer, or entropy model. It is a typed staging model for
future sequence/frame/tile/block/token decisions, with enough validation to
prove that planned decisions can be ordered and inspected without mutating an
output writer.

Reference gate:

- `docs/references/ENCODER-RESEARCH-NOTES.md` reviewed for encoder-scope
  boundaries.
- `docs/references/THIRD-PARTY-NOTICES.md` reviewed for mixed-license
  constraints.
- `docs/references/RAV1E-SOURCE-MAP.md` reviewed because this touches encoder
  planning data structures.
- `docs/references/SVT-AV1-RESEARCH-MAPPING.md` is not used for this change;
  no production pipeline, mode-decision search, motion-estimation,
  rate-control, threading, or SIMD design is being adopted.
- AV2 syntax semantics remain grounded in the committed spec mirror when future
  serialization work lands. This change does not copy AV2 tables, constants,
  entropy CDFs, or normative prose.

## Goals

- Add `ENC-SYNTAX-IR` to the implementation matrix and keep the work attached
  to that Feature ID.
- Add a private `splot-encode` module for deterministic syntax planning.
- Model the requested planning layers: `SequencePlan`, `FramePlan`, `TilePlan`,
  `SuperBlockPlan`, `BlockDecision`, `PredictionDecision`,
  `TransformDecision`, `QuantizedCoefficients`, and ordered syntax/token
  events.
- Use typed indices/newtypes and bounded constructors instead of bare integer
  plumbing at planning boundaries.
- Preserve deterministic iteration/debug rendering independent of thread count
  and unordered map iteration.
- Prove invalid plans fail before observable mutation through focused tests.

## Non-Goals

- Do not emit AV2 headers, tile bytes, OBUs, Annex B data, IVF data, or
  `Packet` payloads.
- Do not add a public `splot encode` success path.
- Do not implement transform kernels, quantization policy, mode decision,
  reference-frame selection, CDF selection, or rate control.
- Do not change the crate dependency graph or add third-party dependencies.
- Do not hand-edit the AV2 spec mirror.

## Decisions

### Keep the IR private

The syntax IR will live behind a private `splot-encode` module and will not be
re-exported from the crate root. That keeps this change free to evolve while
future encoder passes discover the exact AV2 state they need. Any constructor
or inspection helper added for tests remains planning-only and must not imply
that a stream can be produced.

### Separate planning from serialization

The IR stores typed decisions and an ordered event stream. It does not own a
bit writer, `SymbolEncoder`, byte sink, or `Packet`. Serialization will be a
later `ENC-BITSTREAM-WRITER` follow-up that consumes a validated plan.

### Use deterministic collections and explicit order keys

Plans use `Vec`-backed ordered children and explicit index newtypes. Builders
validate monotonic order where a later writer would rely on that order, and
tests compare debug rendering across repeated construction. No hash-map
iteration is part of plan rendering.

### Keep syntax labels non-normative

`PredictionDecision`, `TransformDecision`, and `QuantizedCoefficients` are
encoder planning records, not spec-complete AV2 syntax models. Names and values
must avoid claiming support for AV2 modes, tables, or coefficient semantics
that are not implemented. When future serialization maps a planning field to
normative syntax, that follow-up must cite the AV2 spec mirror and update the
matrix proof.

## Risks And Tradeoffs

- A private IR can churn. That is acceptable here because the first need is a
  stable internal staging boundary, not a public API.
- Over-modeling future AV2 syntax would create false confidence. This design
  keeps fields minimal and non-emitting until exact spec mappings are
  implemented.
- Adding validation to private constructors costs a little boilerplate but gives
  tests a concrete failure-before-mutation contract and prevents later writers
  from inheriting malformed plans.

## Flight Manifest

- Change ID: `encoder-syntax-ir`
- Feature IDs: `ENC-SYNTAX-IR`
- Base commit: `6a9184a3b97daf746742c48553e8ffde7c75e386`
- Depends on merged work: `DOC-ENCODER-PROGRAM-CONTRACT`, `ENC-RECON-DEPENDENCY`,
  `ENC-Y4M-INPUT`, `ENC-CONTEXT-STATE-MACHINE`, and the Section 8.2 symbol
  encoder work in `ENC-BITSTREAM-WRITER`
- Owned source files/directories:
  - `crates/splot-encode/src/lib.rs`
  - `crates/splot-encode/src/syntax_ir.rs`
  - `crates/splot-encode/src/error.rs` only if a public encoder error variant
    becomes necessary
- Owned docs/tracking files:
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `openspec/changes/encoder-syntax-ir/**`
- Forbidden source files/directories:
  - `crates/splot-core/**`
  - `crates/splot-recon/**`
  - `crates/splot-decode/**`
  - `crates/splot-validate/**`
  - `crates/splot-cli/**`
  - `docs/spec/av2/**`
  - Cargo manifests and lockfiles
- Public API/types owned: none
- Matrix rows owned: `ENC-SYNTAX-IR`
- Generated outputs expected: `docs/FEATURE-STATUS.md` and
  `docs/SPEC-COVERAGE.md`
- Open sibling PRs audited: none
- Merge strategy: may merge directly after local CI, GitHub CI, Claude review,
  and explicit Codex acceptance on the final reviewed HEAD
