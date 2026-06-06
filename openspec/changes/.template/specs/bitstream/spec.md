# bitstream delta: <change-id>

Describe the change to the bitstream capability as additions/modifications/removals
of requirements. Cite the AV2 v1.0.0 section for every normative requirement and
reference the Feature ID(s) it advances. Each requirement needs at least one
`#### Scenario:` with `- **WHEN**` / `- **THEN**` bullets, or `openspec validate`
will reject it.

## ADDED Requirements

### Requirement: <short title>

The parser SHALL ... (AV2 v1.0.0 § <section>).

#### Scenario: positive case

- **WHEN** a conformant input is parsed
- **THEN** the expected structured result is produced

#### Scenario: malformed case

- **WHEN** a truncated/invalid input is parsed
- **THEN** a typed `Error` (never a panic) is returned

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
