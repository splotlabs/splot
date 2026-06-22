## 1. Implementation
- [x] 1.1 Add the `decide` module + `RateController` trait + `ConstantQp` impl.
- [x] 1.2 Hold a `ConstantQp` in `Context`; route `base_q_idx` through `frame_base_q_idx()`.

## 2. Tests
- [x] 2.1 `ConstantQp` returns the configured qp (direct + via the trait object).
- [x] 2.2 The cross-tool oracle stays byte-identical (decode(encode(all-128))==all-128).

## 3. Tracking
- [x] 3.1 Add the `ENC-DECIDE-RATE-CONTROLLER` matrix row.
- [x] 3.2 Regenerate feature status; run `cargo xtask ci`.
