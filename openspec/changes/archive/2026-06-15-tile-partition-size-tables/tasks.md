## 1. Implementation

- [x] 1.1 Extend `xtask gen-tables` with narrow `BLOCK_*` symbol resolution for `Partition_Subsize` and `H_Partition_Midsize`, then regenerate generated tables.
- [x] 1.2 Add `crates/splot-decode/src/tile_payload/partition_size.rs` with typed `BlockSize`, `PartitionSubsize`, and table lookup errors backed by generated core tables.
- [x] 1.3 Wire the new module into `tile_payload.rs` and reuse the existing `PartitionType` ordering without adding public APIs.
- [x] 1.4 Add core table spot tests and focused decoder unit tests for valid `Partition_Subsize` entries, invalid `BLOCK_INVALID` entries, `H_Partition_Midsize`, and out-of-range block-size inputs.

## 2. Documentation And Tracking

- [x] 2.1 Add Feature ID `DECODE-TILE-PARTITION-SIZE-TABLE-BOUNDARY` to `docs/IMPLEMENTATION-MATRIX.toml` with proof commands and source citations.
- [x] 2.2 Add decoder support row `tile-partition-size-tables` to `docs/DECODER-SUPPORT-MATRIX.toml`, and update nearby tile-payload/partition notes to remove only this specific exclusion.
- [x] 2.3 Update `docs/DECODER-ROADMAP.md` and regenerate `docs/DECODER-SUPPORT-STATUS.md`.

## 3. Validation

- [x] 3.1 Run `openspec validate tile-partition-size-tables --strict`.
- [x] 3.2 Run targeted checks: `cargo fmt --all -- --check`, `cargo xtask gen-tables --check`, `cargo test -p splot-core --test tables_spot --locked`, `cargo test -p splot-decode tile_payload --locked`, `cargo xtask check-decoder-support`, and `cargo xtask check-feature-status`.
- [x] 3.3 Run the acceptance gate `cargo xtask ci`.

## 4. Review And Archive

- [x] 4.1 Collect subagent review decisions and fix or document any findings.
- [x] 4.2 Archive the OpenSpec change and commit the archive in the branch.
- [x] 4.3 Re-run required local gates after archive before opening a ready pull request.
