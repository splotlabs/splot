## 1. Traversal Design Plumbing

- [x] 1.1 Add the `DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY` feature row to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 1.2 Add `tile-partition-traversal-boundary` to `docs/DECODER-SUPPORT-MATRIX.toml` with supported scope and residuals.
- [x] 1.3 Wire a new crate-private `tile_payload::partition_traversal` module without public API exposure.

## 2. Traversal Implementation

- [x] 2.1 Implement checked frontier input/state types for tile MI geometry, frame facts, tree/chroma state, context state, symbol decoder, and tile CDF subset.
- [x] 2.2 Implement bounded read-only `MiSizes`, `LeftMiSizes`, and `AboveMiSizes` context state access for §8.3.2 partition CDF contexts.
- [x] 2.3 Implement §5.20.3.1 prefix child-call planning to the first `decode_block()` frontier for `NONE`, `HORZ`, `VERT`, `SPLIT`, `HORZ_3`, `VERT_3`, `HORZ_4A`, `HORZ_4B`, `VERT_4A`, and `VERT_4B`.
- [x] 2.4 Compose existing allowed-partition, CDF-context, symbol-read, and decision helpers for each reached partition decision.
- [x] 2.5 Add typed unsupported/residual errors for SDP, BRU-active, bridge/inter, block syntax, and unsupported state surfaces.

## 3. Tests

- [x] 3.1 Add focused positive tests for frontier boundaries and each supported prefix child-call shape in AV2 order.
- [x] 3.2 Add tests proving transactional CDF handling, disabled CDF update immutability, and no `exit_symbol()`/Saved CDF copyback.
- [x] 3.3 Add negative tests for checked arithmetic, context-state bounds, invalid `BLOCK_INVALID` child calls, and unsupported SDP/BRU/inter gates.
- [x] 3.4 Run targeted tests: `cargo test -p splot-decode tile_payload --locked` and `cargo clippy -p splot-decode --all-targets --all-features --locked -- -D warnings`.

## 4. Documentation And Generated Status

- [x] 4.1 Regenerate decoder support/status and feature status docs.
- [x] 4.2 Update `docs/DECODER-ROADMAP.md` if the tile traversal backlog wording changes.
- [x] 4.3 Run `cargo xtask check-decoder-support`, `cargo xtask feature-status`, and `cargo xtask check-feature-status`.
- [x] 4.4 Run `openspec validate --all --no-interactive`.

## 5. Review, Archive, And Gate

- [x] 5.1 Run required subagent reviews for correctness/spec exactness, security/arithmetic/allocation, and performance/data layout.
- [x] 5.2 Fix or explicitly disposition every review finding.
- [x] 5.3 Archive the OpenSpec change with `openspec archive tile-partition-traversal-boundary --yes`.
- [x] 5.4 Run `cargo xtask ci` after archive and before PR.
