# Change: dupehound-duplication-gate

## Feature IDs

- `INFRA-DUPEHOUND-DUPLICATION-GATE`

## Why

The codebase has accumulated structural code duplication that ordinary review
does not catch: 510 duplicate clusters at integration time, including 47 identical
`into_bytes` header serializers, 23 identical `uvlc` writers, and duplicated CDF
row accessors. Left ungated, agent- and human-written code keeps reimplementing
functions that already exist, and the duplication compounds.

This change integrates [dupehound](https://github.com/Rafaelpta/dupehound) — a
structural duplicate-function detector — as a CI gate, then drives the existing
duplication down. The gate has two complementary modes the maintainer selected:
an **absolute budget** (a ratcheting ceiling on total deletable duplicate lines)
and a **per-PR ratchet** (`check --diff`, blocking newly introduced duplication).
This is repository tooling / CI policy; it adds no AV2 conformance behavior and
implements no decoder/encoder algorithmic stage.

## Scope

- Spec sections: none (infrastructure; a new `tooling` capability, sibling to the
  zero-copy and concurrency runtime policies).
- Crates/modules: `xtask` (new `check-duplication` gate that enforces
  `tools/dupehound/budget.toml` via `dupehound scan --json` (default scope), wired
  into `cargo xtask ci` under the run-if-present policy).
- CLI/docs/tests: `tools/dupehound/budget.toml` (the committed ceiling),
  `.github/workflows/ci.yml` (install dupehound, run the budget gate, and the
  PR-only `check --diff` ratchet), `docs/agents/commands.md` + `AGENTS.md`
  (reuse-before-reimplement guidance), the implementation matrix, and the
  `tooling` capability spec. Gate accept/reject unit tests.
- Subsequent dedup commits lower the production budget (the campaign tracked by
  this change), each scoped and CI-green.

## Non-goals

- No decoder, reconstruction, encoder, or residual algorithm work; no algorithmic
  stage marked implemented.
- No change to AV2 conformance behavior, validator diagnostics, rule IDs, spec
  sections, byte/bit offsets, message text, ordering, or the CLI contract.
- No new Cargo dependency: `dupehound` is an external CLI binary (like `typos`,
  `cargo-machete`, `cargo-deny`), not a workspace crate dependency. `xtask` parses
  its JSON with the `serde_json` it already depends on.
- No behavior-changing test rewrites: dedup preserves exact test semantics and
  keeps `cargo xtask ci` green.

## Acceptance criteria

- [ ] Implementation matrix row `INFRA-DUPEHOUND-DUPLICATION-GATE` exists with proof.
- [ ] `tools/dupehound/budget.toml` records the absolute ceiling and documents the
      ratchet-down discipline.
- [ ] `cargo xtask check-duplication` exists, is deterministic, is unit-tested for
      over/at/under-budget cases, follows the run-if-present policy, and runs
      inside `cargo xtask ci`.
- [ ] `.github/workflows/ci.yml` installs dupehound, runs the budget gate, and
      runs the PR-only `dupehound check --diff <base>` ratchet (base bound through
      env for workflow-injection hygiene).
- [ ] `AGENTS.md` + `docs/agents/commands.md` tell agents to reuse before
      reimplementing and how to run the gate.
- [ ] Positive (at/under budget pass) and negative (over budget rejected) tests exist.
- [ ] `cargo xtask check-feature-status` passes.
- [ ] `cargo xtask ci` passes.
