# process delta: validator-conflict-zone-guard

Tracks `XTASK-CONFLICT-ZONE-GUARD`. This is a project-automation gate and does not
add AV2 conformance coverage.

## ADDED Requirements

### Requirement: conflict-zone guard

The workspace SHALL provide a `cargo xtask check-conflict-zone` command that
compares the working branch's committed diff against `main` (merge-base relative)
to a committed denylist of decoder-owned paths, and SHALL exit non-zero when any
changed path falls inside the denylist. The denylist SHALL cover
`crates/splot-decode/**`, `crates/splot-recon/**`, `docs/DECODER-*`,
`docs/LOCAL-REFERENCE-EVIDENCE.toml`, `fuzz/fuzz_targets/decode*`,
`crates/splot-cli/src/commands/decode.rs`, `crates/splot-cli/tests/decode*`,
and new AVM/dav2d integration paths
under the workspace code/build roots. The command SHALL be folded into
`cargo xtask ci` and run as a step in CI.

#### Scenario: a validator change stays clear of the conflict zone

- **WHEN** the diff vs `main` touches only validator/inspector/tooling files
- **THEN** `cargo xtask check-conflict-zone` exits zero with an `ok` notice

#### Scenario: a change touches a decoder-owned path

- **WHEN** the diff vs `main` creates, edits, or deletes any denylisted path
- **THEN** `cargo xtask check-conflict-zone` prints each offending path and exits
  non-zero

### Requirement: conflict-zone guard is decoder-safe

The guard SHALL NOT break the decoder stream or fail spuriously. It SHALL skip
with a notice (returning success) when no `main` base is resolvable, when the diff
is empty, when the current branch is a decoder-stream branch (its name carries a
`decode`/`recon`-family name token, matched whole-token, not by bare substring),
or when `SPLOT_SKIP_CONFLICT_ZONE=1` is set.

#### Scenario: decoder-stream branch is exempt

- **WHEN** the guard runs on a branch whose name carries a `decode`/`recon`-family
  token (e.g. `decode`, `decoder`, `recon`, `reconstruct`)
- **THEN** it skips with a notice and exits zero without inspecting the diff

#### Scenario: no base to compare against

- **WHEN** no `main` base is resolvable (e.g. a shallow clone) or the diff is empty
- **THEN** the guard skips with a notice and exits zero
