## Context

Writer-bridge brick 2 (after `ENC-WRITER-INPUT-FRAMING`). `write_tile_group_obu` takes
a § 5.19 `TileGroupStructure` alongside the § 5.20.1 framing; like the framing it is
`#[non_exhaustive]` and parse-only. This adds the constructor the encoder needs for a
single-tile first tile group.

Unlike the framing constructor (an exact parser-inverse), the structure carries
parse-context byte-accounting (`header_bytes` / `payload_size`) the § 5.19 writers
ignore — `write_tile_group_structure` only reads `outcome` (must be `Complete`), the
`tile_start_and_end_present_flag`, and the `tg_start`/`tg_end` range; the OBU writer
recomputes the header length and takes the payload from the tile data. So the
constructor sets the writer-relevant fields canonically (`flag = false`, `tg_start = 0`,
`tg_end = 0`, `outcome = Complete`) and leaves `header_bytes`/`payload_size` `None`,
matching the existing `TileGroupStructure` writer-test helper. The correctness oracle
is a semantic round-trip (write → reparse → `flag`/`tg_start`/`tg_end` match), which is
exactly the contract the existing `write_tile_group_structure` round-trip tests use.

`#[non_exhaustive]` is preserved (a within-crate struct literal).

## Goals / Non-Goals

**Goals:**

- A public `splot-core` constructor for the single-tile first-tile-group § 5.19
  structure, proven canonical and semantically round-tripping through the writer.

**Non-Goals:**

- No multi-tile / continuation structure, no `CoreSeqView` / `FrameHeaderCore`
  constructors (later bridge bricks), no tile-group OBU, no frame, no packet, no
  encoder consumption yet.

## Decisions

1. **Single-tile first group only.** `tg_start = tg_end = 0`, `NumTiles == 1` — the
   minimal encoder frame; multi-tile/continuation structures are later bricks.

2. **Semantic round-trip, not exact parser-inverse.** `header_bytes`/`payload_size`
   are writer-ignored parse-context (the writer recomputes them), so the constructor
   leaves them `None` and the oracle asserts the syntax fields, matching the existing
   `write_tile_group_structure` round-trip convention.

## Flight Manifest

- Change ID: `encoder-writer-input-structure`
- Feature IDs: `ENC-WRITER-INPUT-STRUCTURE`
- Base commit: `bdd9dcc3`
- Depends on merged changes: `encoder-writer-input-framing`; the § 5.19 parser/writer (`tile-group-structure`/`tile-group-payload-writer`, archived).
- Exact files/directories owned by this PR:
  - `crates/splot-core/src/headers/tile_group.rs` (the `single_tile_first_group` constructor + its two tests only)
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/spec-coverage-writer.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `openspec/changes/encoder-writer-input-structure/**`
  - `openspec/changes/archive/2026-06-20-encoder-writer-input-structure/**`
  - `openspec/specs/encoder-tools/spec.md`
- Exact files/directories forbidden to this PR:
  - all `splot-encode` / `splot-decode` / `splot-validate` / `splot-cli` source
  - all other `splot-core` modules and the `TileGroupFraming::single_tile` constructor
    / the `AV2-5.19-TILE-GROUP` matrix row
  - workspace manifests and `Cargo.lock`; AV2 spec mirror under `docs/spec/av2/**`
- Public APIs/types owned: `TileGroupStructure::single_tile_first_group` (new associated fn)
- Matrix rows owned: `ENC-WRITER-INPUT-STRUCTURE`
- Generated files owned: `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`, `docs/spec-coverage-writer.md`
- Open sibling PRs audited: none at base.
- Changed-file intersection with each sibling PR: none. If a decoder/core-mission PR
  touches `tile_group.rs` or the matrix first, sync `main`, regenerate the tracking
  docs, and re-gate BEFORE pushing (as `ENC-WRITER-INPUT-FRAMING` did with #348).
- Semantic overlap with each sibling PR: none (a new associated fn + two tests).
- Can build/test/merge directly onto main without another open PR: yes.

## Risks / Trade-offs

- [Risk] Adding a constructor to a `splot-core` model owned by the core/writer
  mission. -> Mitigation: maintainer-approved writer-bridge decision; `#[must_use]`,
  `#[non_exhaustive]` preserved, spec cited (§ 5.19 + mirror path), and proven to
  write-encode + semantically round-trip via the existing writer-test contract.
