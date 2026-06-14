# conformance delta: fixtures-manifest-and-check

Tracks `XTASK-CHECK-FIXTURES`. This adds integrity + outcome discipline for the
hand-crafted `tests/fixtures/` corpus; it does not add AV2 conformance coverage or
change any validator behavior.

## ADDED Requirements

### Requirement: fixture manifest integrity gate

The committed `tests/fixtures/*.av2` corpus SHALL have a `tests/fixtures/MANIFEST.toml`
listing every fixture with a `name`, `path`, `sha256`, `description`, `category`
(`valid` | `validation-error` | `parse-error`), and `expect` (`"clean"` or a
`{ diagnostics = [...] }` error-rule-id set). `cargo xtask check-fixtures` SHALL
verify — hermetically, without running the validator, a decoder, or the network —
that every committed `.av2` fixture is listed exactly once, exists, hashes to its
recorded `sha256`, has a unique `name`/`path`, and a `category` consistent with
`expect`, failing with a problem count otherwise. It SHALL be folded into
`cargo xtask ci` and run as a CI step.

#### Scenario: corpus matches the manifest

- **WHEN** every committed fixture is listed with a matching `sha256` and a
  consistent `category`/`expect`
- **THEN** `cargo xtask check-fixtures` exits zero

#### Scenario: a fixture is mutated, missing, or unlisted

- **WHEN** a fixture's bytes change without updating its `sha256`, a manifest path
  is absent, or a committed `.av2` is not in the manifest
- **THEN** `cargo xtask check-fixtures` prints the offending fixture(s) and exits
  non-zero

### Requirement: fixture outcomes verified against the validator

Each fixture's `expect` SHALL be verified against the real validator
(`splot_validate::Validator::validate_bytes`, in-process — the same entry point as
`splot validate`, no external decoder) so a manifest outcome cannot silently drift.
The verification SHALL be anti-vacuous (the corpus exercises both the `clean` and
`diagnostics` arms) and SHALL fail on any committed fixture absent from the manifest.

#### Scenario: expect matches the validator

- **WHEN** the validator runs over a committed fixture
- **THEN** its error rule-id set equals the manifest's `expect` (or is empty for
  `expect = "clean"`)
