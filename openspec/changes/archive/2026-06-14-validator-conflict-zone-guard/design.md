# Design: validator-conflict-zone-guard

## Context

The validator productization stream and the decoder stream develop against the
same `main` in parallel. The hard constraint is that validator-stream changes
never touch decoder-owned files. A path-based, merge-base-relative gate makes that
constraint mechanical and self-documenting (each PR can paste an empty result),
without a new CI workflow file and without any runtime dependency on the decoder
crates.

## Data model / API

New module `xtask/src/conflict_zone.rs` (standalone; depends only on
`crate::git_util::run_git`, already `pub(crate)`):

- `const FORBIDDEN_PREFIXES: &[&str]` — decoder-owned directory trees / path
  prefixes: `crates/splot-decode/`, `crates/splot-recon/`, `docs/DECODER-`,
  `fuzz/fuzz_targets/decode`, `crates/splot-cli/tests/decode` (the decode CLI
  tests, e.g. `decode_cli.rs`).
- `const FORBIDDEN_EXACT: &[&str]` — `docs/LOCAL-REFERENCE-EVIDENCE.toml`,
  `crates/splot-cli/src/commands/decode.rs`.
- `const AVM_SCAN_ROOTS: &[&str]` — code/build roots under which a new
  `avm`/`dav2d` path is an integration attempt: `crates/`, `scripts/`, `tools/`,
  `fuzz/`, `xtask/`, `.github/`.
- `fn is_forbidden(path: &str) -> Option<&'static str>` — pure classifier
  (prefix / exact / scoped AVM-token match). The single unit-tested core.
- `fn mentions_avm_or_dav2d(path) -> bool` — tokenizes each path segment on
  non-alphanumeric chars and matches the tokens `avm` / `dav2d` exactly, so `av2`
  (the codec name) never matches and `avm` only matters under a scan root.
- `pub(crate) fn check_conflict_zone(root: &Path) -> Result<()>` — resolve base,
  diff, classify, report.

Base resolution: try `origin/main`, then `main`, then `FETCH_HEAD`; compute
`merge-base <ref> HEAD` and diff `merge-base..HEAD` with
`git diff --name-only` (three-dot `main...HEAD` semantics). Reporting mirrors
`check_spec_mirror` / `check_license_headers`: accumulate offenders, `eprintln!`
each, `bail!` with a count; print `check-conflict-zone: ok (...)` on success.

## Decoder-safety (the key design decision)

A committed guard in shared `cargo xtask ci` / CI would, by construction, also
fire on the decoder stream's own legitimate decoder-file edits, and a permanent
"never edit `splot-decode`" gate on `main` would be wrong long-term. The guard is
therefore scoped to the validator stream and skips (returns `Ok` with a notice)
when:

- no `main` base is resolvable (fresh/shallow clone), or the diff is empty
  (on `main`, or a branch with no commits ahead);
- the branch is a decoder-stream branch — its name carries a `decode`/`recon`
  *token* (whole-token match, so `fix/reconcile-…` is not falsely exempted);
- `SPLOT_SKIP_CONFLICT_ZONE=1` is set (explicit escape hatch for any legitimate
  conflict-zone edit on a non-decoder-named branch).

The branch name is resolved from `SPLOT_PR_HEAD_REF` first, then the local branch.
A `pull_request` checkout is a detached HEAD where the branch is not locally
derivable, so the CI step passes `SPLOT_PR_HEAD_REF: ${{ github.head_ref }}` (the
safe `env:` pattern, not interpolated into a shell command) and always runs — the
guard's own tokenized logic, identical in CI and locally, decides the exemption.
This keeps the gate enforcing on validator PRs while never breaking the decoder
stream's gate. The convention (decoder-stream branches carry a `decode`/`recon`
token) is documented in `AGENTS.md`; any other branch legitimately editing a
conflict-zone path sets `SPLOT_SKIP_CONFLICT_ZONE=1`.

Robustness notes: the diff uses `--no-renames` so a decoder file renamed *out* of
the zone still surfaces its deleted path, and `core.quotepath=false` so non-ASCII
paths are matched raw rather than C-quoted.

## Spec mapping

None — this is project automation, not AV2 syntax or semantics.

## Diagnostics

None — the guard is an xtask gate using `anyhow::bail!`, not a validator
`Diagnostic`. It adds no rule IDs.

## Tests

- `xtask/src/conflict_zone.rs::tests` — `is_forbidden` returns `Some` for
  `crates/splot-decode/**`, `crates/splot-recon/**`, `docs/DECODER-*`,
  `docs/LOCAL-REFERENCE-EVIDENCE.toml`, `crates/splot-cli/src/commands/decode.rs`,
  `fuzz/fuzz_targets/decode*`, `scripts/*dav2d*`, `crates/**/avm*`; and `None` for
  validator/shared paths, `fuzz/fuzz_targets/parse_obu.rs`,
  `docs/spec/av2/1.0.0/index.md` (`av2` ≠ `avm`), and
  `openspec/changes/avm-differential-harness/proposal.md` (outside the scan roots).

## Alternatives considered

- Alternative: reuse `audit_scope::changed_paths_from_base` + its private glob
  matcher. Why rejected: those helpers are `fn`-private to `audit_scope`; bumping
  their visibility edits a file the decoder stream may also touch, and the matcher
  has no `**` support. A small self-contained module is lower-conflict.
- Alternative: a permanent unconditional gate. Why rejected: it would break the
  decoder stream and forbid all future decoder edits via `cargo xtask ci`.

## Risks

- Spec ambiguity: none.
- Performance: negligible (one `git diff`, string matching).
- Compatibility: decoder stream is explicitly exempt; no behavior change to any
  existing command.
- Maintenance: the denylist must track the conflict zone; it lives in one
  documented const next to its tests.
