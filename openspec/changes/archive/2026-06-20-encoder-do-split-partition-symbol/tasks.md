## 1. do_split partition emitter

- [x] 1.1 Add `partition_emission` with `PartitionToken` / `PartitionCdfRowSelector::DoSplit` and `emit_root_do_split_none()` (symbol 0, `plane_start 0`, `ctx 12`), the ctx pinned against the q80 decode.
- [x] 1.2 Extend `BlockSymbolToken` with `Partition(PartitionToken)` and route it through `symbol()`, the `do_split_root` CDF row (`DEFAULT_DO_SPLIT_CDF[0][12]`), and `row_mut`.

## 2. Tests

- [x] 2.1 A unit test pins the token (`PARTITION_NONE`, symbol 0, `DoSplit { plane_start: 0, ctx: 12 }`).
- [x] 2.2 A roundtrip test: the token round-trips through one § 8.2 coder to `decoded_symbols == [0]`, `symbol_count == 1`.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-DO-SPLIT-PARTITION-SYMBOL` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Keep tracking honest: one private partition symbol, not a block trace, a tile, a frame, a packet, `receive_packet` output, CLI success, or Baseline Encoder Profile v1.
- [x] 3.3 Run OpenSpec validation, focused encode tests, feature-status checks, and `cargo xtask ci`.
