## Why

The decodable-tile arc, brick 1. To produce a tile the AVM-validated general intra decode
path accepts, the encoder must emit the symbol stream that path reads — and the FIRST symbol
it reads (before any block mode or coefficient) is the § 5.20.3.2 `do_split` partition flag.
This is the one primitive genuinely missing from splot-encode (a grep for `do_split` returns
zero non-comment hits); every existing composer starts at the mode-info prefix.

## What Changes

- Add `ENC-DO-SPLIT-PARTITION-SYMBOL` as an encoder block-symbol-trace feature (splot-encode).
- Add a `partition_emission` module with `PartitionToken` / `PartitionCdfRowSelector::DoSplit`
  and `emit_root_do_split_none()` — the `do_split == false` (`PARTITION_NONE`) token for the
  root 64x64 superblock, coded against `TileDoSplitCdf[plane_start 0][ctx 12]`.
- Extend `BlockSymbolToken` with a `Partition(PartitionToken)` variant and route it through
  `symbol()`, the `BlockSymbolTraceCdfRows` (a `do_split_root` row from
  `DEFAULT_DO_SPLIT_CDF[0][12]`), and `row_mut`, so it composes through the existing
  `roundtrip_block_symbol_trace` / `encode_block_symbol_trace`.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the encoder root `do_split` partition symbol.

## Impact

- Affected code: `crates/splot-encode/src/partition_emission.rs` (new),
  `crates/splot-encode/src/block_symbol_trace.rs` (the token + CDF row + routing),
  `crates/splot-encode/src/block_symbol_trace_tests.rs` (roundtrip test),
  `crates/splot-encode/src/lib.rs` (module).
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature status/spec
  coverage, `openspec/specs/encoder-tools/spec.md`.
- Public API impact: none (all crate-private). No dependency-graph change.
- Validator/CLI impact: none.
