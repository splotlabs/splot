# Agent Log

## Planning

- Orchestrator selected `tile-square-split-context-derivation` as the next
  narrow expansion of `DECODE-TILE-CDF-SELECTION-BOUNDARY` after PR #154 merged
  left/above-neighbor context derivation for the other partition-entry CDFs.
- Planning reviewer `019ecbb5-5399-7862-ba79-6be97fa64df3`: PASS.
  Recommended the existing `tile-square-split-context-derivation` change id, a
  separate `SquareSplitContextInput<'a>` shape, availability-gated `MiSizes`
  lookups, typed grid/underflow errors, `BLOCK_256X256` context coverage, and no
  runtime partition traversal.

## Implementation

- Added crate-private `SquareSplitContextInput<'a>` in
  `crates/splot-decode/src/tile_payload/cdf/context.rs`.
- Added typed `MiSizes` grid underflow, row, column, and block-size errors to
  the tile CDF boundary.
- Implemented AV2 § 8.3.2 `do_square_split` context derivation for
  `TileCdfSelector::DoSquareSplit`.
- Left syntax reads, partition decisions, `read_partition()`, and
  `decode_tile()` out of scope.

## Testing

- Added math, bounds, availability short-circuit, `BLOCK_256X256`, and generated
  default-CDF row handoff tests in the crate-private context module.
- `cargo test -p splot-decode tile_payload::cdf --locked`: PASS.
- `cargo test -p splot-decode tile_payload --locked`: PASS.
- `cargo clippy -p splot-decode --all-targets --all-features --locked -- -D warnings`:
  PASS.
- `openspec validate tile-square-split-context-derivation --strict`: PASS.
- `cargo xtask check-feature-status`: PASS.
- `cargo xtask check-decoder-support`: PASS.
- `cargo xtask check-decoder-conformance-coverage`: PASS.
- `git diff --check`: PASS.
- `cargo xtask ci`: PASS.
- Post-archive `openspec validate --all --no-interactive`: PASS.
- Post-archive `cargo xtask check-feature-status`: PASS.
- Post-archive `cargo xtask check-decoder-support`: PASS.
- Post-archive `cargo xtask check-decoder-conformance-coverage`: PASS.
- Post-archive `git diff --check`: PASS.
- Post-archive `cargo xtask ci`: PASS.

## Documentation

- Updated `docs/DECODER-ROADMAP.md`, `docs/DECODER-SUPPORT-MATRIX.toml`, and
  `docs/IMPLEMENTATION-MATRIX.toml`.
- Ran generated-doc refresh commands for decoder support status, feature status,
  and decoder conformance coverage. The rendered markdown stayed unchanged
  because the changed fields are not displayed in those tables.
- Archived the OpenSpec change as
  `openspec/changes/archive/2026-06-15-tile-square-split-context-derivation`
  and verified the `Square split context is bounded` scenario folded into
  `openspec/specs/decoder-support/spec.md`.

## Reviews

- Correctness/spec reviewer `019ecbc3-2c0c-7942-a76f-7d3a0d8d6057`: PASS.
  Verified the § 8.3.2 formula, `BLOCK_256X256` index, availability
  short-circuiting, typed bounds errors, selector bounds, and no AVM/dav2d
  source or runtime invocation.
- Safety reviewer `019ecbc3-2fe1-7370-9727-62ff6ec9bfeb`: PASS. Verified
  checked underflow, `.get`-guarded grid/table access, no new library
  `unwrap`/`expect`/`panic`, and no CLI/output-file path changes.
- Performance/data-layout reviewer `019ecbc3-7017-7d31-8a13-8a57f064aec5`:
  PASS. Verified borrowed `MiSizes`, no allocation or grid copy, no global state
  or scheduler changes, and no avoidable hot-path copies. Residual notes:
  local `BLOCK_256X256_INDEX` is acceptable for this crate-private slice, and
  `context.rs` is near the 1000-line soft budget, so the next context expansion
  should split the module/tests before growing it further.
