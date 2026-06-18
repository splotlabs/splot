## 1. Tokenization API

- [x] 1.1 Add a private `coefficient_tokenization` module with scan/EOB metadata, sign/magnitude facts, and ordered entropy-token records for the 4x4 DCT_DCT DC-only subset.
- [x] 1.2 Add typed encoder errors for unsupported coefficient-tokenization shape, non-DC coefficients, unsupported magnitudes, scan derivation, and symbol roundtrip failures.
- [x] 1.3 Wire the private module into `splot-encode` without re-exporting it or changing packet output.

## 2. Tokenization Tests

- [x] 2.1 Add positive tests for all-zero, positive DC-only, and negative DC-only tokenization.
- [x] 2.2 Add token-to-range-byte-to-symbol-decode roundtrip tests through `splot-core` section 8.2 symbol encoder/decoder.
- [x] 2.3 Add negative tests for unsupported non-DC coefficients and magnitudes outside the current base-symbol tier.
- [x] 2.4 Preserve an explicit no-packet-output test while coefficient tokenization exists.

## 3. Tracking And Verification

- [x] 3.1 Add `ENC-COEFFICIENT-TOKENIZATION-MINIMAL` to the implementation matrix and refresh generated status docs.
- [x] 3.2 Update encoder roadmap/gap audit notes without claiming tile-body, packet, CLI, rate-control, or Baseline Encoder Profile v1 behavior.
- [x] 3.3 Run OpenSpec validation, focused encoder tests, feature-status checks, and `cargo xtask ci`.
