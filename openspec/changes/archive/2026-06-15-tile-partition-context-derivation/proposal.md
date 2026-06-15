# Proposal: Tile Partition Context Derivation

## Summary

Add crate-private AV2 § 8.3.2 context derivation helpers for the existing tile
partition CDF selector boundary. The helpers will derive selector contexts for
`do_split`, `rect_type`, `do_ext_partition`, and
`do_uneven_4way_partition` from bounded `LeftMiSizes` / `AboveMiSizes` inputs
and generated § 9.2 conversion tables.

## Scope

- Add a crate-private context module under `crates/splot-decode/src/tile_payload/cdf/`.
- Add bounded helper APIs that return existing `TileCdfSelector` values.
- Add tests for context math, bounds errors, and selector-to-row handoff.
- Update decoder matrix, roadmap, and decoder support OpenSpec text to state
  that left/above-derived partition CDF contexts are now modeled.
- Keep the `DECODE-TILE-CDF-SELECTION-BOUNDARY` feature partial.

## Non-Goals

- No `do_square_split` context derivation; that requires `MiSizes`, `AvailU`,
  and `AvailL` grid inputs and remains future work.
- No allowed/implied partition logic from § 5.20.3.2.
- No `S()`, `L(1)`, `read_symbol`, partition return values, recursive
  `read_partition()`, or `decode_tile()`.
- No mutation or lifecycle management for `LeftMiSizes` / `AboveMiSizes`.
- No full Tile/Saved CDF banks, `exit_symbol()` after real syntax,
  tile-completion copyback/averaging, `frame_end_update_cdf()`, reconstruction,
  hashes, Y4M output, reference refresh, public API, scheduler, dependency,
  AVM, or dav2d changes.

## Spec Anchors

- AV2 § 5.20.3.2 `read_partition()` ordering and syntax elements:
  `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-3-2`
- AV2 § 6.19.2.1 `LeftMiSizes` / `AboveMiSizes` initialization:
  `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-19-2-1`
- AV2 § 6.19.3.2 partition syntax semantics:
  `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-19-3-2`
- AV2 § 8.3.2 CDF context formulas:
  `docs/spec/av2/1.0.0/08-parsing-process.md#s-8-3-2`
- AV2 § 9.2 conversion tables:
  `docs/spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2`

## Validation Plan

- `cargo test -p splot-decode tile_payload::cdf --locked`
- `cargo test -p splot-decode tile_payload --locked`
- `cargo clippy -p splot-decode --all-targets --all-features --locked -- -D warnings`
- `cargo xtask check-feature-status`
- `cargo xtask check-decoder-support`
- `cargo xtask check-decoder-conformance-coverage`
- `openspec validate tile-partition-context-derivation --strict`
- `cargo xtask ci`
