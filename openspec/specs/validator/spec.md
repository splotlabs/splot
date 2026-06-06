# validator Specification

## Purpose

Parser-driven conformance diagnostics in `splot-validate`. Diagnostics are the
product: every finding is structured data (stable `rule_id`, `severity`, optional
`spec_section`, optional byte/bit offset, human-readable `message`). A malformed
bitstream is a report, never a process failure.

Tracked by Feature IDs: `AV2-5.2.2-OBU-HEADER` (header constraints),
`AV2-5.3-RESERVED-OBU`, `AV2-7.3-OBU-ORDERING`.

## Requirements

### Requirement: structured diagnostics

Every check SHALL emit `Diagnostic`s with a stable `rule_id`, a `severity`, the AV2
`spec_section` where applicable, and a byte offset where known.

#### Scenario: global xlayer constraint

- **WHEN** an `OBU_TEMPORAL_DELIMITER` has `obu_xlayer_id != GLOBAL_XLAYER_ID`
- **THEN** an error diagnostic `obu-header/global-xlayer-required` (§ 6.2.2) is produced

### Requirement: reserved OBU handling

A reserved OBU SHALL be reported informationally; a reserved OBU whose payload is
entirely zero SHALL be an error (AV2 v1.0.0 § 5.3 / § 6.2.3 require a non-zero
trailing bit).

#### Scenario: all-zero reserved payload

- **WHEN** a reserved OBU carries an entirely-zero payload
- **THEN** an error diagnostic `obu-reserved/all-zero-payload` is produced

### Requirement: diagnostic rule-id namespace

Diagnostic rule ids SHALL use a documented kebab/slash prefix (`obu-header/`,
`obu-reserved/`, `bitstream/`). Narrower diagnostics derived from a modeled feature
MAY use the Feature ID as a base with a `.SUFFIX`.

#### Scenario: undocumented prefix is rejected

- **WHEN** a diagnostic rule id uses a prefix that is not documented
- **THEN** `cargo xtask check-feature-status` fails
