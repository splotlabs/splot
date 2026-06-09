# Change: diagnostic-registry-enforcement

## Feature IDs

- `XTASK-DIAGNOSTIC-REGISTRY`

## Why

`docs/VALIDATOR-DIAGNOSTICS.md` is meant to be the canonical registry of validator
diagnostic rule IDs, but nothing kept it in sync with the IDs the validator actually
emits. The planning sections used design-stage names that diverged from shipped IDs
(`quant-matrix/quant-delta-out-of-range` vs emitted `qm/quant-delta-out-of-range`,
`ops/embedded-op-index-out-of-range` vs emitted `ops/inherited-op-index-out-of-range`),
so downstream readers and tooling could copy stale IDs. The existing
`XTASK-FEATURE-STATUS` scan only checks diagnostic *prefixes*, not full IDs.

This change makes the registry complete and machine-enforced: a marker-delimited region of
`docs/VALIDATOR-DIAGNOSTICS.md` must list exactly the diagnostic rule-ID literals present in
`crates/splot-validate/src`, enforced by a new `cargo xtask check-diagnostic-registry` gate
wired into `cargo xtask ci`.

## Scope

- Spec sections: none (tooling / process guarantee; no AV2 syntax change).
- Crates/modules: `xtask/src/diagnostic_registry.rs` (new), `xtask/src/main.rs`
  (subcommand + `run_ci` wiring), reusing helpers from `xtask/src/feature_status.rs`.
- CLI/docs/tests: `cargo xtask check-diagnostic-registry`;
  `docs/VALIDATOR-DIAGNOSTICS.md` rewritten with the marker-delimited registry; unit tests
  in the new module.

## Non-goals

- Machine-checking each diagnostic's `severity` and `spec_section` (the registry documents
  them, but the check enforces only the rule-ID set in v1).
- Changing any emitted diagnostic, rule ID, or validator behavior.
- Replacing the prefix-level `scan_diagnostics` check in `XTASK-FEATURE-STATUS` (kept as a
  complementary, coarser guard).

## Acceptance criteria

- [x] Implementation matrix row `XTASK-DIAGNOSTIC-REGISTRY` exists.
- [x] `cargo xtask check-diagnostic-registry` passes against the rewritten doc.
- [x] The check is wired into `cargo xtask ci` and fails on drift in either direction.
- [x] Registry-only `Check::id()` literals (the `<ns>/syntax` IDs) are documented and labeled.
- [x] Positive and negative unit tests exist (extractor, comment/test exclusion, marker parser, check).
- [x] `cargo xtask check-feature-status` passes.
