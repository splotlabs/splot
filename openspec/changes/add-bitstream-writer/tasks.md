# Tasks

> Status: **parked** (2026-06-11, encoder track behind the VALIDATOR-ROADMAP fence). None started; revival means re-proposing.

## Implementation

- [ ] `BitWriter` (`f(n)`, MSB-first) symmetric with `BitReader`.
- [ ] LEB128 writer (§ 4.11.6).
- [ ] OBU-header writer (§ 5.2.2).

## Tests and proof

- [ ] Round-trip property tests (write → parse).
- [ ] Record proof in the `ENC-BITSTREAM-WRITER` row.

## Checks

- [ ] `cargo xtask ci`
