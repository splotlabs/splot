# Change: inspect-snapshot-tests

## Feature IDs

- `CONF-INSPECT-SNAPSHOTS`

## Why

`splot inspect --json` already emits a structured, fully-deterministic per-OBU summary
(byte offsets, sizes, parsed fields — no paths, timestamps, or filenames), but its behavior
is not frozen by any golden test (`CONF-INSPECT-SNAPSHOTS`, `tests=todo`). A regression or
intended change to the inspector output for a committed fixture would go unnoticed. Adding
`insta` golden snapshots over a diverse set of committed fixtures freezes the inspector and
surfaces any output change as a reviewable snapshot diff — guarding every future inspect
change.

## Scope

- Crates/modules: `crates/splot-cli/tests/inspect_snapshots.rs` (new), snapshots in
  `crates/splot-cli/tests/snapshots/`. `insta` added as a dev-only workspace dependency.
- Fixtures: a representative subset of `tests/fixtures/*.av2` covering diverse OBU types
  (sequence header, OPS, film grain, buffer-removal timing, metadata short/group, frame
  header + MFH + prefix).

## Non-goals

- No change to the inspector itself — this is a test-harness gap only (the output exists;
  this freezes it).
- No new validator diagnostics or parser behavior.

## Acceptance criteria

- [ ] `insta` is a dev-dependency (workspace), passing cargo-deny (licenses/bans/sources).
- [ ] `crates/splot-cli/tests/inspect_snapshots.rs` snapshots `splot inspect --json` for the
      committed fixtures; the `.snap` files are committed.
- [ ] The snapshots pass deterministically on re-run (`cargo test -p splot-cli --test inspect_snapshots`).
- [ ] `CONF-INSPECT-SNAPSHOTS` `tests` stage is `done`, with proof recorded.
- [ ] `cargo xtask ci` passes.
