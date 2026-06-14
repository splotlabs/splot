# Change: docs-validator-examples

## Feature IDs

- `DOC-VALIDATOR-EXAMPLES`

## Why

Items #2–#5 of the validator productization work added user-facing surface — the
`--max-diagnostics` / `--summary-only` output controls on `splot validate` and the
new `splot explain` command — but the README documents them only as one-line
entries in the quick-start cheatsheet. A reader cannot see what `explain` actually
prints, how the truncation notice reads, or how `validate` and `explain` compose
(get a `rule_id`, look it up). This change turns the newest validator surface into
worked examples whose output is copied verbatim from the shipped binary.

## Scope

- Docs only: `README.md`. No code, no behavior, no new flags.
- Adds worked `splot explain` examples (describe, unknown-id hint, `--json` /
  `--list`) and a worked output-controls example (`--max-diagnostics`,
  `--summary-only`), plus a Status-table row for the now-shipping `explain` command.
- Every example's output is real (captured from `cargo run -p splot-cli` against a
  committed fixture or the built-in registry), not hand-written.

## Non-goals

- No change to any CLI behavior, flag, exit code, or diagnostic.
- No new doc files (`docs/FIXTURES.md` already shipped in item #3); no CHANGELOG
  (deliberately absent — the matrix, PRs, and git history are canonical).
- No invented output: examples must match the binary, so the doc cannot drift into
  describing behavior that does not exist.

## Acceptance criteria

- [ ] Matrix row `DOC-VALIDATOR-EXAMPLES` exists.
- [ ] README documents `splot explain` with a worked example (text), and shows the
      unknown-id hint, `--json`, and `--list`.
- [ ] README documents `--max-diagnostics` and `--summary-only` with a worked
      example showing the truncation notice and the full-report counts/exit code.
- [ ] Every example's output matches the current `splot` binary.
- [ ] `cargo xtask ci` is green; the conflict-zone denylist is untouched.
