# encoder-tools delta: writer-coverage-doc

## ADDED Requirements

### Requirement: generated writer coverage document

The `xtask` automation SHALL render a writer coverage document from
`docs/IMPLEMENTATION-MATRIX.toml` — one row per writable feature (every `bitstream-syntax` feature and
every feature with a landed writer), with its spec section(s), feature id, name, `write` maturity, and
module — via a `cargo xtask writer-coverage` subcommand, and `check-feature-status` SHALL regenerate and
compare `docs/spec-coverage-writer.md` so it can never drift from the matrix, exactly as it already
guards the sibling `docs/FEATURE-STATUS.md` and `docs/SPEC-COVERAGE.md`.

#### Scenario: the writer coverage doc is generated and drift-guarded

- **WHEN** `cargo xtask writer-coverage --format markdown --output docs/spec-coverage-writer.md` is run
- **THEN** it SHALL write a deterministic document listing the writable features with their `write`
  status, and a subsequent `cargo xtask check-feature-status` SHALL pass; an out-of-date
  `docs/spec-coverage-writer.md` SHALL make `check-feature-status` fail with the regenerate command.
