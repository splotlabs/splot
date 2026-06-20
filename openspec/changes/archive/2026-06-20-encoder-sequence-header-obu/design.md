## Context

The decoder minimal-tier IVF frame requires `OBU_TEMPORAL_DELIMITER` (brick 6),
`OBU_SEQUENCE_HEADER`, then the frame OBU (brick 5). The committed
`syn-cos-intra-64x64-q180.ivf` conformance vector contains the exact sequence-header OBU the
decoder accepts: `leb128(12)` + header `0x04` + an 11-byte `sequence_header()` body.

## Decision: parse-back the conformance-vector body, write body+tail

`build_minimal_intra_sequence_header` parses the committed 11-byte body into a
`SequenceHeader` (parse-backed: conformant by construction). The OBU payload is the body
**plus the § 5.2.1 / § 5.2.3 OBU tail** (`obu_extension_flag = 0` then `trailing_bits()`,
since the sequence header is an extensible OBU) — `write_obu_payload` emits this, whereas
`write_sequence_header` writes the body alone (no tail). The byte-exact oracle below proved
this distinction: the body-only writer produced `…e0 20` (zero padding) versus the vector's
`…e0 22` (the trailing-bits `1`-marker).

## Oracle

The payload round-trips byte-exact to the 11-byte body, and the Annex B OBU is byte-exact to
the conformance vector's sequence-header OBU (`0c 04` + body) and reparses as one
`OBU_SEQUENCE_HEADER`. Byte-matching a decoder-accepted vector is the strongest splot-core
oracle (it caught the body-vs-tail bug).

## Non-Goals

A temporal unit, an IVF stream, the frame OBU made consistent with this sequence header
(the assembler brick's job — the frame OBU must parse against this header), a packet, or
`receive_packet` output.
