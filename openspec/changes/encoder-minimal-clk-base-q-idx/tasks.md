## 1. Parameterize base_q_idx

- [x] 1.1 Thread `base_q_idx` through `minimal_intra_clk_body_bytes`, `build_minimal_intra_clk_core_impl`, and the tile-group / Annex B / IVF impls; the frozen no-arg public functions delegate at `FROZEN_TIER_BASE_Q_IDX == 255`.
- [x] 1.2 Add the public `encode_minimal_intra_clk_ivf_with_base_q_idx(base_q_idx, tile_data)` and re-export it.
- [x] 1.3 Reject `base_q_idx == 0` with `MinimalIntraCoreError::LosslessBaseQIdx` (it would change the §5.18.2 body layout via `CodedLossless`).

## 2. Tests

- [x] 2.1 Byte-exact oracle: `encode_minimal_intra_clk_ivf_with_base_q_idx(80, q80_tile_data)` reproduces the AVM/dav2d-validated `syn-flat-intra-64x64-q80.ivf` byte-for-byte.
- [x] 2.2 `base_q_idx == 0` is rejected with the typed `LosslessBaseQIdx` error before any bytes are produced.
- [x] 2.3 The existing frozen-tier (255) round-trip and consistency tests still pass unchanged.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-MINIMAL-CLK-BASE-Q-IDX` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Keep tracking honest: a `base_q_idx`-parameterized container, not a decode, a coded skip frame, CLI success, or Baseline Encoder Profile v1; the cross-crate decode oracle is a later brick.
- [x] 3.3 Run OpenSpec validation, focused splot-core tests, feature-status checks, and `cargo xtask ci`.
