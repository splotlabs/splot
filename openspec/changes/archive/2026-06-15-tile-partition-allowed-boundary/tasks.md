## 1. Implementation

- [x] 1.1 Add `crates/splot-decode/src/tile_payload/partition_allowed.rs` and wire it into `tile_payload.rs` without changing public APIs.
- [x] 1.2 Extend the crate-private partition-size boundary with generated-table-backed block geometry helpers needed by allowed-partition derivation.
- [x] 1.3 Implement `get_plane_residual_size()` semantics for partition allowance using generated block dimensions and explicit `BLOCK_INVALID` handling.
- [x] 1.4 Implement `rect_type_implied_by_bsize`, `partition_implied_at_boundary`, and `partition_implied` over typed caller facts.
- [x] 1.5 Implement `is_partition_allowed`, `init_allowed_partitions`, and a convenience facts builder for the existing partition decision boundary.

## 2. Tests

- [x] 2.1 Add focused tests for direct and boundary implied-partition cases, including chroma `BLOCK_8X8` and `BLOCK_64X64` luma-partition reuse.
- [x] 2.2 Add focused tests for allowed-set derivation: partition-subsize sentinels, residual-size invalid cases, mixed-region 4x4 rejection, aspect-ratio gates, frame-edge `PARTITION_NONE` rejection, and fallback to `PARTITION_NONE`.
- [x] 2.3 Add focused tests for extended and uneven four-way gates, chroma-part rect-type implication, chroma-offset block-coded checks, and overflow-safe coordinate arithmetic.

## 3. Documentation And Tracking

- [x] 3.1 Add Feature ID `DECODE-TILE-PARTITION-ALLOWED-BOUNDARY` to `docs/IMPLEMENTATION-MATRIX.toml` with proof commands and source citations.
- [x] 3.2 Add decoder support row `tile-partition-allowed-boundary` to `docs/DECODER-SUPPORT-MATRIX.toml`, and update neighboring tile-payload/partition notes to reference the new boundary without overclaiming traversal.
- [x] 3.3 Update `docs/DECODER-ROADMAP.md` where needed and regenerate `docs/DECODER-SUPPORT-STATUS.md`, `docs/FEATURE-STATUS.md`, and `docs/SPEC-COVERAGE.md`.

## 4. Validation And Review

- [x] 4.1 Run `openspec validate tile-partition-allowed-boundary --strict`.
- [x] 4.2 Run targeted checks: `cargo test -p splot-decode tile_payload --locked`, `cargo clippy -p splot-decode --all-targets --all-features --locked -- -D warnings`, `cargo xtask check-decoder-support`, and `cargo xtask check-feature-status`.
- [x] 4.3 Collect independent subagent review decisions and fix or document any findings.
- [x] 4.4 Run the acceptance gate `cargo xtask ci`.

## 5. Archive

- [x] 5.1 Archive the OpenSpec change and commit the archive in the branch.
- [x] 5.2 Re-run required local gates after archive before opening a ready pull request.
