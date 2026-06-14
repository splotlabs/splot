# Design: validate-explain-rule

## Context

`explain` must describe any of the ~236 emitted validator diagnostics with data
that is correct, never invented, and never drifts from what the validator actually
emits. `docs/VALIDATOR-DIAGNOSTICS.md` is already the CI-enforced single source of
truth (`check-diagnostic-registry` makes its id set equal the emitted set). The
design makes `explain` a read-only view over a registry **generated from that doc**,
so correctness and freshness are mechanical, not manual.

## Data model / API

- `splot-validate` `explain` module:
  - `pub struct DiagnosticInfo { rule_id: &'static str, severity: Severity,
    spec_section: Option<&'static str>, summary: &'static str }` (`Serialize`).
  - `pub fn explain(&str) -> Option<&'static DiagnosticInfo>` — binary search over
    the sorted table.
  - `pub fn all() -> &'static [DiagnosticInfo]` — for `--list`.
  - `pub fn did_you_mean(&str) -> Vec<&'static str>` — same-namespace hints.
  - `mod generated;` — `pub(super) const REGISTRY: &[DiagnosticInfo]`, generated.
- CLI `commands/explain.rs`: `splot explain [RULE_ID] [--list] [--json]`. Describe →
  exit 0; unknown id / missing arg → `bail!` → exit 2 (clean message, never panic).

## Codegen (`cargo xtask gen-explain [--check]`)

Parses the 4-column emitted-diagnostics tables inside the registry markers of
`docs/VALIDATOR-DIAGNOSTICS.md` — `| `<id>` | severity | § section | condition |` —
into `(rule_id, severity, spec_section, summary)`, sorted by id, and emits
`crates/splot-validate/src/explain/generated.rs`. `--check` regenerates into memory
and diffs against the committed file (wired into `cargo xtask ci`), so the registry
cannot diverge from the doc. The output is rustfmt-stable by construction:
`#[rustfmt::skip]` on the table plus fully-qualified type paths (no `use` items to
reorder), so `--check`'s byte diff is reliable. The 3-column `*/syntax` table is
excluded (its 2nd column is not a severity, so it is skipped naturally).

The generated table embeds the 236 id literals; because they are exactly the
documented ids, `check-diagnostic-registry` (which collects whole single-id string
literals) still sees the same emitted set — no gate change is needed, and condition
prose never matches `is_registry_id`.

## The "long explanation" / spec-honesty

The doc records a one-line `condition` per id, not multi-paragraph prose. `explain`
therefore presents `summary` (= the doc's condition) as the explanation, alongside
the severity and a spec reference (the § section plus pointers to the spec mirror
and the doc). This is faithful to the spec-honesty rule — **nothing is invented**.
Richer per-id long-form prose is a deliberate future enrichment (it would be added
to the doc and flow through automatically), not fabricated here.

## Spec mapping

None — `explain` is a catalog view; the underlying diagnostics already cite their
AV2 sections, which the registry carries verbatim.

## Diagnostics

None — `explain` emits no diagnostics and adds no rule ids.

## Tests

- `xtask/src/explain_registry.rs::tests` — table parsing (4-col only), severity /
  section / summary extraction, quote escaping, deterministic render, id grammar.
- `crates/splot-validate/src/explain::tests` — sorted/unique, catalog floor, known
  / unknown lookup, `did_you_mean` namespace preference.
- `crates/splot-cli/tests/explain_cli.rs` — describe text/JSON snapshots, unknown →
  exit 2 + hint, missing arg → exit 2, `--list` sorted/substantial, `--list --json`.
- `crates/splot-cli/tests/help_snapshots.rs` — `explain --help` golden.
- `cargo xtask gen-explain --check` — drift gate.

## Alternatives considered

- Hand-authored in-crate registry: rejected — 236 entries to maintain, drift risk
  vs the doc, and an invitation to invent prose.
- Parse the doc at runtime (`include_str!`): rejected — couples the library to doc
  formatting at runtime, crosses the package boundary (no precedent), and is more
  fragile than a generated table verified by `--check`.

## Risks

- Spec ambiguity: none (data is copied from the CI-enforced doc).
- Performance: negligible (binary search over a static table).
- Compatibility: additive new subcommand; the router edits are additive (after the
  `decode` registration); the top-level `splot --help` is not snapshotted, so adding
  the subcommand breaks no golden.
- Maintenance: the registry is generated; the `--check` gate keeps it honest.
