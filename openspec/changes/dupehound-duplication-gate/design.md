# Design: dupehound-duplication-gate

## Context

dupehound detects near-duplicate functions by fingerprinting normalized syntax
(tree-sitter + winnowing), so it catches copies even after identifiers and
literals are renamed. The maintainer chose to gate **both** ways — an absolute
budget and a per-PR ratchet — and to remove the existing duplication.

## Data model / API

- `tools/dupehound/budget.toml`: `max_deletable_lines: u64` — the committed
  absolute ceiling. A ratchet: lowered when duplication is removed, never raised.
- `xtask::dupehound`:
  - `check_duplication(root)` — run-if-present; runs `dupehound scan <root>
    --json` (default scope), deserializes `score.deletable_lines`, and delegates
    to `enforce_budget`.
  - `enforce_budget(actual, ceiling) -> Result<String>` — pure threshold logic
    (no I/O), unit-tested: errors when `actual > ceiling`; otherwise returns an
    `ok` message that nudges the ratchet down when under budget.
- `Task::CheckDuplication` clap subcommand; `check_duplication` also runs in
  `run_ci()` after the other repository gates.

## Why a ceiling, not strict equality

The gate fails only when `actual > ceiling`, mirroring the coverage gate's
`--fail-under-lines` ceiling. Strict equality would make any unrelated PR that
incidentally removes a duplicate fail until the budget is hand-edited, and would
be brittle across platforms if dupehound's count differs by one. A ceiling plus
hand-ratcheting (lower the budget in the dedup commit) is robust and still
enforces monotonic improvement. The per-PR `check --diff` ratchet independently
blocks *newly introduced* duplication, so a ceiling-only aggregate gate is not a
loophole.

## Why the default scope (not `--include-tests`)

`dupehound scan` excludes the bodies of `#[test]` functions from the slop score
by design, because per-scenario test cases are usually intentionally explicit —
exactly this repo's testing philosophy, where each named test documents one spec
scenario. The gate therefore uses the **default scope** and measures *production*
duplication (plus non-`#[test]` test helpers): the duplication that actually
costs maintainability and is worth removing. The budget is a ratchet on that
production duplication, not a mandate to reach zero — a real codebase always
carries legitimate duplication (explicit tests, deliberate parser/writer
decoupling, `&self`/`&mut self` accessor pairs). An earlier revision gated with
`--include-tests`, which forced the gate to count the deliberately-explicit test
cases and created an unwinnable, unmaintainable target; the default scope
corrects that.

## CI wiring

- `dupehound` is installed via `cargo install dupehound@0.1.2 --locked` (pinned for
  a reproducible count), like the prebuilt `typos`/`cargo-machete` step.
- The budget gate runs as `cargo xtask check-duplication` among the other
  `cargo xtask check-*` steps.
- The PR ratchet `dupehound check --diff "$BASE_SHA" .` runs only on
  `pull_request`; the base SHA is bound through `env` (workflow-injection
  hygiene), and `fetch-depth: 0` gives the merge-base full history.

## Spec mapping

None. This is non-normative repository tooling; it adds no AV2 conformance
behavior. Captured as the `tooling` capability, sibling to the zero-copy and
concurrency runtime policies.

## Diagnostics

None (no validator diagnostics; this is a build-time CI gate, not a bitstream
finding).

## Tests

- `xtask/src/dupehound.rs::tests`: over-budget rejected, at-budget passes,
  under-budget passes + nudges, zero-at-zero passes.
- `cargo xtask check-duplication` against the committed budget (CI + local).

## Alternatives considered

- Ratchet-only (`check --diff`): rejected — the maintainer chose Both; an absolute
  budget also catches aggregate regressions a per-PR diff can miss.
- Strict-equality budget: rejected — brittle and noisy (see above).
- A new Cargo dependency / library: rejected — dupehound is an external CLI, used
  exactly like the existing `typos`/`cargo-machete`/`cargo-deny` tools.

## Risks

- Spec ambiguity: none (non-normative).
- Performance: scan is ~0.25s over the workspace; negligible CI cost beyond the
  one-time dupehound install.
- Compatibility: the count is pinned to dupehound 0.1.2; a version bump may shift
  the count and require a one-line budget update.
- Maintenance: the budget must be lowered as clusters are removed; the gate prints
  the headroom and the exact value to set, and `AGENTS.md` records the discipline.
