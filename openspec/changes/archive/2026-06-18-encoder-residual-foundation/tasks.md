## 1. Residual API

- [x] 1.1 Add a private `residual` module with `ResidualBlock` metadata/accessors and checked construction from a borrowed input plane plus row-strided prediction samples.
- [x] 1.2 Add typed encoder errors for residual block bounds, prediction stride/length, and allocation failure.
- [x] 1.3 Wire the private module into `splot-encode` without re-exporting it or changing packet output.

## 2. Residual Tests

- [x] 2.1 Add positive residual tests for zero residuals, min/max differences, checkerboard/gradient data, and odd-edge blocks with strided input/prediction.
- [x] 2.2 Add negative tests for out-of-bounds block rectangles, too-small prediction stride, and truncated prediction buffers.
- [x] 2.3 Preserve an explicit no-packet-output test while residual calculation exists.

## 3. Tracking And Verification

- [x] 3.1 Add `ENC-RESIDUAL-FOUNDATION` to the implementation matrix and refresh generated status docs.
- [x] 3.2 Update encoder roadmap/gap audit notes without claiming transform, quantization, tile-body, packet, or CLI success behavior.
- [x] 3.3 Run focused encoder tests, OpenSpec validation, feature-status checks, and `cargo xtask ci`.
