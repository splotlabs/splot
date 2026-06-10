# Tasks

> Status: **parked** (2026-06-11, encoder track behind the VALIDATOR-ROADMAP fence). Blocked on `add-bitstream-writer` and enough frame/tile header writer support; revival means re-proposing.

## Implementation

- [ ] Emit a sequence header and one intra frame via the writer.
- [ ] Wire the toy path into the `splot-encode` `Context`.

## Tests and proof

- [ ] `splot validate` accepts the toy output (record as a fixture).
- [ ] (Stretch) `avm decode` accepts the toy output.
- [ ] Record proof in the `ENC-INTRA-TOY-V0` row.

## Checks

- [ ] `cargo xtask ci`
