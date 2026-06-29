## ADDED Requirements

### Requirement: Workspace Clippy allow-list ratchet

The repository SHALL provide `cargo xtask check-lint-policy`, tracked by
`XTASK-LINT-POLICY`, to validate that workspace Clippy lint policy keeps
`clippy::all` and `clippy::pedantic` enabled at workspace scope, keeps
high-signal development lints denied, and rejects any workspace-level
`clippy::<lint> = "allow"` not listed in the reviewed lint-policy baseline.
The check SHALL permit existing broad exceptions to be removed or moved to
narrower scopes without requiring a matching baseline entry.

#### Scenario: new global Clippy allow is added

- **WHEN** a contributor adds a workspace-level Clippy `allow` that is not in the
  lint-policy baseline
- **THEN** `cargo xtask check-lint-policy` fails with the offending lint name
- **AND** the contributor must either move the exception to a narrower scope or
  update the lint-policy baseline with reviewable rationale

#### Scenario: existing broad exception is tightened

- **WHEN** a contributor removes an existing workspace-level Clippy `allow` or
  changes it to `warn` or `deny`
- **THEN** `cargo xtask check-lint-policy` accepts the tighter policy

#### Scenario: required strict lints are weakened

- **WHEN** a contributor weakens the workspace-level denial of `unwrap_used`,
  `expect_used`, `panic`, `todo`, `unimplemented`, or `dbg_macro`
- **THEN** `cargo xtask check-lint-policy` fails
