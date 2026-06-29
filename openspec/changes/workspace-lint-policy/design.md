## Context

`clippy::pedantic` gives useful pressure, but the workspace-level allow-list is
necessarily broader than the long-term target. The durable fix is not a single
mass cleanup of casts, wildcard imports, and `must_use_candidate`; it is a
ratchet that prevents the global exception set from growing while allowing
existing debt to be tightened incrementally.

## Design

`cargo xtask check-lint-policy` parses the root manifest and validates
`[workspace.lints.clippy]` directly:

- `clippy::all` and `clippy::pedantic` must remain enabled with lower group
  priority than per-lint overrides.
- `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented`, and `dbg_macro`
  must remain denied.
- A workspace-level `allow` must be in the reviewed baseline inside
  `xtask/src/lint_policy.rs`.
- Removing or tightening an existing broad allow is accepted without editing the
  baseline.

## Alternatives Considered

- Re-enable the broad families immediately: rejected because the resulting churn
  would touch unrelated codec transcription modules and obscure the policy
  improvement.
- Move every exception to module scope immediately: rejected for the same reason;
  this change creates the guardrail that makes scoped cleanup safe to do
  gradually.

## Risks

- The check is manifest-level, not semantic source analysis. It prevents new
  global blind spots but does not prove every existing local `#[allow]` is ideal.
- Future work still needs crate/module-level cleanup for casts, public API
  `must_use`, and wildcard-import hubs.
