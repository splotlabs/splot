# Design: seq-header-writer-configs

## Context

The six sequence-header config parsers (§ 5.4.3–5.4.8) and `seg_info` (§ 5.4.9) are read
mid-byte inside `sequence_header_obu()`, each gated by flags/derived values from the general
fields. This change writes their inverse, building on the merged general-fields writer.

## Decisions

- **Public per-config writers, tested via the public parser** (mirroring #3a): each
  `write_X(w, cfg, gating)` round-trips through `parse_X(reader, gating)`. The configs begin
  mid-byte, so none take a byte-alignment check (matching the parser's bit position).
- **Reject before any bit, every field.** Each writer validates fully up front in a
  `check_*_encodable` pass that covers EVERY field write (f(n) width, `ns(n)` domain, signed
  ranges, and every non-canonical/inferred/derived value the parser could not have produced).
  The segment config pre-validates the nested `seg_info` body *before* its leading flags, so
  the composite rejects before any bit (a first cut emitted three flags before the seg_info
  body rejected — caught by a `bit_len()==3` test and fixed).
- **Canonicalization where the parser cannot distinguish encodings.** `num_ref_frames == 8`
  is reachable both explicitly (`f(4)` = 7) and by inference; the model cannot distinguish
  them, so the writer always emits the inferred (shorter) form. Semantic round-trip holds;
  byte-exactness is not guaranteed for an explicit-8 input. Documented inline.

## Testing

Per config: a semantic round-trip property test over parser-reachable models across every
branch, byte-exact unit tests, one rejection test per `WriteError` path (asserting
`bit_len()==0`, incl. the field-width and domain paths), and a never-panics test.
