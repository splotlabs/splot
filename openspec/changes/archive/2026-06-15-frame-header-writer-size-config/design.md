# Design: frame-header-writer-size-config

## Context

The intra frame-header control region reads `frame_size()` (§ 5.18.4.1),
`screen_content_params()` (§ 5.18.3.3), and `intrabc_params()` (§ 5.18.3.4). The latter two
read several bits the modeled decode path derives nothing from (`intrabc_params()`'s
`allow_global_intrabc` / `allow_local_intrabc` / `change_bvp_drl` / `max_bvp_drl_bits_minus_1`,
and `screen_content_params()`'s `force_integer_mv` on the intra wrapper), which the parser
previously read purely for bit-alignment and discarded.

## Decisions

- **Maintainer-approved model extension for full byte-exact write.** `FrameHeaderCore` stores
  `consumed_bits`, so canonicalizing the discarded bits (the leb128/num_ref_frames pattern)
  would make the eventual full-core round-trip hold only on a canonical subset. The maintainer
  chose full byte-exact instead, so this change surfaces the discarded bits in the model and
  parser — a deliberate, signed-off exception to the writer mission's additive / read-only-parser
  rule. The surfacing mirrors the existing `screen_content_params` / `_full` pattern:
  `parse_intrabc_params_full` returns an `IntrabcParams` whose conditionally-read fields are
  `Some` exactly when their bit was present; `parse_intrabc_params` stays a `bool` wrapper so the
  inter caller is untouched; `consumed_bits` is unchanged (the same bits are read, just stored).
- **Per-structure writers, threaded gating inputs.** Each writer takes the same gating inputs
  the parser receives (`seq_force_*`, `frame_size_override_flag`, `frame_width/height_bits`,
  `frame_is_intra`, `allow_frame_max_bvp_drl_bits`, `default_dims`) and round-trips through the
  public parser. The composing `write_frame_header` (which supplies those inputs from the
  surrounding header) is a later slice.
- **Reject-before-write covers inferred/derived fields.** `screen_content_params()` codes a flag
  only on the `SELECT` sentinel; the writer rejects an inferred flag that disagrees with the
  sequence force value. `frame_size()` non-override emits no bits, so the writer rejects a size
  that does not equal `default_dims`, an overridden dimension that overflows its `f(n)` field, and
  a zero dimension (the parser derives `minus_1 + 1 >= 1`). `intrabc_params()` rejects any `Option`
  whose presence disagrees with its gate and a `max_bvp_drl_bits_minus_1` outside the `ns(2)`
  domain. The existing `WriteError` variants (`ValueTooWide`, `ValueOutOfRange`,
  `NonCanonicalFrameHeader`) suffice; no new variant is added.

## Testing

Per structure: byte-exact round-trip unit tests across every branch (override / non-override;
`SELECT` / forced; the four intrabc gate combinations) and a round-trip property test driving the
parser on random bits then re-emitting. One reject test per `WriteError` path (asserting
`bit_len() == 0`). A new `intrabc_params_full` parser test confirms the surfacing is faithful and
`consumed_bits`-neutral.
