# Change: conformance-corpus-foundation

## Feature IDs

- `CONF-AVM-VALID-STREAMS` (the committed valid-vector corpus + runner)
- `CONF-AVM-DIFF-HARNESS` (reframed: AVM is a LOCAL oracle/generator, not a
  committed dependency)

## Why

The mission DoD calls for a committed conformance corpus under
`tests/conformance/` with a manifest and scripted generation, but today
`tests/conformance/` does not exist and `cargo xtask conformance` is a stub.
The existing `avm-differential-harness` change proposed a *live* harness that
shells out to a local AVM checkout — but per the maintainer decision, **AVM is
a local oracle/generator only and must never be a committed dependency** (no
AVM in build or CI). AVM-generated artifacts (small AV2 bitstreams) MAY be
committed as plain project fixtures.

This change lays the foundation: a committed, self-contained corpus of small
AVM-generated valid AV2 streams plus a runner that validates them against a
manifest of expected outcomes — with **no AVM dependency in the committed
runner, build, or CI**. AVM's role (encoding the vectors) stays a documented
local oracle step.

## Scope

- New `tests/conformance/` corpus: small AVM-generated valid AV2 vectors
  (`.ivf`) under `vectors/valid/`, plus a `manifest.toml` mapping each vector
  to its expected validation outcome (`clean`, or a set of expected diagnostic
  `rule_id`s for future negatives).
- A committed runner — `cargo xtask conformance` and/or a `splot-validate`
  integration test — that loads the manifest, runs the validator on each
  committed vector, and asserts the expected outcome. **No AVM invocation; no
  network; runs in CI.**
- `.gitignore`: allow committed corpus vectors under `tests/conformance/`
  (the `*.av2` / `*.obu` ignore currently only excepts `tests/fixtures/`).
- Lift the `docs/VALIDATOR-ROADMAP.md` "do not start yet" fence for the
  conformance corpus only (the encoder/writer/decoder stay fenced).
- Reshape `docs/CONFORMANCE.md` and the `CONF-AVM-VALID-STREAMS` /
  `CONF-AVM-DIFF-HARNESS` matrix rows to the committed-corpus + local-oracle
  design.

## Non-goals

- No AVM source vendored; no AVM in build, CI, or any committed runner path.
- No negative/mutated vectors yet (the `conformance-negative-mutator` change).
- No broad positive-vector sweep (the `avm-positive-vector-generation`
  change); this lands the foundation + a small bootstrap set.
- No network access in CI.

## Acceptance criteria

- [ ] `tests/conformance/` holds committed AVM-generated valid `.ivf` vectors
  and a `manifest.toml`; the vectors are tracked (not gitignored).
- [ ] A committed runner validates every manifest vector and asserts its
  expected outcome, with NO AVM dependency, and is exercised by CI.
- [ ] The roadmap fence is lifted for the corpus only; `CONF-AVM-VALID-STREAMS`
  records proof; `docs/CONFORMANCE.md` reflects the local-oracle design.
- [ ] `cargo xtask ci` green.
