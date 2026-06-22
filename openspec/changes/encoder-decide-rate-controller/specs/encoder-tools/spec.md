## ADDED Requirements

### Requirement: RateController quantizer-decision seam

The encoder SHALL take the frame quantizer (`base_q_idx`, AV2 § 5.18.2) behind a
`RateController` trait seam, so a rate-controlled implementation can replace the fixed one
without changing the header writer or the coefficient path. The minimal encoder SHALL
install a `ConstantQp` controller built from `EncoderConfig.qp`, and emission SHALL obtain
`base_q_idx` from the seam. This is tracked by `ENC-DECIDE-RATE-CONTROLLER`.

#### Scenario: the seam returns the configured quantizer

- **WHEN** a `ConstantQp` is built from a config qp
- **THEN** `frame_base_q_idx()` returns that qp

#### Scenario: routing through the seam is byte-identical

- **WHEN** an all-128 frame is encoded with the quantizer obtained from the seam
- **THEN** the emitted packet decodes to the all-128 input exactly as before (no behaviour change)
