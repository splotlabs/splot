# process delta: ci-quality-gates

Advances `XTASK-CI-QUALITY-GATES`. Adds two repository process guarantees — a
strict documentation build gate and a blocking validator coverage threshold —
and brings the local gate, the CI workflow, and the in-repo descriptions of the
gates into agreement. No AV2 syntax change.

## ADDED Requirements

### Requirement: strict documentation build gate

The repository SHALL build rustdoc documentation for the whole workspace with
warnings denied (`RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps
--locked`) as a blocking step in both `cargo xtask ci` and the CI `ci` job. A
rustdoc warning or error in any workspace crate SHALL fail the gate.

#### Scenario: rustdoc warning blocks the gate

- **WHEN** a workspace crate contains a doc comment that rustdoc reports on
  (for example an unresolved or private intra-doc link)
- **THEN** `cargo xtask ci` and the CI `ci` job fail at the docs step

#### Scenario: clean docs pass the gate

- **WHEN** `cargo doc --workspace --no-deps --locked` emits no warnings under
  `RUSTDOCFLAGS=-D warnings`
- **THEN** the docs step passes in both `cargo xtask ci` and the CI `ci` job

### Requirement: blocking validator coverage threshold

CI SHALL measure workspace line coverage with `cargo llvm-cov` and SHALL fail
the coverage job when line coverage over the `crates/splot-validate` sources,
in isolation, is below 90 percent. The job SHALL NOT be marked
`continue-on-error`. The workspace-wide summary and the lcov artifact SHALL
continue to be produced. `cargo xtask coverage` SHALL enforce the same
threshold locally when `cargo-llvm-cov` is installed.

#### Scenario: validator coverage regression blocks the merge

- **WHEN** a change drops `crates/splot-validate` line coverage below 90
  percent
- **THEN** the CI coverage job fails and the PR cannot merge

#### Scenario: other crates do not gate

- **WHEN** line coverage outside `crates/splot-validate` changes
- **THEN** the threshold check is unaffected (only `splot-validate` files are
  in the gated report scope)

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
