## 1. Tile MI-Size State Boundary

- [x] 1.1 Add a crate-private `tile_payload::mi_size_state` module with checked
  luma/chroma MI-size grid and neighbor-line initialization.
- [x] 1.2 Implement checked luma block updates for `MiSizes[0]`,
  `LeftMiSizes[0]`, and `AboveMiSizes[0]` using validated AV2 block sizes.
- [x] 1.3 Implement checked optional chroma block updates for `MiSizes[1]`,
  `LeftMiSizes[1]`, and `AboveMiSizes[1]` from caller-supplied chroma facts.
- [x] 1.4 Expose read-only state views compatible with
  `TilePartitionContextState` without adding public APIs.

## 2. Runtime Integration

- [x] 2.1 Replace minimal runtime ad hoc MI-size context vectors with the new
  state boundary.
- [x] 2.2 Apply the minimal root block MI-size update after traced block-symbol
  success without changing hash/raw/Y4M output bytes.
- [x] 2.3 Keep broad `decode_block()`, recursive `read_partition()`,
  reconstruction expansion, and reference refresh unsupported.

## 3. Tests

- [x] 3.1 Add focused unit tests for initialization, padded allocation
  accounting, luma footprint updates, chroma footprint updates, state view
  reflection, out-of-bounds failures, pre-allocation resource limits, and
  no-mutation-on-error behavior.
- [x] 3.2 Add or update runtime tests proving the minimal fixture hash/raw/Y4M
  outputs remain byte-identical after state integration.
- [x] 3.3 Run targeted tests:
  `cargo test -p splot-decode tile_payload --locked`,
  `cargo test -p splot-decode runtime_hash --locked`,
  `cargo test -p splot-decode runtime_raw --locked`, and
  `cargo test -p splot-decode runtime_y4m --locked`.

## 4. Documentation And Gates

- [x] 4.1 Add `DECODE-TILE-MI-SIZE-STATE-BOUNDARY` to the implementation matrix
  and decoder support matrix with proof commands and honest non-goals.
- [x] 4.2 Update roadmap/status/coverage generated docs as required.
- [x] 4.3 Run `openspec validate tile-mi-size-state-boundary --strict`,
  `openspec validate --all --no-interactive`, `cargo xtask feature-status`,
  `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, and
  `cargo xtask ci`.
- [x] 4.4 Run independent correctness, security, and performance reviews before
  commit/PR.
