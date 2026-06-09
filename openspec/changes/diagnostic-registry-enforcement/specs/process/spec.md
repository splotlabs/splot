# process delta: diagnostic-registry-enforcement

Advances `XTASK-DIAGNOSTIC-REGISTRY`. Adds a repository process guarantee that the
validator diagnostics registry is complete and machine-enforced. No AV2 syntax change.

## ADDED Requirements

### Requirement: validator diagnostic registry enforcement

The repository SHALL enforce that `docs/VALIDATOR-DIAGNOSTICS.md` lists exactly the
diagnostic rule-ID literals present in `crates/splot-validate/src`. A
`cargo xtask check-diagnostic-registry` gate, run as part of `cargo xtask ci`, SHALL extract
the rule-ID literals from non-test, non-comment validator source and compare them against the
IDs documented in the file's enforced registry region. The gate SHALL fail when an emitted ID
is undocumented or when the registry documents an ID that is not present in the source.

#### Scenario: emitted rule ID missing from the registry

- **WHEN** the validator source contains a rule-ID literal that is absent from the registry region
- **THEN** `cargo xtask check-diagnostic-registry` fails and names the undocumented ID

#### Scenario: registry lists an ID not present in source

- **WHEN** the registry region documents a rule ID that does not appear as a literal in non-test validator source
- **THEN** `cargo xtask check-diagnostic-registry` fails and names the unemitted ID

#### Scenario: registry matches the source

- **WHEN** the documented registry IDs equal the rule-ID literals in non-test, non-comment validator source
- **THEN** `cargo xtask check-diagnostic-registry` passes

#### Scenario: registry-only check identifiers are documented

- **WHEN** the validator source contains `Check::id()` registry identifiers (the `<ns>/syntax` literals) that are routed through `syntax_error_diagnostic()` rather than emitted verbatim
- **THEN** those identifiers are documented in a labeled registry sub-table so the documented set still equals the extracted set

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
