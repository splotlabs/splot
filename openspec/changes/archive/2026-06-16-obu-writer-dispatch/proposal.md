# Change: obu-writer-dispatch

## Feature IDs

- `ENC-BITSTREAM-WRITER` (advances the writer surface; umbrella stays `partial`)

## Why

The keystone for the remaining writer-mission backlog (the roundtrip-and-fuzz harness and the
cross-tool `writer stream → splot validate clean` tests): a **unified complete-OBU writer**, the
inverse of `dispatch_obu_payload` / `finish_obu_payload`. Today every per-structure writer emits the
payload **body only** (no OBU header, no `obu_extension_flag` / `trailing_bits()` tail), and there is
no single function that turns a parsed OBU (`ObuHeader` + `ParsedObu`) back into bytes. The fuzz
harness (`parse → write → reparse`) and the cross-tool tests both require it.

(Scope-audit note: the AV2 v1.0.0 Annex B container is *flat* — `leb128(num_bytes_in_obu)` + OBU per
OBU, no temporal-unit/frame-unit hierarchy — so `write_annexb_obu` already frames a single complete
OBU; the IVF record writers already exist. The missing keystone is this OBU-payload dispatch, not a
muxer.)

## What changes

- **Writer** (`crates/splot-core/src/write/obu.rs`, or a new `write/dispatch.rs`):
  - `write_obu_payload(writer, payload: &ParsedObu, is_extensible: bool, passthrough: &[u8])` — the
    inverse of the per-type body **plus** the `finish_obu_payload` tail (when the payload is
    non-empty and `is_extensible`, emit `obu_extension_flag = 0` then `trailing_bits()`). It
    dispatches over `ParsedObu`'s variants.
  - `write_complete_obu(writer, header: &ObuHeader, payload: &ParsedObu, passthrough: &[u8])` —
    `write_obu_header` then `write_obu_payload(.., header.obu_type.is_extensible_obu(), ..)`.
- **Partial coverage (honest stub).** Only the types with a body writer are dispatched:
  `TemporalDelimiter` (empty body), `SequenceHeader` (`write_sequence_header`), `Padding` (opaque
  passthrough bytes + the `padding`/`trailing` split), `MetadataShort` / `MetadataGroup`
  (`write_metadata_*`). The other ten `ParsedObu` variants (Msdo, MultiFrameHeader,
  LayerConfigurationRecord, AtlasSegment, OperatingPointSet, BufferRemovalTiming, QuantizationMatrix,
  FilmGrain, ContentInterpretation) have no body writer yet, so the dispatch returns a typed
  `WriteError::Unimplemented { feature }` (a new additive variant) — the fuzz harness skips them and
  the cross-tool minimal stream does not use them. The frame-carrying types (tile group, SEF / TIP)
  are `PrefixParsed`, not `ParsedObu`, and are framed by `write_tile_group_obu` / the frame-header
  writers separately (a following slice assembles them).
- **Reject-before-write** (scratch-writer; `bit_len()` unchanged on reject): a `passthrough` that
  disagrees with a fully-modeled vs opaque arm; any delegated sub-writer reject; an unwritten type
  (`Unimplemented`). The `metadata_*` arms thread the existing passthrough.
- **No model change.** Reuses the existing per-structure writers + `write_obu_header` /
  `write_trailing_bits`.

## Validator impact

None.

## Non-goals

- No writers for the ten currently-unwritten OBU types (each is its own future slice).
- No frame-carrying-type framing (tile group / SEF / TIP) or stream assembly (a following slice).
- No fuzz target or cross-tool test yet (the next slices, which sit on this).
- No public `encode` command.

## Impact

- Crate: `crates/splot-core` (additive `write` surface + one additive `WriteError::Unimplemented`
  variant).
- Docs: `docs/IMPLEMENTATION-MATRIX.toml` (a WRITER note on `ENC-BITSTREAM-WRITER`) + regenerated
  `docs/FEATURE-STATUS.md`.
