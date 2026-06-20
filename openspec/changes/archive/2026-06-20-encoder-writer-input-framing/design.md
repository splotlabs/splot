## Context

`splot-core`'s § 5.20.1 / § 5.19 writer functions (`write_tile_group_payload`,
`write_tile_group_obu`) take parsed models (`TileGroupFraming`, `TileGroupStructure`,
`FrameHeaderCore`, `CoreSeqView`) as input. Those models are `#[non_exhaustive]` with
no public constructor — splot-core's own writer tests build a framing only by
`parse_tile_group_framing` then rewrite it. So `splot-encode`, which has only coded
bytes, cannot construct a framing to drive the writer. The maintainer approved adding
encoder-friendly constructors to `splot-core` (the writer bridge).

This first brick adds the smallest one: a single-tile `TileGroupFraming`. For a single
(last) tile the § 5.20.1 `tile_group_payload()` carries no `tile_size_minus_1` field
(mirror :8557), so the framing is a single `TileFraming { tile_num: 0,
size_field_offset: None, tile_data_offset: 0, tile_size }` with `defect: None`. This is
exactly what `parse_tile_group_framing(payload, 0, 0, _, false)` reproduces, so the
constructor is the encoder-side inverse of the parser and a write→reparse round-trips
value-equal — the test that proves correctness.

`#[non_exhaustive]` is preserved (a within-crate struct literal builds the internal
`TileFraming`; external crates still cannot struct-literal either type).

## Goals / Non-Goals

**Goals:**

- A public `splot-core` constructor for the conformant single-tile § 5.20.1 framing,
  proven equal to the parser's output and round-tripping through the writer.

**Non-Goals:**

- No multi-tile framing constructor, no `TileGroupStructure` / `FrameHeaderCore` /
  `CoreSeqView` constructors (later bridge bricks), no tile-group OBU, no frame,
  no packet, no encoder consumption yet.

## Decisions

1. **Single-tile only.** The minimal encoder frame is single-tile; a multi-tile
   constructor (which must compute size-field offsets) is a later brick.

2. **Inverse-of-parser contract.** The constructor returns exactly the parser's
   defect-free single-tile framing, so the round-trip is the correctness oracle; it
   does not itself encode the `ZeroSizeTile` defect (the writer rejects a zero-size
   tile, and callers pass real coded bytes).

## Flight Manifest

- Change ID: `encoder-writer-input-framing`
- Feature IDs: `ENC-WRITER-INPUT-FRAMING`
- Base commit: `7d3a7821`
- Depends on merged changes: the § 5.20.1 parser/writer (`tile-payload-boundary-validation`, `tile-group-payload-writer`, both archived).
- Exact files/directories owned by this PR:
  - `crates/splot-core/src/headers/tile_group.rs` (the `single_tile` constructor + its two tests only)
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/spec-coverage-writer.md` (regenerated: the constructor is a `splot-core` write-surface capability)
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `openspec/changes/encoder-writer-input-framing/**`
  - `openspec/changes/archive/2026-06-20-encoder-writer-input-framing/**`
  - `openspec/specs/encoder-tools/spec.md`
- Exact files/directories forbidden to this PR:
  - all `splot-encode` / `splot-decode` / `splot-validate` / `splot-cli` source
  - all other `splot-core` modules (parsers, other writers, the existing framing
    parser body) and the `AV2-5.20-TILE-GROUP-PAYLOAD` matrix row
  - workspace manifests and `Cargo.lock`; AV2 spec mirror under `docs/spec/av2/**`
- Public APIs/types owned: `TileGroupFraming::single_tile` (new associated fn)
- Matrix rows owned: `ENC-WRITER-INPUT-FRAMING`
- Generated files owned: `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`, `docs/spec-coverage-writer.md`
- Open sibling PRs audited: none at base.
- Changed-file intersection with each sibling PR: none. If a decoder/core-mission PR
  touches `tile_group.rs` first, sync `main`, re-run the focused tests, regenerate the
  tracking docs, and re-gate BEFORE pushing.
- Semantic overlap with each sibling PR: none (a new associated fn + two tests).
- Can build/test/merge directly onto main without another open PR: yes.

## Risks / Trade-offs

- [Risk] Adding a constructor to a `splot-core` model owned by the core/writer
  mission. -> Mitigation: maintainer-approved (the writer-bridge decision); the
  constructor is `#[must_use]`, preserves `#[non_exhaustive]`, cites the spec, and is
  proven to match the parser — the established convention for these models.
