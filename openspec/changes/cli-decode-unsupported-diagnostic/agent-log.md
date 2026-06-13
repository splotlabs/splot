# Agent Log: cli-decode-unsupported-diagnostic

## Orchestrator

Objective: implement the next decoder mission item that does not require an
unapproved dependency-graph change. Scope is the existing `splot decode` CLI
entry point: replace the generic stub with a structured unsupported-feature
diagnostic and proof tests. No decoder crate, reconstruction crate, or
dependency graph change is part of this change.

Baseline: `origin/main` at `30dad3ab2693940ede6ad84c97b3805f447d6d40`
(`chore(xtask): add decoder support drift gate`).

## Planning Subagents

### @architect

Agent id: `019ec0be-0084-77a0-b8e5-2fc32231325c`

Findings:

- Implement this as a CLI-only unsupported diagnostic change.
- Touch `crates/splot-cli/src/commands/decode.rs`,
  `crates/splot-cli/tests/cli.rs`, decoder support matrix/status,
  implementation matrix, and generated feature/spec status.
- Keep the support row `unsupported-intentional`; prove the diagnostic
  contract without claiming real decode support.
- Do not add `splot-decode`, `splot-recon`, dependencies, or validator
  diagnostic registry entries.

### @spec-reader

Agent id: `019ec0be-034e-7b83-9ca6-6b193028b9bf`

Findings:

- The unsupported CLI diagnostic should cite `decode/unsupported-feature`,
  severity `error`, AV2 § 7.1, Feature ID `CLI-DECODE`, and matrix row
  `cli-decode-entrypoint`.
- Do not cite deeper decode sections such as § 5.20, § 7.13, or § 8.2 until
  code actually reaches those stages.
- Decoder diagnostic JSON should deliberately use the roadmap contract fields
  (`code`, lowercase `severity`, `matrix_row`, `feature_id`, `remediation`)
  rather than validator JSON's `rule_id` shape.

### @api-designer

Agent id: `019ec0be-05f4-7793-b97b-96b2c9ed5f96`

Findings:

- Add only `--json`; keep `splot decode [--json] INPUT -o OUTPUT`.
- Use CLI-local serializable structs rather than `splot_validate::Diagnostic`.
- Add text and JSON CLI tests and update the decoder support matrix proof.
- Suggested reading input first for command consistency, but this was not
  adopted for this unsupported-only phase because the security review identified
  unnecessary file-system risk before any supported decode path exists.

### @security-reviewer

Agent id: `019ec0be-0931-7ce2-8197-8155f712a396`

Findings:

- Return the unsupported diagnostic before any input read or output open.
- Do not call `read_input()` for this change.
- Do not create or truncate `--output`; opening user-controlled output before
  unsupported decode could clobber files or follow symlinks.
- Do not add `std::process::Command`, wrappers, `xtask`, CI, or dependencies
  for AVM/dav2d.
- Pin JSON/text output with tests and prove missing input still returns the
  unsupported diagnostic rather than an operational file error.

Decision: follow the security recommendation. Until `splot decode` has a
supported byte-consuming path, the command parses CLI arguments only, emits the
structured unsupported diagnostic, and exits `1`.

## Local Reference Boundary

No AVM or dav2d command was run for this change. No AVM/dav2d source, snippets,
binaries, submodules, dependencies, build probes, wrappers, CI jobs, required
scripts, required `xtask` commands, or mandatory tests are proposed.

## Implementation

Implemented in:

- `crates/splot-cli/src/commands/decode.rs`
- `crates/splot-cli/tests/cli.rs`
- `docs/DECODER-SUPPORT-MATRIX.toml`
- `docs/DECODER-ROADMAP.md`
- `docs/IMPLEMENTATION-MATRIX.toml`
- Generated status docs:
  `docs/DECODER-SUPPORT-STATUS.md`, `docs/FEATURE-STATUS.md`,
  `docs/SPEC-COVERAGE.md`

The CLI entry point now emits a structured unsupported diagnostic with stable
text and JSON forms. The command does not read input, create or truncate output,
invoke external tools, or change the crate dependency graph.

## Local Verification

Passed:

- `openspec validate cli-decode-unsupported-diagnostic --strict`
- `cargo test -p splot-cli --test cli decode_unsupported --locked`
- `cargo xtask check-decoder-support`
- `cargo xtask check-feature-status`
- `git diff --check`
- `cargo xtask ci`

Rebase note: after `origin/main` was refreshed, `feat/cli-decode-unsupported-diagnostic`
was checked against `origin/main` at `30dad3ab2693940ede6ad84c97b3805f447d6d40`.
Git reported the branch was already up to date; the in-flight worktree changes
were restored from a temporary stash without conflicts.

## Final Review Sign-offs

### General code/test review

Agent id: `019ec0c9-07b9-71d3-a8a9-adc37896d1e1`

Result: no code correctness or regression issues. The only finding was that
task 4.6 remained unchecked before final sign-off recording.

### Security review

Agent id: `019ec0c9-0a7e-70b1-9ef9-aa2861a69616`

Result: no actionable security findings. The review confirmed `splot decode`
returns the unsupported diagnostic before input/output file handling, does not
call the shared `read_input()` helper, does not invoke external decoders or
processes, and has tests proving output preservation and no missing-path
creation.

### AV2/spec/status review

Agent id: `019ec0c9-0d26-7fd0-a723-f938e99ab40a`

Result: no findings. The diagnostic fields match the requested
`decode/unsupported-feature` contract with severity `error`, AV2 § 7.1,
Feature ID `CLI-DECODE`, and matrix row `cli-decode-entrypoint`; the docs keep
the row `unsupported-intentional` and do not cite deeper decode sections.

### Dependency and encoder-boundary review

Agent id: `019ec0c9-100e-72c3-9ca0-dd1b4e0adfcc`

Result: no actionable findings. The review confirmed no dependency graph,
encoder-facing crate, `xtask`, CI/workflow, manifest, AVM/dav2d, or external
decoder integration changes were introduced.
