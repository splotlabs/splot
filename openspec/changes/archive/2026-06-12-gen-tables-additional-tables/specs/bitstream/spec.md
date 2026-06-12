# bitstream delta: gen-tables-additional-tables

Advances `AV2-9-ADDITIONAL-TABLES` from stub to generated tables.

## ADDED Requirements

### Requirement: generated additional tables

The § 9 additional tables SHALL be code-generated from the
specification's `all_tables.h` attachment — committed under the
quarantined spec-mirror path with recorded provenance — by a
deterministic `cargo xtask gen-tables` whose output is drift-checked in
CI. Table contents SHALL never be hand-transcribed; the generator SHALL
fail loudly on unhandled constructs rather than silently skipping them.

#### Scenario: regeneration is byte-identical

- **WHEN** `cargo xtask gen-tables` runs against the committed
  attachment
- **THEN** the generated modules are byte-identical to the committed
  output and the drift check passes

#### Scenario: generated values match the mirror

- **WHEN** a generated table is compared against the mirror's § 9 text
- **THEN** spot-checked values agree, with the citation recorded in the
  test

#### Scenario: unhandled construct fails loudly

- **WHEN** the attachment contains a construct the generator does not
  model
- **THEN** generation fails with a diagnostic instead of emitting
  partial tables silently
