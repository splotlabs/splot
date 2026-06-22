## 1. Implementation
- [x] 1.1 Add `DoSquareSplit` syntax + selector + the root context (0) + the two emitters.
- [x] 1.2 Route the `do_square_split_root` CDF row through `BlockSymbolTraceCdfRows`.

## 2. Tests
- [x] 2.1 Emitter unit tests (symbols, selectors, contexts).
- [x] 2.2 `[do_split=1, do_square_split=1]` §8.2 roundtrip → `[1, 1]`.

## 3. Tracking
- [x] 3.1 Add the `ENC-PARTITION-DO-SQUARE-SPLIT` matrix row.
- [x] 3.2 Regenerate feature status + spec coverage; run `cargo xtask ci`.
