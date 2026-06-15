# Change: obu-header-and-size-writer

## Feature IDs

- `ENC-BITSTREAM-WRITER` (advances the writer surface; umbrella stays `partial`)
- `AV2-5.2.2-OBU-HEADER` (advances its `write` stage `todo -> done`)
- `AV2-5.2.3-TRAILING-BITS` (advances its `write` stage `todo -> done`)

## Why

The OBU-header, trailing-bits, and Annex B framing writers are the first *structural*
writers built on the landed `BitWriter` primitives. They are the exact inverse of the
§ 5.2.2 OBU-header parser and the § 5.2.3 trailing-bits parser, and they assemble an
Annex B OBU (size prefix + header + payload). This unblocks the container muxer and the
parser↔writer round-trip proofs, and it lands the `write_trailing_bits` helper a prior
review flagged as missing (`align_to_byte`/`into_bytes` emit `byte_alignment()`, not the
§ 5.2.3 marker-bit tail).

## What changes

- Add `crates/splot-core/src/write/obu.rs`: `write_obu_header`, `write_obu_header_extension`,
  `write_annexb_obu`, and an `obu_total_len` size helper — the inverse of
  `crate::obu::read_obu_header_from_slice` and the `crate::annexb` OBU envelope.
- Add `BitWriter::write_trailing_bits` (`crates/splot-core/src/write/bit_writer.rs`): the
  inverse of `crate::obu::parse_trailing_bits` (a `1` marker bit then zeros).
- Add `WriteError::{EmptyTrailingBits, InconsistentHeader, NonInferableLayerIds,
  ObuTooLarge}`. The module stays **additive** — no parser, model, or parser-error edits.

## Validator impact

None. No new diagnostics; the validator is unchanged. The writers are reachable only
from library code and tests.

## Non-goals

- No sequence/frame/tile/metadata or other payload writers.
- No IVF muxer (the IVF container write helpers already exist via `AV2-IVF-CONTAINER`;
  wiring them into writer-track round-trip tests is a later change).
- No `obu_extension_flag` / `obu_extension_data` payload-tail emission (§ 5.2.1) — that
  is payload, owned by a future payload writer; `write_annexb_obu` treats payload as
  opaque bytes.
- No entropy/range encoder; no public `encode` CLI.

## Impact

- Crate: `crates/splot-core` (additive `write` module only).
- Docs: `docs/IMPLEMENTATION-MATRIX.toml` (+ regenerated `docs/FEATURE-STATUS.md`),
  `README.md`.
- The implementation matrix remains the source of truth for status.
