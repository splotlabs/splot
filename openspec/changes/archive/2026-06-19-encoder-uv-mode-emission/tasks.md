## 1. uv_mode emission API

- [x] 1.1 Extend the private `intra_mode_emission` module with a `uv_mode` syntax / CDF selector and an `emit_minimal_dc_chroma_uv_mode` function producing the ordered AV2 §5.20.5.6 `uv_mode` token record for the DC chroma mode at the non-directional context 0.
- [x] 1.2 Reuse the existing token / §8.2 roundtrip machinery and typed error model; hold only the non-directional `TileUVModeCflNotAllowedCdf` row and reject other contexts.
- [x] 1.3 Cite §5.20.5.6 (read intra UV mode) and §8.3.2 (CDF context = `is_directional(YMode)`, 0 for DC_PRED).

## 2. uv_mode emission tests

- [x] 2.1 Prove the DC chroma mode emits exactly the ordered `uv_mode=0` token record with its scoped CDF selector.
- [x] 2.2 Prove the token record roundtrips through the §8.2 symbol encoder/decoder back to the same symbol.
- [x] 2.3 Add a negative test for a non-supported `uv_mode` context selector.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-UV-MODE-SYMBOL-EMISSION` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Update encoder roadmap/gap audit notes without claiming coefficient, tile-body, packet, CLI, or Baseline Encoder Profile v1 behavior.
- [x] 3.3 Run OpenSpec validation, focused encoder tests, feature-status checks, and `cargo xtask ci`.
