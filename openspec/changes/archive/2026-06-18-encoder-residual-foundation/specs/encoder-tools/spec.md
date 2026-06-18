## ADDED Requirements

### Requirement: Encoder residual foundation

The encoder SHALL provide a private residual-calculation stage tracked by
`ENC-RESIDUAL-FOUNDATION`. For the current 8-bit YUV420 input surface, the stage
SHALL compute signed row-major residual blocks as `source_sample -
prediction_sample` over validated borrowed input planes and caller-provided
prediction samples. The stage SHALL validate geometry and prediction shape
before returning residual data, SHALL use explicit signed arithmetic/storage,
and SHALL NOT emit syntax or create coded packets.

#### Scenario: Valid residual block computes signed differences

- **WHEN** a block rectangle inside a borrowed visible input plane and matching
  row-strided prediction samples are supplied
- **THEN** the residual stage SHALL return row-major signed samples equal to
  source minus prediction for each block sample
- **AND** the result SHALL retain the plane id and block rectangle used to
  compute it.

#### Scenario: Strided visible input and prediction rows are honored

- **WHEN** the input plane visible rectangle or prediction buffer uses stride
  padding outside the selected block
- **THEN** only samples inside the selected block SHALL contribute to the
  residual values
- **AND** padding samples SHALL NOT affect the returned row-major residuals.

#### Scenario: Invalid residual inputs are rejected

- **WHEN** the selected block is outside the visible input plane, the prediction
  stride is too small, or the prediction buffer cannot cover the selected block
- **THEN** the residual stage SHALL return a typed encoder error
- **AND** SHALL NOT return partial residual data.

#### Scenario: Residual foundation does not produce packets

- **WHEN** residual calculation is available in `splot-encode`
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
  until a later tile-body and writer integration change lands
- **AND** no documentation or matrix row SHALL claim Baseline Encoder Profile v1
  output from residual calculation alone.
