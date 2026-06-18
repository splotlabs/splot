## 1. Feature Metadata

- [x] 1.1 Add `DECODE-COEFF-BASE-CDF-ROWS` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 1.2 Add a `coeff-base-cdf-rows` row to `docs/DECODER-SUPPORT-MATRIX.toml`.
- [x] 1.3 Add decoder conformance coverage metadata for `DECODE-COEFF-BASE-CDF-ROWS`.

## 2. CDF Row Boundary

- [x] 2.1 Add coefficient base/base-EOB/base-range row aliases and default-loaded fields to `BlockCdfRows`.
- [x] 2.2 Add typed `BlockCdfSelector`, `TileCdfSelector`, and `TileCdfArray` variants with bounds-checked row and row_mut access.
- [x] 2.3 Include the new rows in tile copy/save/average and frame-end count-scaling behavior.

## 3. Tests

- [x] 3.1 Add tests that coefficient base/base-EOB/base-range selectors return generated default rows.
- [x] 3.2 Add tests for selector bounds errors and tile-copy non-aliasing.
- [x] 3.3 Add a mutable-row handoff test through `read_block_symbol_trace` proving row mutation without runtime decode integration.

## 4. Verification

- [x] 4.1 Run `cargo test -p splot-decode tile_payload::cdf --locked`.
- [x] 4.2 Run `openspec validate coeff-base-cdf-rows --strict`.
- [x] 4.3 Run `cargo xtask feature-status`, `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, and `cargo xtask check-decoder-conformance-coverage`.
- [x] 4.4 Run `cargo xtask check-source-lines`.
- [x] 4.5 Run `env RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin cargo xtask ci`.
