## Why

The decoder now has crate-private CDF context derivation and individual AV2 §5.20.3.2 partition-entry `S()` symbol reads, but it still has no typed boundary that follows the local `read_partition()` branch order to produce a final partition decision from already-derived facts. Adding that boundary is the next smallest Phase 3 step before broader recursive partition traversal.

## What Changes

- Add Feature ID `DECODE-TILE-PARTITION-DECISION-BOUNDARY` for a crate-private `splot-decode` partition-decision helper.
- Introduce a typed partition decision model for the ten AV2 §5.20.3.2 partition outcomes and the branch-local syntax trace.
- Consume existing `TileCdfSubset::read_partition_entry_symbol` reads for `do_split`, `do_square_split`, `rect_type`, `do_ext_partition`, and `do_uneven_4way_partition` only when the §5.20.3.2 branch order reaches them.
- Consume the one-bit `uneven_4way_partition_type L(1)` only when `do_uneven_4way_partition` is true.
- Keep `partition_implied`, `init_allowed_partitions`, `is_partition_allowed`, `Partition_Subsize`, `H_Partition_Midsize`, recursive `read_partition()`, recursive `decode_partition()`, `decode_tile()`, reconstruction, output, reference refresh, `exit_symbol()`, and Saved CDF/frame-end update behavior out of scope.

## Capabilities

### New Capabilities
- `tile-partition-decision-boundary`: Crate-private AV2 §5.20.3.2 partition decision boundary over caller-provided allowed/implied partition facts.

### Modified Capabilities
- `decoder-support`: Track the new decoder support matrix row and clarify that broader tile payload, CDF selection, and partition symbol-read rows remain partial or narrow.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload.rs`, a new `crates/splot-decode/src/tile_payload/partition.rs`, and existing tile-payload/CDF tests as needed.
- Affected docs/status: `docs/DECODER-ROADMAP.md`, `docs/DECODER-SUPPORT-MATRIX.toml`, generated decoder support/status coverage docs, `docs/IMPLEMENTATION-MATRIX.toml`, and generated feature/spec coverage docs.
- APIs: crate-private only; no public library, CLI, dependency graph, fixture, AVM/dav2d, or CI integration changes.
- Diagnostics: no new public diagnostic is expected for this internal boundary; typed internal errors should preserve selector/symbol/literal failures and caller fact inconsistencies for future decode diagnostics.
