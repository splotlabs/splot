## ADDED Requirements

### Requirement: single fixed-quantizer source

The encoder SHALL read its fixed quantizer from one `EncoderConfig.qp` field, threading it
into the frame-header `base_q_idx` (AV2 § 5.18.2), so the quantizer index cannot diverge
across the bitstream. For `qp` outside the modeled coefficient-CDF-q-context range
(`1..=90`), `receive_packet` SHALL retire the frame without emitting a packet rather than
emit output it cannot honestly decode. This is tracked by `ENC-CONFIG-QP-FIELD`.

#### Scenario: default qp round-trips

- **WHEN** an all-128 64×64 frame is encoded at the default qp (80)
- **THEN** `splot decode` reconstructs the all-128 input

#### Scenario: non-default qp round-trips

- **WHEN** an all-128 64×64 frame is encoded at a non-default qp (40, in range)
- **THEN** `splot decode` reconstructs the all-128 input (the qp threaded into `base_q_idx`)

#### Scenario: out-of-range qp emits no packet

- **WHEN** the configured qp is outside `1..=90`
- **THEN** `receive_packet` retires the frame without a packet
