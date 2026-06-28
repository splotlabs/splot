## Context

`xtask/src/source_lines.rs` enforces a 1000-line soft limit and a 2500-line hard
cap, with per-file allowances. Four files hold allowances today:

| File | Lines | Allowance | Tests/docs share | Activity (90d) |
| --- | --- | --- | --- | --- |
| `crates/splot-core/src/headers/frame/info.rs` | 5213 | 5340 | ~55% tests | 25 commits (hot) |
| `crates/splot-validate/src/celu.rs` | 3693 | 3693 | ~67% tests+doc | 1 commit (cold) |
| `crates/splot-core/src/headers/sequence.rs` | 3282 | 3328 | ~36% tests | 11 (mostly sweeps) |
| `crates/splot-decode/src/runtime_minimal/wienerns_lr/tx_records.rs` | 2615 | 2615 | ~20% in-file + 4 ext | 18 (active ac0ej3) |

A maintainability review proposed splitting all four by responsibility:

- `info.rs` → `activation, show_existing, bridge, intra, inter_control, shared_tail, status`
- `celu.rs` → `facts, roles, phase_order, output_presence, order_hint/doh, leadingness, diagnostics`
- `sequence.rs` → `profile, level, chroma, timing, mlayer_presence, mod`
- `tx_records.rs` → "split the chroma residual decoders into a submodule"

A structural read of each file (full-file analysis, per-file) found that the
proposal is directionally right (the files are too long) but that most of its
named modules do not map to real seams: several already live in sibling modules,
several are interleaved fields of one shared accumulator, and the dominant size
driver in three of four files is the in-file **test module**, which the proposal
never addresses. This design records the seams the code actually supports and the
order in which to act, given that two of the four files are under active
bit-exact decoder development.

## Goals / Non-Goals

**Goals:**

- Retire the allowances for the files that can be split safely now (`celu.rs`,
  then `sequence.rs`), reducing each under the hard cap.
- Split along real, low-coupling seams; preserve public APIs via re-exports and
  keep behavior byte-identical.
- Record a deferred, real-seam plan for the two hot files so a future split does
  not repeat the proposal's mis-cuts.
- One file per PR; remove each file's allowance in the PR that shrinks it.

**Non-Goals:**

- No code changes in this OpenSpec change (plan only).
- No AV2 syntax/semantics, encoder, conformance, or fuzz changes.
- No crate dependency-graph change (every move stays within its crate).
- No public API, ABI, or diagnostic `rule_id` change.
- No split of `frame/info.rs` or `tx_records.rs` now — both deferred.

## Decisions

### Decision 1: Split `celu.rs` first by relocating tests, not by responsibility

`celu.rs` is cold (1 commit/90d) and ~67% non-logic (~2267 lines of `#[test]` +
~224 lines of rustdoc). Its ~1196 lines of production code are one cohesive
state machine: every helper is `fn(celu: &mut CeluState, …)` mutating one shared
~20-field accumulator, and `observe_frame` (~lines 846–1073) records clk/olk
identity, leadingness, ascending-mlayer order, output-slot grammar, unit
accounting, lowest-layer kind, and order-hint capture in one pass with subtle
ordering dependencies (the round-numbered F1/F2/F3/F5 and poison-scope
invariants the rustdoc spends ~120 lines justifying against AVM behavior).

- **Do:** move the in-file `#[test]` module to a `#[path = "celu/tests.rs"]`
  sibling (optionally topic-split: ordering / output-presence / clk-olk-leading /
  doh / ambiguity-poison). The tests touch only `pub(crate)` surface, so a
  sibling preserves access. Optionally extract `DohTuAccumulator` (its own
  struct + impl, narrow 3-method interface) to `celu/doh.rs`.
- **Result:** `celu.rs` drops to ~1200–1400 lines (production + rustdoc), under
  the hard cap; allowance removed.
- **Alternative considered (rejected):** the proposed 8-way split. `facts`/
  `roles` already live in `context/frame_facts.rs`; `phase_order`,
  `output_presence`, and `leadingness` are short helpers mutating shared
  `CeluState` fields inside `observe_frame` — extracting them widens field
  visibility and scatters the poison-scope reasoning that makes the machine
  correct. It maps "poorly" to the code.

### Decision 2: Convert `sequence.rs` to a `sequence/` directory with re-exports

`sequence.rs` is organized by AV2 spec-section affinity (each child-config struct
sits next to its parser), not by "value models vs parser logic." The clean
low-coupling islands are:

- `profile.rs` — `ProfileIdc` enum + `from_bits/get/is_reserved/is_configurable`
  + its hand-written `PartialEq/Eq/PartialOrd/Ord/Hash` on the raw 5-bit value
  (the only type with custom identity/order/hash; touched only via `from_bits`/
  `get`). This is the single cleanest seam in any of the four files.
- `layer_dependency.rs` — `MLayerDependencyMap`, `MLayerPresenceMap`,
  `TLayerDependencyMap` (kept **together**: `MLayerPresenceMap` is produced by
  `MLayerDependencyMap::presence_map()` — one transitive-closure unit), plus the
  private `DependencyMaps` + `parse_dependency_maps` and the `MAX_NUM_*` consts.
- `child_configs.rs` — `SuperblockSize`, the eight §5.4.x child-config structs
  and their parsers, `DrlReorder`/`CdefOnSkipTxfm`, `read_drl_reorder` (~860
  lines: the biggest real mass, missed by the proposal). May further become a
  `sequence/configs/` dir, one file per §5.4.x config, if still over budget.

The file becomes `sequence/mod.rs` holding the `SequenceHeaderGeneral`/
`SequenceHeader` composite, the orchestrating parsers, and **re-exports** of
every moved public item.

- **Alternative considered (rejected):** standalone `level.rs`/`chroma.rs` (≈35 /
  ≈80 lines) — over-fragmentation for negligible payoff; fold these scalar types
  into `mod.rs` or a single `scalar_types.rs`. `mlayer_presence.rs` as proposed
  is rejected — it would split a single closure derivation. `timing.rs` framed as
  a "value model" is rejected — it is a parser+struct pair; keep timing and the
  decoder-model-info pair together if extracted at all.
- **Critical constraint:** ≈70 files import via `crate::headers::sequence::…`,
  while `headers.rs` re-exports only `SequenceHeader`/`SequenceHeaderGeneral`.
  Every moved public item MUST be re-exported from `sequence/mod.rs` (and any
  `headers.rs` re-export preserved) or the split becomes large import churn.

### Decision 3: Defer `frame/info.rs`; pre-record its real seams

`info.rs` is hot (25 commits/90d) and is the deliberate state-owning orchestrator
of the frame-header parse: every path mutates one ~45-field `FrameHeaderCore` and
reads shared `CoreSeqView`/`MfhFrameView`/`FrameReferenceStateView`; the heavy
per-structure parsers (config, quant, segmentation, tiling, filtering,
restoration, inter, inter_shared_tail, tail, size) are **already** sibling
modules. ~2860 lines are integration tests driving the public entry. The
maintainer's allowance reason already says "split separately."

- **When stable, split by real seams:** `status` (the `FrameHeaderParseMode`/
  `FrameHeaderParseStatus`/`FrameType`/`SefTrailingBits` vocabulary, low
  coupling), `show_existing` (the SEF path + `classify_sef_trailing_bits`, the
  lowest-coupling island), and `seq_view` (`CoreSeqInterView`/`CoreSeqView`/
  `MfhFrameView` input-gathering). Move tests to a `#[path]` sibling.
- **Rejected proposal modules:** `activation` (the prefix lives in the parent
  `mod.rs`), `inter_control`/`shared_tail` (already in `inter.rs`/
  `inter_shared_tail.rs`), and `bridge`-as-one (the two bridge variants straddle
  the inter/intra dispatch fork — they are not one cohesive unit).

### Decision 4: Defer `tx_records.rs`; pre-record its real seams, not chroma-first

`tx_records.rs` is the active ac0ej3 decoder frontier — nearly every recent
commit touches it, and its allowance reason explicitly defers ("Split the chroma
residual decoders into a submodule separately if this grows further"). It is one
cohesive §5.20.5/§5.20.6/§5.20.7 intra tile-decode walk; its center of gravity
(the 396-line orchestrator + four residual-chunk functions) threads a dense web
of private types by value, and the luma/chroma walks are deliberately interleaved
(`decode_selectable_residual_chunks` calls `decode_chroma_group` inline).

- **When stable, split by real seams** (mirroring the already-extracted
  `ccso.rs`/`max_rect.rs`/`skip_records.rs` precedent): `per_block_syntax`
  (`DeltaQState` + `CdefState`, low coupling), `tx_partition` (the
  `apply_tx_partition` family + `read_tx_partition_symbols` geometry), and
  `error` (the `SelectableTransformRecordError` taxonomy).
- **Rejected:** chroma-first. `decode_chroma_group` is invoked inline from the
  luma walk — extracting it exports a `pub(super)` seam without reducing the
  interleaving; it is neither the biggest nor the most decoupled part.

## Risks / Trade-offs

- **Scattering hard-won correctness history** → split only cold files now; split
  by cohesive seams (never per-field); move tests with the code; keep each split
  behavior-preserving and gated by the existing test/golden suite.
- **Import churn from the `sequence/` directory** → re-export every moved public
  item from `sequence/mod.rs`; verify with a full `-p splot-core` build that no
  call site needs edits.
- **`#[path]` test relocation losing `use super::*` access to privates** → before
  moving, confirm the test module touches only `pub(crate)`/`pub` surface (true
  for `celu.rs`); if a test reaches a private item, narrow it or keep it in-file.
- **Intra-doc link breakage** (repo memory: `missing_errors_doc` / dangling
  intra-doc links can break `cargo doc -D warnings`) → move rustdoc that
  cross-references a moved item (`[DohTuAccumulator]`) with it, and run the
  doc gate per split.
- **Acting on a deferred file** contradicts the maintainer's recorded intent and
  the AGENTS.md "keep the diff scoped" rule → `info.rs` and `tx_records.rs` stay
  untouched until their frontier stabilizes; this change only records their plan.
- **dupehound/visibility blast radius** when promoting shared helpers to
  `pub(super)` → prefer seams whose helpers are already local; measure with
  `cargo xtask check-duplication` per split.

## Migration Plan

1. Land this plan (proposal + spec + design + tasks); add
   `INFRA-MODULE-SIZE-REFACTOR` to `docs/IMPLEMENTATION-MATRIX.toml`.
2. PR 1 — `celu.rs`: relocate tests to `celu/tests.rs` (optional `celu/doh.rs`);
   remove the `celu.rs` allowance; run `cargo xtask ci`.
3. PR 2 — `sequence.rs` → `sequence/` dir: extract `profile.rs`,
   `layer_dependency.rs`, `child_configs.rs`; re-export; lower/remove the
   `sequence.rs` allowance; run `cargo xtask ci`.
4. Hold `info.rs` and `tx_records.rs`. When the ac0ej3 decoder frontier
   stabilizes, open PR 3 / PR 4 using the seams in Decisions 3–4, each as its own
   dedicated PR with maintainer review.

Rollback: each PR is an isolated, behavior-preserving move; revert the single PR
to restore the prior file (and its allowance) with no downstream impact.

## Open Questions

- Should `child_configs.rs` be one file or a `sequence/configs/` directory (one
  file per §5.4.x config)? Decide at PR 2 based on the post-extraction line count.
- For `celu.rs`, split tests into topic files now, or one `celu/tests.rs`? Lean to
  one file unless it lands over the soft limit on its own.
- Is `INFRA-MODULE-SIZE-REFACTOR` the right umbrella ID for all four files, or
  should each deferred file get its own row when its split lands? (Maintainer
  call; default: one umbrella row, partial until the deferred files land.)
