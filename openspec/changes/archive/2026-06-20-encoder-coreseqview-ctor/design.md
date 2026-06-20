## Context

Header-bridge brick 2. `write_tile_group_obu` serializes the whole frame header from a
`&CoreSeqView` + `&FrameHeaderCore`. Brick 1 added the one blocked nested view
(`CoreSeqInterView`); this brick adds the `CoreSeqView` itself so the encoder can build
the sequence-derived input without a parsed `SequenceHeader`.

The constructor reproduces exactly the view the `base_seq()` writer/parser test helper
built by hand — every unused sequence tool disabled, 8-bit YUV420 — parameterized by
the two things that vary for the encoder's target: the § 5.4.1 frame-size maxima and
`single_picture_header_flag`. For a single-picture frame `frame_size_override_flag` is
inferred 0, so the frame size is taken from the maxima; the flag also propagates to the
§ 5.4.10 filter view and the § 5.4.1 CCSO view (the § 6.17.2 single-picture inferences
read all three).

The oracle is the existing frame-header round-trip suite: `base_seq()` is promoted to
delegate to the constructor, so all ~45 round-trip tests that use it now regress the
constructor (`CoreSeqView` has no `PartialEq`, so this delegation is the proof rather
than a clone-compare). Promoting `base_seq()` makes the nested-view test helpers
(`test_sub_views`/`base_filter`/`base_restoration`/`base_ccso`/`base_inter`) dead; they
are removed, so the constructor (~110 lines) replaces ~124 lines of test helpers and
`info.rs` stays under its existing 5090 allowance.

`#[non_exhaustive]` is preserved (a within-crate struct literal).

## Goals / Non-Goals

**Goals:**

- A public `splot-core` minimal-intra `CoreSeqView` constructor, regressed by the
  frame-header round-trip suite (via the promoted `base_seq()`) plus a parameterization
  test.

**Non-Goals:**

- No `FrameHeaderCore` assembler (brick 3), no re-exports (brick 4), no tile-group OBU
  / frame / packet / encoder consumption.

## Decisions

1. **Promote `base_seq()` for the oracle.** `CoreSeqView` has no `PartialEq`; delegating
   the round-trip-proven `base_seq()` helper to the constructor regresses it across the
   whole suite at zero new-test cost, and removing the now-dead nested helpers keeps
   `info.rs` under its allowance (net ~+9 lines).

2. **Parameterize only what varies.** The encoder target differs from `base_seq()` only
   in `single_picture_header_flag` and the frame-size maxima, so those are the three
   parameters; everything else is the fixed minimal-intra shape.

## Flight Manifest

- Change ID: `encoder-coreseqview-ctor`
- Feature IDs: `ENC-WRITER-INPUT-SEQ-VIEW`
- Base commit: `c66f42b6`
- Depends on merged changes: `encoder-coreseq-inter-ctor`.
- Exact files/directories owned by this PR:
  - `crates/splot-core/src/headers/frame/info.rs` (the `new_minimal_intra` ctor, the `base_seq` promotion + dead-helper removal, one parameterization test)
  - `crates/splot-core/src/write/frame_header_core_proptests.rs` (the inter-view de-dup)
  - `xtask/src/source_lines.rs` (the info.rs allowance reason)
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/spec-coverage-writer.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `openspec/changes/encoder-coreseqview-ctor/**`
  - `openspec/changes/archive/2026-06-20-encoder-coreseqview-ctor/**`
  - `openspec/specs/encoder-tools/spec.md`
- Exact files/directories forbidden to this PR:
  - all `splot-encode` / `splot-decode` / `splot-validate` / `splot-cli` source
  - all other `splot-core` modules; the `FrameHeaderCore` / tile-group constructors;
    `CoreSeqView::from_sequence` body
  - workspace manifests and `Cargo.lock`; AV2 spec mirror under `docs/spec/av2/**`
- Public APIs/types owned: `CoreSeqView::new_minimal_intra` (new associated fn)
- Matrix rows owned: `ENC-WRITER-INPUT-SEQ-VIEW`
- Generated files owned: `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`, `docs/spec-coverage-writer.md`
- Open sibling PRs audited: none at base.
- Changed-file intersection with each sibling PR: none. If a decoder/core-mission PR
  touches `info.rs`/the matrix first, sync `main`, regenerate the tracking docs, and
  re-gate BEFORE pushing (as the prior bricks did with #348/#354).
- Semantic overlap with each sibling PR: none.
- Can build/test/merge directly onto main without another open PR: yes.

## Risks / Trade-offs

- [Risk] Removing the `base_seq()` nested-view helpers could change the round-trip
  fixtures if the constructor's values differ. -> Mitigation: the constructor
  reproduces the exact `base_seq()` values (the entire round-trip suite stays green —
  83 frame_header_core tests), so the delegation is behaviour-preserving.
- [Risk] Adding a public constructor to a `splot-core` model owned by the core mission.
  -> Mitigation: maintainer-approved writer-bridge decision; `#[must_use]`,
  `#[non_exhaustive]` preserved, § 5.4.1 + mirror path cited; the net `info.rs` growth
  is ~+9 lines (the ctor offsets the removed helpers) and stays under the allowance.
