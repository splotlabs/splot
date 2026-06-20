## Why

Writer-bridge brick 2. After the § 5.20.1 single-tile `TileGroupFraming` constructor
(`ENC-WRITER-INPUT-FRAMING`), `write_tile_group_obu` also needs a § 5.19
`TileGroupStructure` — also `#[non_exhaustive]` and parse-only. This adds the
constructor for the single-tile first-tile-group structure the encoder needs.

## What Changes

- Add `ENC-WRITER-INPUT-STRUCTURE` as an encoder writer-input-bridge feature (code in
  `splot-core`, driven by the encoder mission).
- Add `TileGroupStructure::single_tile_first_group() -> TileGroupStructure` in
  `splot-core`: `NumTiles == 1` so `tile_start_and_end_present_flag` is inferred `0`,
  `tg_start = 0`, `tg_end = 0`, `outcome = Complete`. `header_bytes` / `payload_size`
  are the parser's byte-accounting and are left `None` — the § 5.19 writers ignore
  them (parse-context they recompute).
- Prove the constructor has the canonical single-tile fields and that
  `write_tile_group_structure` of it reparses to the same syntax fields (a semantic
  round-trip on `flag` / `tg_start` / `tg_end`).

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the encoder writer-input single-tile
  first-group structure constructor.

## Impact

- Affected code: `crates/splot-core/src/headers/tile_group.rs` (the constructor +
  tests). No new types, no signature changes; `#[non_exhaustive]` is preserved.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status/spec coverage, encoder roadmap/gap audit, `openspec/specs/encoder-tools/spec.md`.
- Public API impact: one new associated constructor on the existing public
  `splot-core` `TileGroupStructure`. No dependency-graph change.
- Validator/CLI impact: none.
