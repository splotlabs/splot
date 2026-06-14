# Change: validate-explain-rule

## Feature IDs

- `CLI-VALIDATE-EXPLAIN`

## Why

`splot validate` emits stable rule ids, but a user who sees one (e.g.
`obu-header/global-xlayer-required`) has no in-tool way to learn what it means. The
authoritative catalog already exists — `docs/VALIDATOR-DIAGNOSTICS.md`, CI-enforced
to list exactly the emitted ids — but it isn't reachable from the CLI. `splot
explain <rule-id>` surfaces that catalog (severity, spec section, summary) at the
command line, driven by a registry generated from the doc so it can never drift or
invent.

## Scope

- Spec sections: none (a read-only catalog over existing AV2-grounded diagnostics).
- Crates/modules: `crates/splot-validate/src/explain/` (new `DiagnosticInfo`,
  `explain`/`all`/`did_you_mean`, and the generated `generated.rs` table);
  `xtask/src/explain_registry.rs` (new `cargo xtask gen-explain [--check]`);
  `crates/splot-cli/src/commands/explain.rs` (new) + `main.rs`/`commands/mod.rs`
  (additive router wiring after the `decode` registration).
- CLI/docs/tests: `splot explain <rule-id>` with `--json` / `--list`; README;
  snapshot + behavioral + codegen tests; `explain --help` golden.

## Non-goals

- Does not change any validator behavior, parser semantics, or what diagnostics are
  emitted; `explain` is read-only and reads only what the validator documents.
- Does not hand-author or invent per-id prose: every field is generated from
  `docs/VALIDATOR-DIAGNOSTICS.md` (the CI-enforced single source of truth).
- Does not cover the 13 `*/syntax` registry identifiers (not user-visible
  diagnostics — they route through `bitstream/parse-error`).
- No new runtime dependency (the registry is a generated in-crate table).

## Acceptance criteria

- [ ] Matrix row `CLI-VALIDATE-EXPLAIN` exists.
- [ ] `cargo xtask gen-explain` generates the registry from the doc, and
      `--check` (folded into `cargo xtask ci`) fails on drift.
- [ ] `splot explain <rule-id>` describes a known id (text + `--json`), `--list`
      enumerates all ids, and an unknown id / missing argument is a clean error with
      exit code 2 — never a panic.
- [ ] Snapshot + behavioral + codegen tests ship, plus the `explain --help` golden.
- [ ] `cargo xtask check-feature-status`, `cargo xtask check-diagnostic-registry`,
      and `cargo xtask ci` pass.
