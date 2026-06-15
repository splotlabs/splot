# Change: bit-writer-primitives

## Feature IDs

- `ENC-BITSTREAM-WRITER` (advances `write` from the `RangeEncoder`-only stub to a
  realized primitive layer; umbrella stays `partial`)
- `AV2-4.11.3-UVLC`, `AV2-4.11.4-SVLC`, `AV2-4.11.5-LE`, `AV2-4.11.6-LEB128`,
  `AV2-4.11.7-SU`, `AV2-4.11.8-NS` (each advances its `write` stage `todo -> done`)
- `AV2-4.11.10-RG` (new descriptor row created to track the `rg(n)` parse + write
  stages independently, like its `§ 4.11` siblings)
- `AV2-5.2.4-BYTE-ALIGNMENT` (advances its `write` stage for zero-pad alignment)

## Why

The bitstream writer is the serialization half of the codec and the foundation of
the encoder bridge. Every higher-level writer (OBU header, sequence/frame headers,
metadata, the container muxers) is built on a bit/byte writer that is the exact
inverse of `splot-core`'s `BitReader`. This change lands that foundation:
`BitWriter`, the inverse of every `BitReader` primitive, proven by property tests
that establish `read(write(x)) == x` for every value the writer accepts.

This is a deliberate, maintainer-approved start of the writer track: the
`docs/VALIDATOR-ROADMAP.md` "do not start yet" fence is lifted for the writer in the
same change. The writer's correctness is defined by round-trips against the existing
parser, not by any public command — no `encode` CLI ships.

## What changes

- Add `crates/splot-core/src/write/`: a `BitWriter` (`bit_writer.rs`) and a
  self-contained `WriteError` (`error.rs`). The module is **additive** — it depends
  on the reader/model read-only and changes no parser, model, or error code.
- Invert every `BitReader` primitive: `f(n)` (`write_bit`/`write_bits`/
  `write_bits_u8`), `su(n)`, `uvlc`, `svlc`, `le(n)`/`le(n)->u64`, `leb128`, `ns(n)`,
  `rg(n)`, and zero-pad byte alignment (`align_to_byte`).
- Property tests for the round-trip contract plus unit tests for every error path,
  and a "writer never panics on any value/width" property test.

## Validator impact

None. No new diagnostics; the validator and its rule set are unchanged. The writer
is reachable only from library code and tests.

## Non-goals

- No OBU-header, sequence/frame-header, metadata, or payload writers (later changes).
- No container muxers (Annex B / IVF) (later changes).
- No entropy/range encoder — the `RangeEncoder` stub stays unimplemented.
- No `cargo fuzz` target yet: the differential round-trip and primitive fuzz targets
  land with the dedicated `roundtrip-and-fuzz-harness` change. The proptest suite
  already exercises the `read(write(x)) == x` contract over the full value space.
- No public `encode` CLI command.

## Impact

- Crates: `crates/splot-core` (additive `write` module only).
- Docs: `docs/IMPLEMENTATION-MATRIX.toml` (+ regenerated `docs/FEATURE-STATUS.md`,
  `docs/SPEC-COVERAGE.md`), `docs/VALIDATOR-ROADMAP.md` (fence lift), `README.md`.
- The implementation matrix remains the source of truth for status.
