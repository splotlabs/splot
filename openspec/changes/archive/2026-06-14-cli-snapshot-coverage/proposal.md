# Change: cli-snapshot-coverage

## Feature IDs

- `CONF-CLI-SNAPSHOT-COVERAGE`

## Why

`CONF-INSPECT-SNAPSHOTS` freezes the `splot inspect --json` surface, but two
user-facing CLI surfaces have no snapshot coverage: the `inspect` **human (text)**
output (the default per-OBU dump and `--headers`), and the `validate` / `inspect`
`--help` text. As the validator productization work starts adding flags
(`--max-diagnostics`, `--summary-only`, `explain`, …), a frozen `--help` snapshot
is the backward-compatibility tripwire required by the mission's definition of
done: any change to the argument surface shows up as a reviewable diff, and
additive flags update the snapshot intentionally. Pinning the text dump closes the
remaining gap in inspector output coverage.

## Scope

- Spec sections: none (CLI output/contract coverage, not AV2 syntax).
- Crates/modules: tests only — `crates/splot-cli/tests/help_snapshots.rs` (new),
  `crates/splot-cli/tests/inspect_text_snapshots.rs` (new), and their committed
  `.snap` goldens. No production code changes.
- CLI/docs/tests: extends the snapshot test layer; no new flags or behavior.

## Non-goals

- Does not add or change any CLI flag, diagnostic, or validator/inspector behavior.
- Does not snapshot the top-level `splot --help` (keeps the validator stream
  decoupled from the `decode`/`encode` subcommand wording).
- Does not snapshot `--version` (a moving value).

## Acceptance criteria

- [ ] Matrix row `CONF-CLI-SNAPSHOT-COVERAGE` exists.
- [ ] `validate --help` and `inspect --help` are frozen as `insta` goldens.
- [ ] `inspect` default and `--headers` text output is frozen for representative
      committed fixtures.
- [ ] Snapshots are deterministic (no paths/timestamps/version strings) and pass
      against committed goldens.
- [ ] `cargo xtask check-feature-status` and `cargo xtask ci` pass.
