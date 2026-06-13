## ADDED Requirements

### Requirement: decoder diagnostic registry enforcement

The repository SHALL enforce that `docs/DECODER-DIAGNOSTICS.md` lists exactly
the emitted `decode/*` diagnostic `rule_id` literals present in current decoder
emission source roots. `cargo xtask check-diagnostic-registry`, run as part of
`cargo xtask ci`, SHALL compare the emitted decoder `rule_id` set against the
marker-delimited registry region and fail on drift in either direction. The
gate SHALL reject diagnostic-looking rule IDs in decoder emission roots or the
decoder registry when they do not use the `decode/*` namespace. Tracked by
`XTASK-DECODER-DIAGNOSTIC-REGISTRY`.

#### Scenario: emitted decoder rule ID missing from the registry

- **WHEN** a decoder emission source contains a `decode/*` rule-ID literal that
  is absent from the decoder registry region
- **THEN** `cargo xtask check-diagnostic-registry` fails and names the
  undocumented ID

#### Scenario: decoder registry lists an ID not present in source

- **WHEN** the decoder registry region documents a `decode/*` rule ID that does
  not appear in decoder emission source
- **THEN** `cargo xtask check-diagnostic-registry` fails and names the unemitted
  ID

#### Scenario: decoder registry matches source

- **WHEN** the documented decoder registry IDs equal the emitted decoder rule-ID
  literals
- **THEN** `cargo xtask check-diagnostic-registry` passes the decoder registry
  check

#### Scenario: decoder rule ID uses another namespace

- **WHEN** a decoder emission source or the decoder registry contains a
  diagnostic-looking rule ID outside the `decode/*` namespace
- **THEN** `cargo xtask check-diagnostic-registry` fails and names the
  unsupported namespace
