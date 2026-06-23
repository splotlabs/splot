## ADDED Requirements

### Requirement: Active Wiener NS LR Source-Read Frontier

The tile partition traversal boundary and minimal runtime SHALL advance active
frame-level Wiener NS loop-restoration source-bound facts to a fail-closed
source-read frontier for `DECODE-AC0EJ3-LR-SOURCE-READ-FRONTIER`. The frontier
SHALL use caller-resolved AV2 §7.20.1 bounds to attempt AV2 §7.20.2 source
sample selection state for supported active block centers, and MUST NOT claim
complete Wiener tap reads, chroma luma-source reads, §7.20.3 filtering, or
decoded output.

#### Scenario: Active source reads are attempted after bounds

- **WHEN** a supported root LR frontier retains active Wiener NS source-bound
  facts
- **THEN** the runtime attempts source sample selection state for active
  block-center coordinates
- **AND** the previous source-bounds diagnostic is no longer the live ac0ej3
  frontier

#### Scenario: Source reads remain fail-closed before filtering

- **WHEN** the active source-read boundary is reached for the local ac0ej3 mission
  stream
- **THEN** the runtime emits a structured unsupported diagnostic for the
  source-read/filtering frontier
- **AND** no complete Wiener tap reads, chroma luma-source reads, §7.20.3 Wiener
  NS filtering, decoded-frame allocation, reference refresh, hash, raw, or Y4M
  output is produced

#### Scenario: Source-read failures are transactional

- **WHEN** source sample selection derivation fails for an active LR block
- **THEN** the runtime reports a structured decode error or unsupported
  diagnostic
- **AND** LR CDF mutations and retained frontier state are not committed past
  the failed read boundary
