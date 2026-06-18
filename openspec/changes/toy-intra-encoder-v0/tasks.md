# Tasks

> Status: **parked and superseded** (2026-06-18, `encoder-program-contract`).
> This bootstrap-era change is not the implementation starting point. Future
> all-intra work must be re-proposed under Baseline Encoder Profile v1 with the
> current writer, reconstruction, validation, and conformance gates.

## Implementation

- [ ] Emit a sequence header and one intra frame via the writer.
- [ ] Wire the toy path into the `splot-encode` `Context`.

## Tests and proof

- [ ] `splot validate` accepts the toy output (record as a fixture).
- [ ] (Stretch) `avm decode` accepts the toy output.
- [ ] Record proof in the `ENC-INTRA-TOY-V0` row.

## Checks

- [ ] `cargo xtask ci`
