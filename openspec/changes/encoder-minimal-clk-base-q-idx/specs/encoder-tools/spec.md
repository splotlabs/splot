## ADDED Requirements

### Requirement: base_q_idx-parameterized minimal CLK container

The encoder writer bridge SHALL assemble the minimal `OBU_CLOSED_LOOP_KEY` IVF container at a
caller-chosen `base_q_idx`, tracked by `ENC-MINIMAL-CLK-BASE-Q-IDX`, so a frame can be muxed
whose decoder-derived coefficient CDF q-context matches the coded `tile_data` (`base_q_idx <=
90` selects q-context 0). The frozen no-arg assemblers SHALL keep their `base_q_idx == 255`
behavior unchanged. A `base_q_idx == 0` SHALL be rejected with a typed error (it would make
`CodedLossless == 1` and change the § 5.18.2 body layout the canonical writer does not model).

#### Scenario: base_q_idx 80 reproduces the AVM-validated fixture

- **WHEN** the minimal CLK IVF is assembled at `base_q_idx == 80` with the
  `syn-flat-intra-64x64-q80.ivf` fixture's own `tile_data`
- **THEN** the emitted bytes SHALL equal that AVM- and dav2d-validated fixture exactly.

#### Scenario: base_q_idx 0 is rejected

- **WHEN** the minimal CLK IVF is requested at `base_q_idx == 0`
- **THEN** assembly SHALL fail with a typed `LosslessBaseQIdx` error before any bytes are
  produced.

#### Scenario: the bridge does not decode

- **WHEN** the `base_q_idx`-parameterized container is available
- **THEN** no documentation or matrix row SHALL claim a decode, a coded skip frame, CLI
  success, or Baseline Encoder Profile v1 output from it; the cross-crate decode oracle is a
  later brick.
