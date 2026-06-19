## 1. Block mode-trace API

- [x] 1.1 Add a private `block_symbol_trace` module with a `compose_minimal_intra_dc_block_mode_trace` function that returns the ordered `y_mode_set`, `y_mode_index`, `uv_mode` token sequence by reusing the existing luma/chroma mode emitters.
- [x] 1.2 Cite AV2 §5.20.5.3 for the `read_intra_y_mode()`-before-`read_intra_uv_mode()` order; reuse the existing token / §8.2 roundtrip machinery and typed errors.
- [x] 1.3 Wire the private module into `splot-encode` without re-exporting it or changing packet output.

## 2. Block mode-trace tests

- [x] 2.1 Prove the composed trace is exactly the ordered luma mode tokens followed by the chroma `uv_mode` token.
- [x] 2.2 Prove the composed sequence roundtrips through one §8.2 symbol encoder/decoder back to the same ordered symbols with shared CDF state.
- [x] 2.3 Prove the roundtrip is deterministic.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-INTRA-BLOCK-MODE-TRACE` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Update encoder roadmap/gap audit notes without claiming coefficient, tile-body, packet, CLI, or Baseline Encoder Profile v1 behavior.
- [x] 3.3 Run OpenSpec validation, focused encoder tests, feature-status checks, and `cargo xtask ci`.
