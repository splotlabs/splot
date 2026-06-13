# Tasks

> Status: **proposed**. None started. (The committed-corpus runner that occupies
> `cargo xtask conformance` landed under the archived
> `conformance-corpus-foundation` change; the tasks below are the remaining
> *live* AVM comparison.)

## Implementation

- [ ] Define the local AVM checkout discovery (no vendoring; opt-in, never CI).
- [ ] Implement a live, opt-in `avm encode` → `splot validate` comparison mode in
  `xtask` (distinct from the committed-corpus runner).
- [ ] Document the opt-in reproduction command in `docs/CONFORMANCE.md`.

## Tests and proof

- [ ] Record the reproduction command in the `CONF-AVM-DIFF-HARNESS` row.

## Checks

- [ ] `cargo xtask ci`
