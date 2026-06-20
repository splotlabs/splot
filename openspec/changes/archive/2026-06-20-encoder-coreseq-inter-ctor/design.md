## Context

Header-bridge brick 1. The encoder packet path needs a `CoreSeqView` to drive
`write_tile_group_obu` (it serializes the whole frame header from a `&CoreSeqView` +
`&FrameHeaderCore`). `CoreSeqView` bundles seven nested sequence sub-views. A design
pass confirmed six of them (`CoreSeqQuantView`, `SegView`, `TileView`, `FilterView`,
`RestorationView`, `CcsoView`) are already publicly constructible (all-pub fields, not
`#[non_exhaustive]`), and only `CoreSeqInterView` is blocked (`#[non_exhaustive]`,
crate-private fields, no constructor). This brick unblocks it.

An intra frame never reads the § 5.4.6 inter config — the § 5.18.2 control region skips
the inter tail — so the inert state is the all-disabled view (every inter tool off,
every motion mode disabled), which is exactly what the `base_inter()` test helpers
build by hand. The constructor returns that, and the three `base_inter()` helpers are
promoted to call it, so the entire existing frame-header round-trip suite regresses the
constructor. `CoreSeqInterView` has no `PartialEq`, so the direct test asserts the
fields; the round-trips prove it serializes correctly.

`#[non_exhaustive]` is preserved (a within-crate struct literal).

## Goals / Non-Goals

**Goals:**

- A public `splot-core` constructor for the all-disabled minimal-intra
  `CoreSeqInterView`, proven by field assertions and the existing round-trip suites.

**Non-Goals:**

- No `CoreSeqView` constructor (brick 2), no `FrameHeaderCore` assembler (brick 3), no
  re-exports (brick 4), no tile-group OBU / frame / packet / encoder consumption.

## Decisions

1. **All-disabled, zero-arg.** An intra frame reads no inter config, so the view has no
   degrees of freedom worth a parameter; the constructor is zero-arg and returns the
   inert state.

2. **Promote `base_inter()` for the oracle.** `CoreSeqInterView` has no `PartialEq`, so
   instead of a fragile clone-compare the three `base_inter()` helpers delegate to the
   constructor — the existing round-trip suites then regress it at zero new-test cost,
   plus one direct field test.

## Flight Manifest

- Change ID: `encoder-coreseq-inter-ctor`
- Feature IDs: `ENC-WRITER-INPUT-INTER-VIEW`
- Base commit: `321547ce`
- Depends on merged changes: `encoder-writer-input-framing`, `encoder-writer-input-structure`.
- Exact files/directories owned by this PR:
  - `crates/splot-core/src/headers/frame/info.rs` (the `new_minimal_intra` constructor, one field test, the promoted `base_inter`)
  - `crates/splot-core/src/write/frame_header_core_tests.rs` (promoted `base_inter`)
  - `crates/splot-core/src/write/tile_group_obu_tests.rs` (promoted `base_inter`)
  - `xtask/src/source_lines.rs` (info.rs allowance bump +41 for the constructor + test)
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `openspec/changes/encoder-coreseq-inter-ctor/**`
  - `openspec/changes/archive/2026-06-20-encoder-coreseq-inter-ctor/**`
  - `openspec/specs/encoder-tools/spec.md`
- Exact files/directories forbidden to this PR:
  - all `splot-encode` / `splot-decode` / `splot-validate` / `splot-cli` source
  - all other `splot-core` modules; the `CoreSeqView` struct/`from_sequence`; the
    other nested sub-views; the tile-group constructors / matrix rows
  - workspace manifests and `Cargo.lock`; AV2 spec mirror under `docs/spec/av2/**`
- Public APIs/types owned: `CoreSeqInterView::new_minimal_intra` (new associated fn)
- Matrix rows owned: `ENC-WRITER-INPUT-INTER-VIEW`
- Generated files owned: `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`
- Open sibling PRs audited: none at base.
- Changed-file intersection with each sibling PR: none. If a decoder/core-mission PR
  touches `info.rs`/`frame_header_core_tests.rs`/the matrix first, sync `main`,
  regenerate the tracking docs, and re-gate BEFORE pushing.
- Semantic overlap with each sibling PR: none.
- Can build/test/merge directly onto main without another open PR: yes.

## Risks / Trade-offs

- [Risk] Adding a constructor to a `splot-core` model owned by the core mission. ->
  Mitigation: maintainer-approved writer-bridge decision; `#[must_use]`,
  `#[non_exhaustive]` preserved, spec cited (§ 5.4.6 + mirror path), and the promoted
  `base_inter()` ties it to the existing parser/writer round-trip proofs.
