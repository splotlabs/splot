# SVT-AV1 Research Mapping for `splot` AV2 Encoder Development

**Generated:** 2026-06-06
**Scope:** original `splot` research summary
**Primary upstream:** AOMediaCodec/SVT-AV1
**Use level:** inspiration, production encoder architecture, stage scheduling, heuristics, profiling culture
**Do not use level:** copied AV1 syntax, tables, constants, entropy CDFs, comments, prose, or source code

> SVT-AV1 is useful because it is a production-oriented AV1 encoder with extensive design notes.
> Use it to understand encoder pipeline structure, resource lifetimes, mode-decision staging,
> motion-estimation organization, rate control, filter search, and speed/quality policies.
>
> **It is not a normative AV2 source. AV2 syntax and decoder-visible behavior come from the AV2
> specification and AVM.**

---

## 1. Required research gate

Before implementing a feature inspired by SVT-AV1, record:
This gate supplements the canonical full gate in `docs/references/ENCODER-RESEARCH-NOTES.md` § 2.

```md
### SVT-AV1 reference gate

- Feature being implemented:
- AV2 spec sections read:
- AVM source/tests used as oracle:
- Decoder-visible behavior? yes/no
- `docs/SPEC-MAPPING.md` entries updated? yes/no
- SVT documents/source areas consulted:
- Concepts reused:
- Material copied from SVT-AV1: none
- AV1 syntax/tables/constants checked and excluded: yes
- Bitstream impact:
- Layer/reference/reconstruction impact:
- Tests/traces added:
- How this design aims to be better than the reference:
```

The expected answer for “Material copied from SVT-AV1” is **none**. If it is not none, stop and do
a license/legal review before continuing.

---

## 2. Source inventory

| Source | Link | Use in `splot` |
|---|---|---|
| Repository root | <https://gitlab.com/AOMediaCodec/SVT-AV1> | Root layout, releases, project metadata. |
| README | <https://gitlab.com/AOMediaCodec/SVT-AV1/-/raw/master/README.md> | Top-level docs and project overview. |
| Encoder design | <https://gitlab.com/AOMediaCodec/SVT-AV1/-/raw/master/Docs/svt-av1-encoder-design.md> | Best starting point for pipeline, processes, resource managers. |
| Docs tree | <https://gitlab.com/api/v4/projects/AOMediaCodec%2FSVT-AV1/repository/tree?ref=master&path=Docs&per_page=100> | Current appendix names and availability. |
| `Source/App` tree | <https://gitlab.com/api/v4/projects/AOMediaCodec%2FSVT-AV1/repository/tree?ref=master&path=Source/App&per_page=100> | CLI/application split. |
| `Source/Lib` tree | <https://gitlab.com/api/v4/projects/AOMediaCodec%2FSVT-AV1/repository/tree?ref=master&path=Source/Lib&per_page=100> | Codec library and SIMD/C layout. |
| `Source/Lib/Codec` tree | <https://gitlab.com/api/v4/projects/AOMediaCodec%2FSVT-AV1/repository/tree?ref=master&path=Source/Lib/Codec&per_page=100> | Core encoder source-map areas. |
| License | <https://gitlab.com/AOMediaCodec/SVT-AV1/-/raw/master/LICENSE.md> | BSD 3-Clause Clear license notice if vendoring ever happens. |
| Patent notice | <https://gitlab.com/AOMediaCodec/SVT-AV1/-/raw/master/PATENTS.md> | AOM patent-license context; not AV2 legal advice. |

---

## 3. What SVT-AV1 teaches `splot`

SVT-AV1 is the **large production encoder architecture model**. It helps answer:

- How do production encoders split work into stages?
- How are sequence-level, picture-level, reference, and result objects managed?
- Where should picture analysis, prediction-structure planning, motion estimation, rate control,
  mode decision, filtering, entropy coding, and packetization sit?
- How should speed presets control search budgets?
- How can expensive features be staged behind cheap analysis and pruning?
- How can threading, object pools, and SIMD be introduced without destroying maintainability?

Use SVT-AV1 to design the future `splot-encode` architecture, but implement a small deterministic
scalar path first.

---

## 4. Pipeline mapping

The transferable SVT pattern is a staged pipeline with explicit objects and queues. A future AV2
encoder can grow toward this shape:

```text
input frame
  -> resource coordination
  -> picture analysis
  -> picture decision / prediction structure
  -> open-loop motion estimation and source analysis
  -> initial rate control
  -> picture manager / reference scheduling
  -> rate control
  -> mode decision config
  -> mode decision and encode pass
  -> in-loop filter parameter search and application
  -> entropy coding
  -> AV2 OBU packetization
  -> output packet
```

### `splot` module mapping

| SVT concept | Meaning | Future `splot` module |
|---|---|---|
| Resource coordination | Allocate input/picture objects and route work | `splot-encode::pipeline`, `resources` |
| Sequence Control Set | Sequence-level configuration/state | `splot-core::sequence`, `splot-encode::sequence_state` |
| Picture Control Set | Per-picture state and decisions | `splot-encode::frame_plan`, `frame_state` |
| Picture analysis | Low-cost metrics, variance, scene/content stats | `splot-encode::analysis` |
| Picture decision | Temporal structure and reference plan | `splot-encode::prediction_plan` |
| Motion estimation process | Open-loop ME candidate generation | `splot-encode::motion` |
| Source-based operations / TPL | Temporal dependency analysis | `splot-encode::lookahead`, `tpl` |
| Initial/final rate control | Bit allocation, Q index/lambda decisions | `splot-encode::rate_control` |
| Picture manager | Reference availability and lifecycle | `splot-encode::reference` |
| Mode decision config | Search budgets and speed policy | `splot-encode::search_policy` |
| Mode decision | Partition/mode/transform RDO | `splot-encode::rdo`, `partition`, `mode_decision` |
| Deblocking/CDEF/restoration | Filter search/apply | `splot-encode::filters` |
| Entropy coding | Syntax writer and coder | `splot-encode::entropy` |
| Packetization | OBUs and Annex-B envelope | `splot-core::obu`, `splot-encode::packetize` |

---

## 5. Appendix/source map

| Topic | SVT document to study | `splot` use |
|---|---|---|
| Encoder architecture | `Docs/svt-av1-encoder-design.md` | Pipeline, objects, threading model, stage boundaries. |
| Dynamic prediction structure | `Appendix-Dynamic-Mini-GoP.md`, prediction-structure source | Temporal planning ideas. AV2 layer semantics still spec-derived. |
| Altref / overlay | `Appendix-Alt-Refs.md` | Reference/filtering strategy inspiration only. |
| Mode decision | `Appendix-Mode-Decision.md` | Candidate staging, pruning, trial/commit split. |
| Motion estimation | `Appendix-Open-Loop-Motion-Estimation.md` | HME/full-pel/subpel search organization. |
| Compound prediction | `Appendix-Compound-Mode-Prediction.md` | Candidate family organization; AV2 modes from spec. |
| Global/local warped motion | `Appendix-Global-Motion.md`, `Appendix-Local-Warped-Motion.md` | Parameter search design; AV2 ranges/processes from spec/AVM. |
| OBMC | `Appendix-Overlapped-Block-Motion-Compensation.md` | Search/apply staging if AV2 supports analogous tools. |
| Intra block copy | `Appendix-Intra-Block-Copy.md` | Screen-content architecture; AV2 syntax from spec. |
| Palette prediction | `Appendix-Palette-Prediction.md` | Palette candidate/search structure. |
| Recursive intra | `Appendix-Recursive-Intra.md` | Recursive search strategy for intra. |
| CfL | `Appendix-CfL.md` | Chroma prediction idea; AV2 semantics must be verified. |
| Deblocking | `Appendix-DLF.md` | Filter architecture and search staging. |
| CDEF | `Appendix-CDEF.md` | Directional filter search and speed policy. |
| Restoration | `Appendix-Restoration-Filter.md` | Filter parameter search architecture. |
| Film grain | `Appendix-Film-Grain-Synthesis.md` | Analysis/synthesis workflow; AV2 film grain OBU semantics from spec. |
| Rate control | `Appendix-Rate-Control.md` | RC service architecture, pass summaries, lambda/Q policy. |
| TPL | `Appendix-TPL.md` | Temporal dependency modeling idea. |
| Transform search | `Appendix-TX-Search.md` | Search staging; AV2 transform set from spec/AVM. |
| Reference scaling / super-res | `Appendix-Reference-Scaling.md`, `Appendix-Super-Resolution.md` | Scaling architecture; AV2 syntax/processes only. |
| Variance boost / SQ weight | `Appendix-Variance-Boost.md`, `Appendix-SQ-Weight.md` | AQ/perceptual weighting ideas; retune for AV2. |
| Screen content detection | `Appendix-Antialiasing-Aware-Screen-Content-Detection-Mode.md` | Detection architecture; do not copy thresholds. |
| Profiling | `Appendix-Profiling.md` | Profiling instrumentation culture. |

---

## 6. Feature research capsules

### 6.1 Prediction structure and reference planning

Borrow:

- explicit prediction-plan object;
- separation of coding order from display order;
- deterministic reference availability checks;
- small search policies before complex adaptive GOP logic.

For AV2:

- model `TemporalLayerId`, `EmbeddedLayerId`, and `ExtendedLayerId` from the AV2 OBU header;
- derive OBU ordering and layer constraints from AV2 spec and AVM;
- start with all-intra or single-layer deterministic plans;
- add temporal structures only after validator and traces exist.

Do not copy AV1 reference-slot names or AV1 refresh rules.

### 6.2 Motion estimation

Borrow:

- staged search: cheap candidates -> full-pel -> subpel -> mode-decision integration;
- deterministic tie-breaking;
- separated scalar kernels before SIMD;
- source-analysis ME separate from final reconstruction.

For AV2:

- define AV2 `MotionVector`, precision, clamp, and reference rules from spec/AVM;
- implement safe scalar SAD/SATD first;
- fuzz edge clipping and bounds.

### 6.3 Mode decision and RDO

Borrow:

- staged candidate classes;
- speed-policy budgets;
- trial encode versus commit pass;
- traceable reasons for candidate pruning.

For AV2:

- partition, mode, transform, and coefficient syntax must be AV2-specific;
- rate estimates must be based on AV2 entropy syntax, not AV1 tables;
- exact scalar RDO path comes before approximate pruning.

### 6.4 Rate control and TPL

Borrow:

- RC as a service with state and pass summaries;
- lambda/Q decisions separated from syntax writing;
- temporal dependency modeling as a future feature.

For AV2:

- decoder model, level/tier/profile constraints, and timing rules must be mapped from AV2;
- do not copy SVT constants or tuning curves;
- build traceable RC decisions before optimizing.

### 6.5 Filters and restoration

Borrow:

- joint thinking about filter parameter search;
- filter workspaces separated from syntax writer;
- speed policies for filter search depth.

For AV2:

- deblocking, CDEF-like tools, restoration, CCSO/GDF or any AV2-specific filters must be verified
  from AV2 spec and AVM;
- implement decoder-visible filter application exactly before doing encoder-side parameter search.

### 6.6 Screen content and palette-like tools

Borrow:

- analysis-only screen-content metrics;
- detection before enabling syntax behavior;
- synthetic tests for flat/text/noise patterns.

For AV2:

- AV2 screen-content tools and palette/IBC syntax must be mapped first;
- thresholds must be retuned and benchmarked;
- keep detector outputs as advisory statistics until syntax is implemented.

### 6.7 Threading and SIMD

Borrow:

- stage-level and tile/block-level parallelism concepts;
- architecture-specific kernel boundaries;
- profiling before optimization.

For `splot`:

- scalar safe Rust first;
- deterministic tests before threads;
- SIMD only behind narrow modules with scalar fallback and exact tests.

---

## 7. AV2 guardrails

Every SVT-inspired feature must obey these rules:

1. AV2 spec and AVM define the syntax and decoder-visible behavior.
2. SVT-AV1 may explain **why** a production encoder is organized a certain way, not **what bits** to write.
3. Do not copy AV1 OBU headers, AV1 OBU type tables, AV1 frame-header fields, AV1 reference semantics,
   AV1 transform/filter tables, AV1 entropy CDFs, or AV1 constants.
4. Do not copy source code, comments, or substantial prose.
5. Add `TODO(spec: <FEATURE-ID>): section/topic` instead of inventing missing AV2 details.
6. Keep validator and bitstream inspector ahead of encoder complexity.
7. Add tracing so developers can see why a decision was made.
8. Treat SVT as a baseline to surpass, not a target to clone.

---

## 8. Better-than-reference goals

`splot` should aim to be better than the references in these ways:

- **Spec traceability:** every syntax element links to AV2 section and tests.
- **Safe boundaries:** scalar Rust path avoids pointer arithmetic and global mutable state.
- **Diagnostics-first:** validator diagnostics and encoder traces are first-class outputs.
- **Determinism:** same input/config produces reproducible plans and packets.
- **Small public API:** stable types and strong names, not C-era global structures.
- **Fuzzability:** small modules can be fuzzed independently.
- **Readable research memory:** design choices stay documented in `docs/references/`.
- **Performance after correctness:** SIMD/threading land only after scalar differential tests.

---

## 9. Suggested follow-up tasks

### Task: encoder architecture doc

Create `docs/ENCODER-ARCHITECTURE.md` that combines:

- SVT stage pipeline;
- rav1e Rust API/module lessons;
- AV2 spec and AVM guardrails;
- first milestones: validator -> bit writer -> intra-only encoder -> RDO -> inter.

### Task: prediction planner skeleton

- Define AV2-aware picture/layer plan types.
- Support only one embedded/extended layer initially.
- Add deterministic tests.
- Do not write bitstream syntax yet.

### Task: scalar SAD and full-pel ME baseline

- Implement safe scalar SAD.
- Implement exhaustive full-pel search over clipped areas.
- Add tie-break and edge tests.
- No SIMD or unsafe.

### Task: encoder trace schema

- Add JSON structs for prediction plan, ME results, MD candidates, RC decisions, and OBU packet summary.
- Keep disabled by default.
- Use AV2 terms.

---

## 10. PR checklist

Every PR inspired by SVT-AV1 must answer:

- Which AV2 sections define the syntax and semantics?
- Which AVM behavior validates this?
- Which SVT documents/source areas were read?
- What concept was borrowed?
- What was explicitly **not** copied?
- Does the change affect OBUs, layer IDs, reference state, or reconstruction?
- Are there positive, negative, edge, and differential tests?
- Is tracing available?
- Is there a simple fallback if the feature is disabled?
- Is the scalar path correct before performance work?
