## 1. Implementation
- [x] 1.1 Add `EncoderConfig.qp` (+ `DEFAULT_QP`); thread into `base_q_idx` via a
      `_with_base_q_idx` skip emitter; `receive_packet` reads `self.config.qp`.
- [x] 1.2 Restrict to the supported q-context-0 range (`SUPPORTED_SKIP_QP = 1..=90`).

## 2. Tests
- [x] 2.1 Cross-tool oracle at the default qp (80) and a non-default qp (40).

## 3. Tracking
- [x] 3.1 Add the `ENC-CONFIG-QP-FIELD` matrix row.
- [x] 3.2 Regenerate feature status + spec coverage; run `cargo xtask ci`.
