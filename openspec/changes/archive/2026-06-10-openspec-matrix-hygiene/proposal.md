# Proposal: OpenSpec hygiene — fix failing main specs, gate validation, park encoder changes

## Feature IDs

- `XTASK-CI-QUALITY-GATES` (completes the openspec-validation parity its change
  deferred)
- `ENC-BITSTREAM-WRITER`, `ENC-INTRA-TOY-V0` (parking their OpenSpec changes;
  no stage change)

## Why

Three hygiene problems block trusting the OpenSpec surface:

1. **The documented validation command fails on main.**
   `openspec validate --all --no-interactive` — the exact command the CI
   workflow runs when `openspec` is installed — fails on two main specs:
   `spec/bitstream` (requirement 40, "New frame parsers never panic") and
   `spec/validator` (requirements 46 and 50, the frame-QM-reference and
   MFH-layer-dependency requirements). All three requirements contain SHALL,
   but the OpenSpec CLI only inspects the requirement body's first line, and
   in all three the keyword sits on a wrapped continuation line. Anyone with
   `openspec` installed gets a red validation on a clean checkout.
2. **`cargo xtask ci` skips OpenSpec validation entirely.** The
   `ci-quality-gates` change explicitly deferred adding it until the main-spec
   failures were fixed (its Non-goals). With (1) fixed, the local gate can run
   `openspec validate --all --no-interactive` under the same run-if-present
   policy as the other external tools, closing the last xtask-vs-CI parity
   gap.
3. **Two bootstrap-era encoder changes read as active work.**
   `add-bitstream-writer` and `toy-intra-encoder-v0` are encoder-track stubs
   (0 tasks done, no design.md, pre-dating the current change conventions)
   behind the explicit "do not start yet" fence in
   `docs/VALIDATOR-ROADMAP.md`. Their "proposed" state in the Active-changes
   table misrepresents them as next-up work during a validator-first phase.

Explicitly **not** a problem: five matrix rows reference OpenSpec change ids
with no folder (`encoder-input-model`, `rate-control-v0`,
`encoder-speed-presets`, `fetch-public-vectors`, `inspect-snapshot-tests`).
`docs/IMPLEMENTATION-MATRIX.schema.md` § 3 expressly allows *planned* change
ids without folders, so these are schema-legal recorded intent and stay
untouched.

## What Changes

- `openspec/specs/bitstream/spec.md` requirement 40 and
  `openspec/specs/validator/spec.md` requirements 46 and 50 are reflowed so
  the SHALL keyword appears on the first line of the requirement body.
  **Semantics are unchanged** — this is a wording/reflow edit only, working
  around the OpenSpec CLI first-line keyword scan. (CLI quirk noted for a
  potential upstream report; the reflowed text is also simply clearer.)
- `xtask/src/main.rs::run_ci` gains a run-if-present OpenSpec validation step
  (`openspec validate --all --no-interactive`), mirroring the CI workflow's
  conditional step; `AGENTS.md` § 4 documents it.
- `openspec/changes/README.md`: `add-bitstream-writer` and
  `toy-intra-encoder-v0` move from state `proposed` to `parked (encoder
  track, behind the VALIDATOR-ROADMAP fence)`; each proposal gets a one-line
  parked banner pointing at the fence. The changes are NOT deleted — they
  remain recorded intent, and reviving either means re-proposing against the
  current conventions.
- Matrix: `XTASK-CI-QUALITY-GATES` notes record the completed parity;
  `ENC-BITSTREAM-WRITER` / `ENC-INTRA-TOY-V0` notes record the parked state.

## Non-goals

- Deleting or rewriting the parked encoder changes (revival = re-propose).
- Re-pointing the five planned-change-id matrix references (schema-legal).
- Making `openspec` a required tool anywhere (run-if-present stays; CI does
  not install it).
- Any normative change to the bitstream/validator spec requirements.

## Acceptance criteria

- [ ] `openspec validate --all --no-interactive` passes on the branch (12/12).
- [ ] `cargo xtask ci` runs the OpenSpec validation when `openspec` is
  installed and prints the standard skip hint when not.
- [ ] The reflowed requirements are semantically identical (review-verified).
- [ ] Active-changes table shows both encoder changes as parked with the fence
  reference; `openspec list` still shows them (not deleted).
- [ ] `cargo xtask ci` passes end to end.
