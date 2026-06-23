## 1. General-context skip tokens

- [x] 1.1 Add `general_intra_64x64_luma_all_zero_token` (`TX_64X64` `txSzCtx 4`, luma `ctx 0`) and `general_intra_32x32_chroma_u_all_zero_token` (`TX_32X32` `txSzCtx 3`, U `ctx 6`); the `txSzCtx` values confirmed empirically against the decoder's general `txb_skip` selector.
- [x] 1.2 Route the two general `txb_skip` rows (`DEFAULT_TXB_SKIP_CDF[0][0][4][0]`, `[0][0][3][6]`) through `BlockSymbolTraceCdfRows` and `row_mut`; the V `txb_skip` reuses the neutral `[0][0]` row.

## 2. Composer

- [x] 2.1 Add `general_intra_trace::compose_general_intra_dc_skip_block_trace()`: `[do_split, y_mode_set, y_mode_index, uv_mode, luma all_zero, U all_zero, V all_zero]` at q-context `0`.

## 3. Tests

- [x] 3.1 Constructor tests pin each general token's selector (`TX_64X64`/`TX_32X32` `txSzCtx`, plane/ctx) and symbol `1`.
- [x] 3.2 A composer test asserts the ordered 7-token trace and symbols `[0, 0, 0, 0, 1, 1, 1]`.
- [x] 3.3 A roundtrip test: the trace round-trips through one § 8.2 coder to `decoded_symbols == [0, 0, 0, 0, 1, 1, 1]`, `symbol_count == 7`.

## 4. Tracking and verification

- [x] 4.1 Add `ENC-GENERAL-INTRA-SKIP-BLOCK-TRACE` to the implementation matrix and refresh generated status/coverage docs.
- [x] 4.2 Keep tracking honest: one in-memory symbol trace, not a tile, a frame, a packet, `receive_packet` output, CLI success, or Baseline Encoder Profile v1.
- [x] 4.3 Run OpenSpec validation, focused encode tests, feature-status checks, and `cargo xtask ci`.
