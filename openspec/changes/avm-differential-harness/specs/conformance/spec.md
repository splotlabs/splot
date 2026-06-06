# conformance delta: avm-differential-harness

## ADDED Requirements

### Requirement: AVM differential harness

`xtask` SHALL provide a reproducible `avm encode` → `splot validate` comparison
against a local AVM checkout/corpus, without vendoring AVM or running in normal CI.

#### Scenario: AVM-produced streams validate

- **WHEN** the harness runs over AVM-produced streams
- **THEN** `splot validate` accepts conformant streams and flags real defects
