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
`parse_frame_header_core` and the same gating inputs SHALL yield the original on every structural
field (`parse(write(x)) == x`). Because the delegated sub-writers canonicalize redundant
descriptor encodings (e.g. `write_read_delta_q` emits the shorter `delta_coded == 0` form for a
`delta_q == 0` model the parser also accepts as the coded-zero form, like the § 5.18.6 quant /
§ 5.4 leb128-minimal cases), this round-trip is semantic universally and byte-exact only on the
canonical subset; the informational derived `consumed_bits` is excluded from the equality.

The writer SHALL accept ONLY a model whose `status == IntraHeaderComplete` (with
`frame_is_intra` set, the required fields present, and no partial loop-restoration parse); any
other model — an inter / switch / TIP / bridge / show-existing-frame header, a non-complete
status, or a model with a missing required field — SHALL be rejected with a typed writer error
before any bit is written. The composition SHALL never leave a partial buffer: a reject at any
step SHALL leave `bit_len() == 0`.

#### Scenario: a complete intra frame header round-trips

- **WHEN** an intra frame header that the parser turned into an `IntraHeaderComplete`
  `FrameHeaderCore` is written with `write_frame_header_core` and the same sequence / MFH inputs
- **THEN** the written bytes SHALL reparse to a `FrameHeaderCore` equal to the original on every
  structural field (and byte-exact when the original used the canonical sub-encodings), across the
  intra frame types (single-picture Key, closed-/open-loop key, intra-only), lossless and
  non-lossless, grain present/absent, single- and multi-tile, and `cur_mfh_id` 0 and > 0.

#### Scenario: a non-intra-complete model is rejected before any bit

- **WHEN** a model carries a non-`IntraHeaderComplete` status, a missing required field, a
  partial loop-restoration parse, or a show-existing-frame / inter header
- **THEN** the writer SHALL return a typed `WriteError` and write no bit.
