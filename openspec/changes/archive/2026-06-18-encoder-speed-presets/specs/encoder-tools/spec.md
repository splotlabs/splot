## ADDED Requirements

### Requirement: Typed runtime speed presets

The encoder runtime policy SHALL include a typed speed preset tracked by
`ENC-SPEED-PRESETS`. The preset SHALL be separate from `EncoderConfig`, SHALL have
a documented accepted numeric range, and SHALL be retained by `Context` without
creating coded packets while the encoder packet path remains unimplemented.

#### Scenario: Default runtime preset is explicit

- **WHEN** an `EncoderRuntimeConfig` is created with only a thread policy
- **THEN** it SHALL use the default speed preset
- **AND** `Context` SHALL expose that preset through runtime accessors.

#### Scenario: CLI speed is validated by the library type

- **WHEN** `splot encode --speed <n>` receives a supported preset value
- **THEN** the CLI SHALL pass the corresponding typed speed preset into
  `EncoderRuntimeConfig`
- **AND** the command SHALL continue to fail honestly because no coded packet
  path exists yet.

#### Scenario: Unsupported speed is rejected before context construction

- **WHEN** `splot encode --speed <n>` receives a value outside the accepted range
- **THEN** the value SHALL be rejected through the typed speed-preset validation
  path before encoder context construction.

#### Scenario: Speed preset is not bitstream configuration

- **WHEN** a caller chooses any accepted speed preset
- **THEN** `EncoderConfig` SHALL remain unchanged
- **AND** no documentation or API SHALL claim Baseline Encoder Profile v1 output,
  rate control, mode decision, or syntax emission from the preset framework alone.
