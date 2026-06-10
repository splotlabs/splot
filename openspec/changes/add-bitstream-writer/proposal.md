# Change: add-bitstream-writer

> **Parked (2026-06-11):** encoder-track change behind the explicit
> "do not start yet" fence in
> [`docs/VALIDATOR-ROADMAP.md`](../../../docs/VALIDATOR-ROADMAP.md). This is a
> bootstrap-era stub predating the current change conventions; reviving it
> means re-proposing against them.

## Feature IDs

- `ENC-BITSTREAM-WRITER`
- `AV2-4.11.6-LEB128` (advances its `write` stage)
- `AV2-5.2.2-OBU-HEADER` (advances its `write` stage)
- `AV2-5.2.3-TRAILING-BITS` (advances its `write` stage, when modeled)
- `AV2-5.2.4-BYTE-ALIGNMENT` (advances its `write` stage, when modeled)

## Why

An encoder needs to emit bytes. A bit/byte writer symmetric with the parsers is the
foundation for LEB128, OBU headers, and (later) high-level syntax — and for
round-trip tests that prove the parser and writer agree.

## Scope

- Spec sections: § 4.11.6 (LEB128), § 5.2.2 (OBU header), § 5.2.3/§ 5.2.4
  (trailing bits / byte alignment) as those are modeled.
- Crates/modules: `splot-core` (`bitio`).

## Non-goals

- No entropy (range) encoder.
- No high-level syntax the parser cannot yet read.

## Acceptance criteria

- [ ] A `BitWriter` mirrors `BitReader` for `f(n)`.
- [ ] LEB128 and OBU-header writers exist.
- [ ] Round-trip tests: write → parse yields an equal structure.
- [ ] Matrix `write` stage and proof are updated only where round-trip-proven.

> Status: **proposed**. Not implemented.
