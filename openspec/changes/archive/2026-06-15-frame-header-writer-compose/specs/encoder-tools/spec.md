# encoder-tools delta: frame-header-writer-compose

## ADDED Requirements

### Requirement: composing intra frame-header writer

`splot-core` SHALL provide a composing writer `write_frame_header_core` that is the exact
inverse of `parse_frame_header_core` on the path that reaches
`FrameHeaderParseStatus::IntraHeaderComplete`. It SHALL emit the whole intra frame header in
§ 5.18.2 order — the activation prefix, the control-region glue bits (the frame-type arm, the
long-term-id reads, the output-control flags, `frame_size_override_flag`, `order_hint`,
`refresh_frame_flags`, and `disable_cdf_update`), and every sub-structure (frame size,
screen-content, intrabc, tile, quantization, segmentation, QM setup, delta-Q, lossless,
deblocking, GDF, CDEF, loop-restoration, CCSO, and the tail) — by delegating each sub-structure
to its existing writer. For every model the writer accepts, reparsing the written bits with
`parse_frame_header_core` and the same gating inputs SHALL yield the original
(`parse(write(x)) == x`).

The writer SHALL accept ONLY a model whose `status == IntraHeaderComplete` (with
`frame_is_intra` set, the required fields present, and no partial loop-restoration parse); any
other model — an inter / switch / TIP / bridge / show-existing-frame header, a non-complete
status, or a model with a missing required field — SHALL be rejected with a typed writer error
before any bit is written. The composition SHALL never leave a partial buffer: a reject at any
step SHALL leave `bit_len() == 0`.

#### Scenario: a complete intra frame header round-trips byte-exactly

- **WHEN** an intra frame header that the parser turned into an `IntraHeaderComplete`
  `FrameHeaderCore` is written with `write_frame_header_core` and the same sequence / MFH inputs
- **THEN** the written bytes SHALL be byte-exact and SHALL reparse to an equal `FrameHeaderCore`,
  across the intra frame types (single-picture Key, closed-/open-loop key, intra-only), lossless
  and non-lossless, grain present/absent, single- and multi-tile, and `cur_mfh_id` 0 and > 0.

#### Scenario: a non-intra-complete model is rejected before any bit

- **WHEN** a model carries a non-`IntraHeaderComplete` status, a missing required field, a
  partial loop-restoration parse, or a show-existing-frame / inter header
- **THEN** the writer SHALL return a typed `WriteError` and write no bit.
