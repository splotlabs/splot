# Proposal: Matrix validate-stage honesty sweep

## Feature IDs

The sweep touches status/notes (never code) on the rows listed in tasks.md;
the umbrella bookkeeping id for the sweep itself is the feature-tracking
process (`DOC-FEATURE-TRACKING` documentation + `XTASK-FEATURE-STATUS`
enforcement context; no stage changes on either).

## Why

71 matrix rows carry `validate = "partial"`. The Phase 1 mission audit
(2026-06-10) found that for roughly 30 of them the "partial" is **not**
remaining locally-decidable validator work: the residual is owned by a
different row, is blocked on parsing work that has its own backlog item, is
decoder-blocked, or the landed checks already cover everything the spec mirror
actually requires for that row. Leaving these as bare `partial` makes the
coverage matrix overstate remaining work ~2x and hides the real dependency
structure. The roadmap's own done-criteria demand "every stateful row has
either `validate = done` or an explicitly documented blocked dependency".

## What Changes

Status/notes-only edits to `docs/IMPLEMENTATION-MATRIX.toml` (no library code,
no new diagnostics), in three dispositions — each verified against the spec
mirror and `docs/VALIDATOR-DIAGNOSTICS.md` before editing, never taken on
faith from the audit:

1. **Close with proof** (`validate` → `done`) where the landed checks cover
   every conformance requirement the mirror states for that row's sections,
   with `[feature.proof]` naming the landed diagnostic ids/tests.
2. **Blocked-on notes** (`validate` stays `partial`) where the residual is
   parse-blocked or decoder-blocked: the note names the blocking matrix
   row/backlog change explicitly (e.g. "blocked on
   `AV2-5.18-FRAME-HEADER` inter paths"; "decoder-blocked: §6.16.13 needs the
   §7.21 output process").
3. **Residual-ownership notes** where the leftover semantics are owned by
   another row (e.g. `AV2-5.2.1-OBU-TYPE` residuals owned by
   `AV2-6.2.2-OBU-HEADER-ACTIVATED-SEQUENCE-LIMITS`): the note names the
   owner; the stage closes if nothing else remains on the row itself.

The feature-tracking spec gains the matching requirement: a `partial` stage on
a normative row SHALL name what remains or what blocks it.

`docs/FEATURE-STATUS.md`/`docs/SPEC-COVERAGE.md` regenerate; audit ledger
re-records.

## Non-goals

- Implementing any new validator check (tranche-B backlog items own those).
- Touching rows whose `partial` reflects genuinely remaining implementable
  work (e.g. `AV2-6.4-SEQUENCE-HEADER-SEMANTICS` residuals, Annex A/E).
- Stage changes anywhere except `validate` (and `decode_check` only where the
  same verified reasoning applies trivially).
- Schema changes; `check-feature-status` behavior changes.

## Acceptance criteria

- [ ] Every row the sweep touches has either `validate = done` with proof
  naming landed diagnostics/tests, or a note naming the concrete blocker
  (matrix row id or spec process) — no bare `partial` remains among the
  audited set.
- [ ] Every disposition was re-verified against the spec mirror section and
  the diagnostics registry; rows whose audit claim did not verify are left
  untouched and reported.
- [ ] `cargo xtask check-feature-status` and `cargo xtask ci` pass; generated
  docs and ledger refreshed.
