# rav1e Source Map for `splot` AV2 Encoder Research

**Generated:** 2026-06-06
**Scope:** original `splot` research summary
**Primary upstream:** Xiph.Org `xiph/rav1e`
**Use level:** inspiration, source navigation, Rust architecture patterns
**Do not use level:** copied AV1 syntax, tables, constants, entropy CDFs, comments, or source code

> `rav1e` is an AV1 encoder. It is valuable to `splot` because it is a mature Rust codec project,
> not because its AV1 bitstream logic is reusable for AV2. Treat it as a map of Rust API design,
> ownership boundaries, RDO organization, tests, fuzzing, profiling, and SIMD layering.
>
> **Normative AV2 behavior must come from the AV2 specification and AVM, never from rav1e.**

---

## 1. Required research gate

Before using this document for an implementation task, add a note to the issue, PR, or agent log:
This gate supplements the canonical full gate in `docs/references/ENCODER-RESEARCH-NOTES.md` § 2.

```md
### rav1e reference gate

- Feature being implemented:
- AV2 spec sections read:
- AVM source/tests used as oracle:
- Decoder-visible behavior? yes/no
- `docs/SPEC-MAPPING.md` entries updated? yes/no
- rav1e modules/docs consulted:
- Concepts reused:
- Material copied from rav1e: none
- AV1 syntax/tables/constants checked and excluded: yes
- Tests/traces added:
- How this design aims to be better than the reference:
```

The expected answer for “Material copied from rav1e” is **none**. If the answer is anything else,
stop and do a license/legal review before continuing.

---

## 2. Source inventory

| Source | Link | Use in `splot` |
|---|---|---|
| Repository root | <https://github.com/xiph/rav1e> | Root layout, active development style, issues, releases. |
| README | <https://raw.githubusercontent.com/xiph/rav1e/master/README.md> | High-level feature and build overview. |
| `doc/STRUCTURE.md` | <https://raw.githubusercontent.com/xiph/rav1e/master/doc/STRUCTURE.md> | Best source-map style document for navigating code. |
| `doc/RDO.md` | <https://raw.githubusercontent.com/xiph/rav1e/master/doc/RDO.md> | Compact RDO/search hierarchy explanation. |
| `doc/FRAME_TYPE_SELECTION.md` | <https://raw.githubusercontent.com/xiph/rav1e/master/doc/FRAME_TYPE_SELECTION.md> | Lookahead, scenecut, frame-type planning ideas. |
| `doc/QUALITY_&_SPEED_FEATURES.md` | <https://raw.githubusercontent.com/xiph/rav1e/master/doc/QUALITY_%26_SPEED_FEATURES.md> | Speed/quality feature inventory and pruning concepts. |
| `doc/PROFILING.md` | <https://raw.githubusercontent.com/xiph/rav1e/master/doc/PROFILING.md> | Profiling workflow ideas. |
| `doc/TILE_ON_FRAME_BOUNDARY.md` | <https://raw.githubusercontent.com/xiph/rav1e/master/doc/TILE_ON_FRAME_BOUNDARY.md> | Tile/plane-region edge handling lessons. |
| Public API docs | <https://docs.rs/rav1e/latest/rav1e/> | Rust public API shape: config, context, frame, packet, status. |
| License | <https://github.com/xiph/rav1e/blob/master/LICENSE> | BSD-2-Clause license notice if anything is ever vendored. |
| Patent notice | <https://github.com/xiph/rav1e/blob/master/PATENTS> | AOM patent-license context; not a substitute for AV2 legal review. |

---

## 3. What rav1e teaches `splot`

rav1e is most useful as the **Rust-native small/medium encoder model**. It helps answer:

- How should a Rust encoder expose configuration, context, frames, packets, and statuses?
- How can encoder state be split between sequence-level invariants, frame-level invariants,
  mutable frame state, tile state, and reference state?
- How should RDO be organized so cheap decisions prune expensive trial encodes?
- How should frame and tile region types avoid out-of-bounds edge bugs?
- How should fuzz targets, tests, debug dumps, profiling, and feature gates grow over time?
- How can future SIMD be isolated behind scalar fallbacks and dispatch layers?

rav1e is **not** useful for AV2 syntax truth. Its OBU writing, frame headers, entropy tables,
CDF contexts, transform tables, frame update semantics, and tool flags are AV1-oriented.

---

## 4. Repository and source layout lessons

rav1e's root layout contains documentation, fuzzing, benches, examples, tests, tools, a public
Rust library, CLI code, and CPU-specific implementation areas. For `splot`, keep the existing
workspace split because `splot` is broader than an encoder:

```text
crates/splot-core       # AV2 bitstream model, types, parsers, future shared codec primitives
crates/splot-validate   # validator diagnostics and conformance checks
crates/splot-encode     # future AV2 encoder API and implementation
crates/splot-cli        # thin CLI wrapper only
xtask                   # automation
fuzz                    # fuzz targets
```

Borrow rav1e's culture of close-to-code docs, fuzz targets, benchmarks, profiling notes, and
simple public API entry points. Do not collapse `splot` into one monolithic crate just because
rav1e is structured that way.

---

## 5. Module map: rav1e to future `splot`

| rav1e area | Conceptual role | `splot` analogue | Copy risk |
|---|---|---|---|
| `src/api/*` | Public config/context/frame/packet API | `splot-encode::{config, context, packet}` | Low if pattern-only. Do not copy names blindly. |
| `src/bin/*` | CLI, muxing, stats, y4m I/O | `splot-cli` | Low. Keep CLI thin. |
| `src/encoder.rs` | Central encode orchestration | `splot-encode::encoder` or smaller modules | Medium. Split early; avoid giant file. |
| `src/frame/*` | Frame and plane storage | `splot-core::frame` or `splot-encode::frame` | Low for concepts; code not copied. |
| `src/tiling/*` | Tile state, regions, tile blocks | `splot-encode::tiling` | Low for concepts. AV2 tiling must be spec-derived. |
| `src/context/*` | Syntax/context writers, block/partition/transform units | `splot-encode::syntax`, `entropy`, `partition` | High. AV2 context/syntax must be original. |
| `src/rdo.rs` | RD search and decision machinery | `splot-encode::rdo` | Medium. Use formula and structure only. |
| `src/rdo_tables.rs` | AV1 RDO rate tables | None | High. Do not copy. |
| `src/me.rs` | Motion search | `splot-encode::motion` | Medium. Search patterns only; syntax and MV rules from AV2. |
| `src/mc.rs` | Motion compensation kernels | `splot-encode::motion_comp` | High. Kernels must be AV2/AVM-derived. |
| `src/predict.rs` | Prediction candidate tools | `splot-encode::predict` | High. AV2 tool set only. |
| `src/partition.rs` | Partition traversal helpers | `splot-encode::partition` | Medium. Use recursive-search idea only. |
| `src/transform/*` | Transform code | `splot-encode::transform` | High. AV2 transform tables/processes only. |
| `src/quantize/*` | Quant/dequant | `splot-encode::quant` | High. AV2 quantization only. |
| `src/dist.rs` | Distortion metrics | `splot-encode::distortion` | Low/medium. Write fresh code/tests. |
| `src/rate.rs` | Rate-control state | `splot-encode::rate_control` | Medium. Model state shape, not formulas blindly. |
| `src/deblock.rs`, `src/cdef.rs`, `src/lrf.rs` | In-loop filtering | `splot-encode::filters` | High. AV2 syntax/processes only. |
| `src/ec.rs`, `src/entropymode.rs`, `src/token_cdfs.rs` | AV1 entropy coder/CDFs | `splot-encode::entropy` later | Very high. Do not copy AV1 tables/CDFs. |
| `src/x86`, `src/arm`, `src/asm`, `src/cpu_features` | SIMD/dispatch | Future `splot-kernels` or internal backends | Medium. Delay until scalar correctness. |
| `fuzz/`, `src/fuzzing.rs` | Fuzzing hooks | `fuzz/` and parser/encoder fuzz targets | Low. Use the practice, not code. |

---

## 6. Public API pattern to borrow

rav1e's broad API pattern is worth adapting:

```text
EncoderConfig -> EncoderContext -> send_frame -> receive_packet/status
```

For AV2, make it explicit that the first implementation may be a stub or intra-only:

```rust
// Conceptual shape only. Do not copy rav1e API directly.
pub struct EncoderConfig { /* AV2-specific, validated fields */ }
pub struct EncoderContext { /* private state */ }
pub enum EncoderStatus { NeedMoreData, PacketReady, LimitReached, Finished }
pub struct EncodedPacket { /* AV2 OBU/Annex-B payload metadata */ }
```

Design goals:

- API is library-first; CLI just calls the library.
- Configuration validation returns typed errors, never panics.
- Syntax-writing APIs use AV2 terms from the spec.
- Stubbed features return structured `Unimplemented` errors.
- Public API does not expose AV1 names such as AV1 frame update slots or AV1-only tool flags.

---

## 7. Encoder state model to adapt

rav1e's conceptual state split is one of its best lessons. For `splot`, use an AV2-specific version:

```text
EncoderConfig
  -> SequenceState
      -> FramePlan / FrameInvariants
          -> FrameState
              -> TileState
                  -> PartitionSearchState
```

Recommended AV2 state objects:

| `splot` object | Responsibility |
|---|---|
| `EncoderConfig` | User configuration, speed policy, tune, deterministic settings. |
| `Av2SequenceState` | AV2 sequence/layer state derived from spec. |
| `FramePlan` | Stable frame-level plan: order, frame role, layer IDs, references, qindex/lambda, tile plan. |
| `FrameState` | Mutable per-frame buffers: input, recon, entropy state, filter workspaces, stats. |
| `ReferenceManager` | Reconstructed frame store and AV2 reference lifecycle. |
| `TileState` | Tile-local buffers, contexts, motion/filter stats. |
| `PlaneRegion` | Bounds-checked access to visible and padded image regions. |
| `EncoderTrace` | JSON/debug trace of decisions, costs, and emitted OBUs. |

Keep immutable planning data separate from mutable workspaces. This makes it easier to fuzz, test,
trace, and parallelize later.

---

## 8. RDO ideas to borrow

The transferable rav1e RDO idea is the search pipeline, not AV1 rate tables.

General objective:

```text
RD score = distortion + estimated_bits * lambda
```

Recommended `splot` shape:

```text
candidate generator
  -> cheap feature/rate estimate
  -> SATD or residual prefilter
  -> trial transform/quant/reconstruct/rate estimate
  -> full RD score
  -> commit winner
  -> trace why winner won
```

Do this for intra first. Inter comes later because it also requires reference management, motion
estimation, motion compensation, frame ordering, entropy contexts, and reconstruction correctness.

### Search-policy object

Do not scatter `if speed >= N` checks across the encoder. Centralize them:

```rust
pub struct SearchPolicy {
    pub partition_budget: PartitionBudget,
    pub intra_candidate_budget: CandidateBudget,
    pub inter_candidate_budget: CandidateBudget,
    pub transform_search: TransformSearchPolicy,
    pub filter_search: FilterSearchPolicy,
    pub motion_search: MotionSearchPolicy,
    pub use_temporal_rdo: bool,
}
```

AV2-specific syntax rates should be added only after the corresponding AV2 spec section and AVM
behavior are mapped in `docs/SPEC-MAPPING.md`.

---

## 9. Tile and frame-boundary lessons

The tile/plane-region lesson is especially important for a safe Rust encoder. Prediction,
reconstruction, and filters must not operate on raw slices with scattered edge checks.

Recommended first milestone:

```text
Plane<P>
RegionRect
PlaneRegion<'a, P>
PlaneRegionMut<'a, P>
EdgePolicy
TileRect
TileState
```

Test cases to add before prediction kernels grow:

```text
1x1 frame
width not divisible by block size
height not divisible by block size
both dimensions not divisible by superblock size
odd chroma sizes
right-edge block
bottom-edge block
multi-tile right boundary
multi-tile bottom boundary
transform block straddling visible edge
```

A safe edge policy is a place where `splot` can be **better than the references**: make edge access
explicit and fuzzed instead of implied by pointer arithmetic.

---

## 10. Fuzzing, testing, and profiling lessons

Start fuzzing at the bitstream boundary and grow inward:

```text
parse_annexb_obu
parse_obu_header
roundtrip_obu_writer_parser
parse_sequence_header
parse_frame_header
encode_intra_tiny_frame
rdo_partition_search_random_planes
transform_quant_roundtrip
motion_search_small_refs
filter_edge_cases
```

Differential testing principle for AV2:

```text
splot encode or parse
  -> splot validate
  -> AVM inspect/decode/reference behavior
  -> compare parser traces, reconstruction, or diagnostics
  -> preserve failures in corpus
```

Profiling order:

1. Scalar correctness and deterministic traces.
2. Fuzz and differential tests.
3. Profiling spans and flamegraphs.
4. Pruning policy improvements.
5. Threading.
6. SIMD.

---

## 11. AV1 traps to reject

Never copy from rav1e into AV2:

- AV1 OBU header fields or OBU type table.
- AV1 frame header syntax.
- AV1 DPB/reference slot semantics.
- AV1 entropy CDFs, token tables, context maps, or probability update rules.
- AV1 transform tables, scan tables, filter tables, mode lists, or constants unless AV2 spec/AVM
  proves the same value and the value is re-derived in `splot`.
- AV1 public API names that would leak AV1 assumptions into AV2.
- Source comments or documentation prose verbatim.

Allowed use:

- Read source to understand architecture.
- Summarize ideas in original words.
- Link to upstream.
- Implement fresh AV2 code from AV2 spec and AVM.
- Add tests that prove AV2 behavior.

---

## 12. Suggested follow-up tasks

### Task: add `splot-encode` facade skeleton

- Add `EncoderConfig`, `EncoderContext`, `EncoderStatus`, and `EncodedPacket`.
- Use AV2 terms only.
- Return structured unimplemented errors for real encoding.
- Add state-transition tests.

### Task: add RDO cost types

- Add `Distortion`, `EstimatedBits`, `Lambda`, and `RdCost`.
- Add monotonicity tests.
- Do not add AV1 rate tables.

### Task: add frame/plane region types

- Model visible and padded dimensions separately.
- Add edge-policy tests.
- Do not implement prediction until regions are safe.

### Task: add encoder trace schema

- Trace prediction plan, RDO candidates, ME result summary, RC decisions, and OBU packet summary.
- Keep disabled by default.
- Use JSON schema-friendly types.

---

## 13. PR checklist

Every PR inspired by rav1e must answer:

- Which AV2 spec sections define the behavior?
- Which AVM code path or reference stream validates it?
- Which rav1e files were read?
- What design idea was borrowed?
- What was explicitly **not** copied?
- Are all names AV2-specific rather than AV1-specific?
- Are there positive, negative, edge, fuzz, and differential tests?
- Is the implementation deterministic?
- Is the scalar path correct before any SIMD/threading?
- Does the trace/debug output explain decisions?
