# Encoder Research Notes and Reference Gate

**Generated:** 2026-06-06
**Scope:** `splot` encoder research policy and implementation workflow
**Feature ID:** `DOC-ENCODER-REFERENCE-GATE`
**Audience:** human contributors, AI coding agents, reviewers

> Read this file before any encoder implementation or research-driven refactor. `splot` uses
> rav1e and SVT-AV1 as excellent sources of engineering knowledge, but AV2 implementation must be
> original, spec-traceable, AVM-validated, and better tested than the references.

---

## 1. Golden rule

```text
AV2 spec + AVM = truth.
rav1e + SVT-AV1 = inspiration.
splot implementation = original Rust design.
```

Never implement decoder-visible behavior because rav1e or SVT-AV1 does it that way. Use those
projects to learn architecture, RDO search shape, rate-control organization, pipeline staging,
testing culture, and profiling practice. Then derive the AV2 implementation from:

1. AV2 Bitstream & Decoding Process Specification v1.0.0.
2. AVM reference software and differential behavior.
3. `splot`'s validator, diagnostics, and spec-mapping docs.
4. Original Rust code and original documentation.

---

## 2. Mandatory reference gate

No encoder feature should be implemented until this checklist is answered in the issue, PR, or
agent log:

```md
### Encoder research gate

- Feature / module:
- User-visible goal:
- Decoder-visible behavior? yes/no
- AV2 spec sections read:
- AVM files, tests, or streams used as oracle:
- Matrix row added/updated in `docs/IMPLEMENTATION-MATRIX.toml`? yes/no
- Reference docs read:
  - `docs/references/RAV1E-SOURCE-MAP.md`: yes/no/not relevant
  - `docs/references/SVT-AV1-RESEARCH-MAPPING.md`: yes/no/not relevant
  - `docs/references/THIRD-PARTY-NOTICES.md`: yes/no
- rav1e/SVT concepts used for inspiration:
- Third-party material copied: none
- AV1 syntax/tables/constants excluded: yes
- New strong types introduced:
- Diagnostics/tracing added:
- Tests added:
- Differential test plan:
- How this is designed to be better than the reference:
```

The expected answer for “Third-party material copied” is **none**.

---

## 3. Required reading order

For any new contributor or AI coding agent:

1. `AGENTS.md` / `CLAUDE.md` / `.github/copilot-instructions.md`.
2. `docs/SPEC-MAPPING.md`.
3. `docs/references/ENCODER-RESEARCH-NOTES.md`.
4. `docs/references/THIRD-PARTY-NOTICES.md`.
5. For Rust API, RDO, tiling, fuzzing, profiling: `docs/references/RAV1E-SOURCE-MAP.md`.
6. For production pipeline, mode decision, ME, RC, filters: `docs/references/SVT-AV1-RESEARCH-MAPPING.md`.
7. AV2 spec sections for the feature.
8. AVM source/tests for the feature.

---

## 4. What to borrow, what to reject

### Borrow aggressively

- Stage-based encoder architecture.
- Clear public API over private state.
- Strong split between configuration, sequence state, frame plan, frame state, tile state, and
  syntax writer.
- RDO search hierarchy and trial/commit split.
- Candidate generation followed by cheap pruning and full scoring.
- Deterministic speed/search policy objects.
- Thin CLI over library APIs.
- Fuzzing and differential-testing culture.
- Profiling before optimization.
- Scalar correctness before SIMD/threading.
- Debug traces that explain encoder decisions.

### Borrow cautiously

- Rate-control formulas.
- Lambda/Q tuning curves.
- AQ, variance boost, screen-content thresholds.
- Filter-search heuristics.
- Altref/overlay and temporal-structure policies.
- Partition pruning constants.
- Reference scaling/super-resolution decisions.

All of these must be retuned and validated for AV2.

### Do not borrow directly

- AV1 OBU headers or type tables.
- AV1 frame-header syntax.
- AV1 entropy CDFs, context models, token tables, or probability update rules.
- AV1 transform/filter/scan/quantization tables unless re-derived from AV2 spec/AVM.
- AV1 public names that would leak into AV2 APIs.
- SVT/rav1e source code, comments, or substantial documentation prose.

---

## 5. Combined architecture target

Use the references together:

```text
SVT-AV1 teaches:
  large production pipeline, resource/object lifetimes, process boundaries,
  feature search staging, RC/TPL/filter architecture.

rav1e teaches:
  Rust API shape, module boundaries, safe ownership, RDO implementation shape,
  tiling/plane regions, fuzzing, profiling, scalar-to-SIMD path.

AV2 spec + AVM teach:
  actual syntax, semantics, decoding process, conformance, reference behavior.

splot should become:
  validator-first Rust AV2 toolkit with a future encoder that starts small,
  safe, deterministic, testable, and traceable, then grows toward production quality.
```

---

## 6. Implementation phases

### Phase 0: validator and inspector first

- Annex-B/LEB128 envelope parser.
- AV2 OBU header parser.
- Structured diagnostics.
- Inspector output that can be diffed.
- Fuzz malformed inputs.

No encoder complexity should bypass this foundation.

### Phase 1: encoder API skeleton

- `EncoderConfig`.
- `EncoderContext`.
- `EncoderStatus`.
- `EncodedPacket`.
- State transitions and errors.
- No real syntax writing unless the bit writer is validated.

### Phase 2: frame, plane, tile, and region types

- Visible versus padded dimensions.
- Plane/region abstractions.
- Edge policies.
- Tile-local workspaces.
- Tests for non-multiple dimensions and chroma edge cases.

### Phase 3: AV2 bit writer and roundtrip parser tests

- Write only syntax already modeled in `splot-core`.
- Roundtrip with parser.
- Validate with `splot-validate`.
- Compare with AVM where possible.

### Phase 4: minimal intra-only encoder experiment

- Small test frames.
- Deterministic all-intra plan.
- Scalar prediction/transform/quant/reconstruction only after AV2 mapping.
- Trace every decision.

### Phase 5: RDO framework

- Typed `Distortion`, `EstimatedBits`, `Lambda`, `RdCost`.
- Search-policy objects.
- Candidate generators.
- Trial/commit split.
- Monotonicity and deterministic tests.

### Phase 6: reference management and inter foundation

- Reference manager.
- Frame ordering and layer-aware plans.
- Scalar SAD and full-pel ME.
- Safe motion-compensation boundaries.
- Inter only after intra is correct.

### Phase 7: filters and reconstruction correctness

- Decoder-visible filter application from AV2 spec.
- Filter parameter search only after application is correct.
- Joint filter traces and regression tests.

### Phase 8: rate control and lookahead

- RC state service.
- Pass summaries.
- Scene/content analysis.
- TPL-like experiments only after inter/reference foundation.

### Phase 9: differential testing harness

- AVM wrapper.
- Parser-trace comparison.
- Decode/reconstruction comparison where possible.
- Corpus preservation for failures.

### Phase 10: speed, threading, SIMD

- Profiling spans.
- Benchmarks.
- Search pruning.
- Optional deterministic threading.
- SIMD traits and scalar fallbacks.

---

## 7. Feature design template

Create a short design note before large features:

```md
# Feature Design: <name>

## Goal

## Non-goals

## Normative AV2 mapping

| Behavior | AV2 section | AVM file/test | `splot` module |
|---|---|---|---|

## Reference inspiration

| Reference | Files/docs read | Concepts used | Material copied |
|---|---|---|---|
| rav1e | | | none |
| SVT-AV1 | | | none |

## Proposed `splot` design

## Strong types

## Diagnostics and traces

## Tests

## Fuzz/differential plan

## Risks

## How this improves on references
```

---

## 8. Review rules

A reviewer should block a PR if:

- it writes AV2 bits without citing AV2 spec sections;
- it imports AV1 names/tables/constants without proof from AV2 spec/AVM;
- it copies source/prose from rav1e or SVT-AV1;
- it adds panics or unchecked indexing in library code;
- it adds performance complexity before scalar correctness and tests;
- it lacks diagnostics/tracing for complex decisions;
- it changes reference, layer, reconstruction, or syntax behavior without differential tests.

---

## 9. Agent instruction snippet

Add this to agent instructions:

This snippet is a compact starting point for agent configuration. Keep `AGENTS.md` as the live
canonical version and adapt wording only when the target surface needs a shorter form.

```md
## Encoder reference gate

Before changing `crates/splot-encode`, encoder-facing `splot-core` syntax/parsing code, or any
encoder research documentation, read:

1. `docs/references/ENCODER-RESEARCH-NOTES.md`
2. `docs/references/THIRD-PARTY-NOTICES.md`
3. `docs/references/RAV1E-SOURCE-MAP.md` when using Rust API, RDO, tiling, fuzzing, profiling, or
   safe data-structure ideas from rav1e
4. `docs/references/SVT-AV1-RESEARCH-MAPPING.md` when using production pipeline, mode-decision,
   motion-estimation, rate-control, filter-search, threading, or SIMD ideas from SVT-AV1

Use rav1e and SVT-AV1 only as inspiration. Do not copy AV1 syntax, source code, tables, constants,
entropy CDFs, comments, or prose. AV2 behavior must be derived from the AV2 specification and AVM.
If a feature touches syntax, reconstruction, reference state, or layer behavior, update
`docs/SPEC-MAPPING.md` before implementation.
```

---

## 10. Better-than-reference checklist

For every feature, ask:

- Is the AV2 mapping explicit?
- Is the safe scalar path tested?
- Is every boundary checked or modeled by a type?
- Can malformed input produce diagnostics instead of panics?
- Can an encoder decision be traced?
- Can the module be fuzzed independently?
- Is deterministic output preserved?
- Can AVM validate the behavior?
- Is there a simpler fallback mode?
- Does the design improve clarity, safety, or testability compared with the references?
