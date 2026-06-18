## Why

The encoder program needs an explicit contract before implementation resumes, because
the repo now has useful writer and reconstruction building blocks but `splot-encode`
is still an unimplemented API shell. This change records the maintainer-approved
Baseline Encoder Profile v1 scope, the current gaps, and the serialization rules for
future encoder PRs without changing production encoder behavior.

Feature ID: `DOC-ENCODER-PROGRAM-CONTRACT`.

## What Changes

- Add encoder goal, roadmap, and gap-audit documentation for the Baseline Encoder
  Profile v1 program.
- Replace the stale validator-roadmap encoder fence with a scoped encoder carve-out
  that preserves validator ownership.
- Normalize the parked `toy-intra-encoder-v0` status so it is clearly superseded by
  the encoder program contract and must not be resumed directly.
- Record writer, reconstruction, API, dependency, conformance, and active-PR
  baselines for the first encoder flight.
- Add or update only documentation, OpenSpec artifacts, and matrix/status metadata;
  no Rust behavior, dependency graph, RangeEncoder, rate control, recon dependency,
  or public encoder success path changes in this PR.

## Capabilities

### New Capabilities

- `encoder-program`: Documents the Baseline Encoder Profile v1 program contract,
  phase order, evidence gates, non-goals, and first-PR ownership rules.

### Modified Capabilities

- `encoder-api`: Clarifies the future encoder API contract for supported input,
  output, determinism, lifecycle, and bitstream/runtime configuration separation.
- `encoder-tools`: Updates the writer and reconstruction prerequisite contract to
  match the current matrix baseline and known gaps.

## Impact

- Affected docs: `docs/ENCODER-GOAL.md`, `docs/ENCODER-ROADMAP.md`,
  `docs/ENCODER-GAP-AUDIT.md`, `docs/VALIDATOR-ROADMAP.md`, and related generated
  status if the matrix changes.
- Affected OpenSpec artifacts: new `encoder-program-contract` change files and
  deltas for `encoder-program`, `encoder-api`, and `encoder-tools`.
- Affected matrix metadata: one docs-only feature row and refreshed encoder notes
  where needed.
- No code, API behavior, crate manifest, dependency graph, CLI behavior, or external
  codec integration changes.
