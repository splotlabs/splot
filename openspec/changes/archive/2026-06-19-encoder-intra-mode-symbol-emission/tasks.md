## 1. Intra-mode emission API

- [x] 1.1 Add a private `intra_mode_emission` module that produces ordered `y_mode_set` and `y_mode_index` entropy-token records for the current minimal DC_PRED luma block at the tile-origin neutral context.
- [x] 1.2 Derive the §8.3.2 CDF row selectors (`y_mode_set` with no context, `y_mode_index` with the tile-origin context 0) and cite §5.20.5.5 for the syntax.
- [x] 1.3 Add typed encoder errors for unsupported CDF selectors and the §8.2 symbol write/read/finalize/mismatch boundaries.
- [x] 1.4 Add a roundtrip helper that writes the token records through the in-tree AV2 §8.2 symbol encoder and decodes them back through the symbol decoder.
- [x] 1.5 Wire the private module into `splot-encode` without re-exporting it or changing packet output.

## 2. Intra-mode emission tests

- [x] 2.1 Prove the minimal DC_PRED block emits exactly the ordered `y_mode_set=0` and `y_mode_index=0` token records with their scoped CDF selectors.
- [x] 2.2 Prove the token records roundtrip through the §8.2 symbol encoder/decoder back to the same ordered symbols.
- [x] 2.3 Add negative tests for an out-of-range `y_mode_index` context selector.
- [x] 2.4 Preserve an explicit no-packet-output test while intra-mode emission exists.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-INTRA-MODE-SYMBOL-EMISSION` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Update encoder roadmap/gap audit notes without claiming chroma mode, coefficient, tile-body, packet, CLI, or Baseline Encoder Profile v1 behavior.
- [x] 3.3 Run OpenSpec validation, focused encoder tests, feature-status checks, and `cargo xtask ci`.
