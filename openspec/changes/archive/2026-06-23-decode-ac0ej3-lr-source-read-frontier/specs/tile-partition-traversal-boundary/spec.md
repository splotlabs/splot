## ADDED Requirements

### Requirement: Active Wiener NS LR Source-Read Frontier

The tile partition traversal boundary and minimal runtime SHALL advance active
frame-level Wiener NS loop-restoration source-bound facts to a fail-closed
classified/source-read frontier for `DECODE-AC0EJ3-LR-SOURCE-READ-FRONTIER`.
The frontier SHALL gate active classified luma before §7.20.4, SHALL use
caller-resolved AV2 §7.20.1 bounds to attempt AV2 §7.20.2 source sample
selection/read state for supported unclassified active block output samples,
§7.20.3 Wiener tap coordinates, and chroma luma-source coordinates, and MUST
NOT apply §7.20.3/§7.20.4 filtering or produce decoded output.

#### Scenario: Classified luma is gated before source reads

- **WHEN** retained active luma Wiener NS source-bound facts use more than one
  frame-level filter class
- **THEN** the runtime emits a structured unsupported diagnostic for the
  §7.20.4 classified-luma boundary before deriving source reads

#### Scenario: Active source reads are attempted after bounds

- **WHEN** a supported root LR frontier retains active Wiener NS source-bound
  facts
- **THEN** the runtime attempts source sample selection state for active output,
  tap, and luma-source coordinates
- **AND** the previous source-bounds diagnostic is no longer the live ac0ej3
  frontier

#### Scenario: Wiener taps and chroma luma-source reads are covered

- **WHEN** the source-read frontier derives state for an active luma Wiener NS
  block
- **THEN** it resolves the output sample centers and Wiener tap coordinates
  through the §7.20.2 source sample process
- **WHEN** the source-read frontier derives state for an active chroma Wiener NS
  block in a 4:2:0 sequence
- **THEN** it resolves chroma output/tap coordinates and the corresponding luma
  source coordinates through the §7.20.2 source sample process

#### Scenario: Source-read accounting uses a source-read budget

- **WHEN** source-read state is derived for multiple planes
- **THEN** it is charged to `DecodeLimitName::MaxLoopRestorationSourceReads`
- **AND** it is not rejected solely because source-read operations exceed
  `DecodeLimitName::MaxLumaSamplesPerFrame`

#### Scenario: Source reads and classification remain fail-closed before filtering

- **WHEN** the active classified/source-read boundary is reached for the local
  ac0ej3 mission stream
- **THEN** the runtime emits a structured unsupported diagnostic for the
  classified/source-read/filtering frontier
- **AND** no source sample value reads, §7.20.3/§7.20.4 filtering,
  decoded-frame allocation, reference refresh, hash, raw, or Y4M output is
  produced

#### Scenario: Source-read failures are transactional

- **WHEN** source sample selection derivation fails for an active LR block
- **THEN** the runtime reports a structured decode error or unsupported
  diagnostic
- **AND** LR CDF mutations and retained frontier state are not committed past
  the failed read boundary
