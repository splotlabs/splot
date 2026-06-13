# Change: avm-differential-harness

## Feature IDs

- `CONF-AVM-DIFF-HARNESS`

## Why

AVM is the AV2 reference software and our local conformance oracle. A *live*
differential harness lets us compare `splot` against AVM systematically rather
than by hand.

The committed-corpus foundation already landed (the archived
`conformance-corpus-foundation` change): `cargo xtask conformance` and the
`crates/splot-cli/tests/conformance.rs` CI gate now validate committed,
AVM-generated vectors against a manifest with **no AVM dependency**. This change
covers what remains — the *live* comparison against a local AVM checkout.

## Scope

- A live, opt-in `avm encode` → `splot validate` comparison driven from a local
  AVM checkout (later, `splot encode` → `avm decode` once an encoder exists).
- Crates/modules: `xtask` (a live mode alongside the committed-corpus runner).

## Non-goals

- No vendoring of AVM; AVM is never a build or CI dependency (maintainer
  decision). The live harness is a local, opt-in developer step, never run in
  normal CI.
- No network access in normal CI.
- The committed-corpus runner and its manifest are already done (the archived
  `conformance-corpus-foundation` change); this change does not redo them.

## Acceptance criteria

- [ ] A documented, opt-in command runs `avm encode` → `splot validate` against
      a local AVM checkout and reports clean/defect per stream.
- [ ] Results are reproducible from a documented command; it does not run in
      normal CI and requires a local AVM checkout the developer supplies.
- [ ] Proof is recorded in the `CONF-AVM-DIFF-HARNESS` row.

> Status: **proposed**. The committed-corpus runner that occupies
> `cargo xtask conformance` landed separately; the *live* AVM comparison is not
> yet implemented.
