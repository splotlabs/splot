# Agent Log

## Planning

- Orchestrator selected `tile-rect-type-cdf-selector` as a narrow expansion of
  `DECODE-TILE-CDF-SELECTION-BOUNDARY`, not a `read_partition()` or
  `decode_tile()` implementation.
- Planning explorer `019ecb5f-8c3a-7630-b39d-99f8a8537252`: PASS. Confirmed
  `TileRectTypeCdf` is a coherent selector-only expansion of
  `DECODE-TILE-CDF-SELECTION-BOUNDARY`, with generated
  `DEFAULT_RECT_TYPE_CDF: [[[i32; 3]; 64]; 2]`, and identified the required
  anchors: § 5.20.3.2, § 8.2.2, § 8.3.1, § 8.3.2, and § 9.3.

## Implementation

- Added crate-private `TileCdfSelector::RectType { plane_start, ctx }` and
  `TileCdfArray::RectType` mapped to `TileRectTypeCdf`.
- Extended `TileCdfRows` with generated `DEFAULT_RECT_TYPE_CDF` rows shaped as
  `[[[i32; 3]; 64]; 2]`.
- Added checked immutable and mutable row access for `TileRectTypeCdf` with
  typed `plane_start` and `ctx` bounds errors.
- Included `TileRectTypeCdf` in saved CDF copy/average handling while preserving
  the existing unsupported boundary for real tile completion and frame-end CDF
  updates.
- Kept all implementation crate-private and preserved public API, dependency
  graph, scheduler behavior, runtime output behavior, and AVM/dav2d exclusion.
- Moved CDF unit tests into `crates/splot-decode/src/tile_payload/cdf/tests.rs`
  so `cdf.rs` stays below the 1000-line source-file soft budget.

## Testing

- `cargo fmt --all`: PASS.
- `cargo test -p splot-decode tile_payload::cdf --locked`: PASS, 5 tests.
- `cargo test -p splot-decode tile_payload --locked`: PASS, 38 tests.
- `cargo clippy -p splot-decode --all-targets --all-features --locked -- -D warnings`: PASS.
- `cargo xtask check-feature-status`: PASS.
- `cargo xtask check-decoder-support`: PASS.
- `cargo xtask check-decoder-conformance-coverage`: PASS.
- `openspec validate tile-rect-type-cdf-selector --strict`: PASS.
- `git diff --check`: PASS.
- `cargo xtask ci`: PASS after the post-review test-module split and paperwork
  updates.

## Documentation

- Updated `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, and `docs/DECODER-ROADMAP.md` to list
  `TileRectTypeCdf` in the selector-only CDF boundary.
- Preserved residual documentation that `rect_type` context derivation,
  `read_partition()`, `decode_tile()`, full § 8.3 CDF selection, real tile
  completion mutation, frame-end saved-CDF updates, reconstruction, output
  hashes/Y4M, AVM/dav2d integration, and scheduler changes remain out of scope.
- Regenerated decoder support/status, feature status, and decoder conformance
  coverage docs through the repo checks.

## Reviews

- Correctness/spec review agent `019ecb64-6e9e-7e52-92d0-8d77bf366dfd`: PASS.
  Confirmed `TileRectTypeCdf` uses generated defaults, validates
  `plane_start < 2` and `ctx < 64`, and matches AV2 § 8.2.2, § 8.3.2, and
  § 9.3 anchors. No blocking findings.
- Safety review agent `019ecb64-8840-7130-8183-8545bb7a3e30`: PASS. Confirmed
  checked access, fixed-size memory, closure-scoped mutable row handoff, no
  public API exposure, and no dependency/scheduler/process/output changes. No
  blocking findings.
- Performance/data-layout review agent `019ecb64-9f5f-7240-ae5a-d152d4bd4f47`:
  PASS. Confirmed bounded fixed-size storage growth, no extra hot-path copies,
  deterministic saved averaging, no scheduler interaction, and honest docs. No
  blocking findings.
