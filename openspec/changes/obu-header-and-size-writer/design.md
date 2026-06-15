# Design: obu-header-and-size-writer

## Context

`crate::obu::read_obu_header_from_slice` reads the AV2 § 5.2.2 OBU header MSB-first:
byte 0 is `obu_header_extension_flag` f(1), `obu_type` f(5), `obu_tlayer_id` f(2); byte 1
(only when the extension flag is set) is `obu_mlayer_id` f(3), `obu_xlayer_id` f(5). When
the extension is absent the parser *infers* `obu_mlayer_id = 0` and `obu_xlayer_id` to
`GLOBAL_XLAYER_ID` for the global-scope types or `0` otherwise. `parse_trailing_bits`
(§ 5.2.3) reads a `trailing_one_bit == 1` then zero bits. Annex B frames each OBU as
`leb128(num_bytes_in_obu)` + header + payload.

The writer must invert all three. The correctness contract is the round-trip:
`read(write(x)) == x` for every value the writer accepts.

## Decision D1 — `write_trailing_bits` on `BitWriter`, header/framing in `write/obu.rs`

`trailing_bits(nbBits)` is a primitive over bits (no model dependency), so it lives on
`BitWriter` beside `align_to_byte`. The header and Annex B framing writers depend on the
model (`ObuHeader`, `ObuType`, layer ids), so they live in the new `write/obu.rs`. The
speculative `write_trailing_bits_to_alignment` convenience is deferred until a payload
writer needs it (no caller yet).

## Decision D2 — reject non-producible headers, don't silently drop

Following the primitive layer's established philosophy (reject exactly the values the
reader could never produce, so round-trip holds for everything accepted):

- `write_obu_header` validates `has_header_extension` ⇔ `header_size_bytes`
  (`InconsistentHeader` otherwise).
- For a no-extension header it validates that the layer ids equal the § 5.2.2 inference
  (`obu_mlayer_id == 0`, `obu_xlayer_id ==` inferred); a header carrying non-inferable ids
  is unrepresentable without the extension byte, so the writer returns
  `NonInferableLayerIds` rather than emitting byte 0 and letting `read(write(x))` differ.

This makes the semantic round-trip **unconditional** for every header `write_obu_header`
accepts, rather than a documented precondition.

## Decision D3 — byte-exact is the canonical-subset guarantee; semantic is universal

- **Semantic** (`read(write(x)) == x`) holds for every header the writer accepts and
  every framed OBU — the property-tested contract.
- **Byte-exact** (`parse -> write -> bytes` identical) holds for the canonical subset:
  a header is canonical (single inference path), and the Annex B size prefix is minimal.
  The one alternate encoding is a non-minimal LEB128 size; `write_annexb_obu` always emits
  the minimal form, so byte-exactness is lost there while semantic round-trip is preserved.
  Documented on `write_annexb_obu` and asserted by the non-canonical caveat test.

## Testing strategy

- Header semantic + byte-exact round-trip on the four canonical parser vectors.
- Exhaustive header-byte sweep (every `obu_type` × `tlayer` no-extension with inferred ids;
  every `mlayer` × `xlayer` for the extension case), write → reparse → equal.
- `trailing_bits` round-trip (`write_trailing_bits` → `parse_trailing_bits`) for a range of
  widths, plus the `trailing_bits(0)` rejection.
- Annex B framing byte-exact + reparse on canonical vectors; a non-canonical-size caveat
  test asserting the re-emission differs but reparses semantically equal.
- Error-path unit tests for `InconsistentHeader`, `NonInferableLayerIds`, and `ObuTooLarge`
  (via the extracted `obu_total_len`, avoiding a `u32::MAX`-byte allocation).
- Property tests: `roundtrip_obu_header`, `roundtrip_annexb_obu`, and a never-panics test.
- The `cargo fuzz` differential harness remains the dedicated `roundtrip-and-fuzz-harness`
  change; the proptests already exercise the round-trip over the value space.
