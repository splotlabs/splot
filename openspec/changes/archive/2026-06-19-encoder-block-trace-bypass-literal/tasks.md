## 1. Bypass-literal token kind

- [x] 1.1 Add a `BlockSymbolToken::Bypass { width, value }` variant + a `bypass` constructor + `symbol()` view, representing an AV2 §8.2.5 `L(n)` literal.
- [x] 1.2 Dispatch bypass tokens in `roundtrip_block_symbol_trace` write/decode loops via `SymbolEncoder::write_literal` / `SymbolDecoder::read_literal`, before CDF-row selection; add the unreachable `row_mut` arm for exhaustiveness.
- [x] 1.3 Cite AV2 §8.2.5 (`L(n)` literal), §5.20.7.27 (`sign_bit`), §5.20.7.28 (golomb tail) as the motivating consumers.

## 2. Bypass-literal tests

- [x] 2.1 Prove bypass literals interleave bit-exactly with CDF symbols through one §8.2 coder (a mixed trace roundtrips to the expected ordered values).
- [x] 2.2 Prove the bypass-literal roundtrip is deterministic.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-INTRA-BLOCK-TRACE-BYPASS-LITERAL` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Update encoder roadmap/gap audit notes (the bypass-literal foundation unblocks coded chroma signs and the golomb tail) without claiming those consumers, tile-body, packet, CLI, or Baseline Encoder Profile v1 behavior.
- [x] 3.3 Run OpenSpec validation, focused encoder tests, feature-status checks, and `cargo xtask ci`.
