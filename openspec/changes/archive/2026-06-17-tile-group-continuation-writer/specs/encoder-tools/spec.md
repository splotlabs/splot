# encoder-tools delta: tile-group-continuation-writer

## ADDED Requirements

### Requirement: non-first tile-group continuation writer

`splot-core` SHALL provide a writer that serializes a non-first (`is_first_tile_group == 0`)
`tile_group_obu()` payload (§ 5.19 / § 5.20.1) back to bytes — the inverse of `parse_tile_group_prefix`
on the continuation path, the `frame_header_copy()` region (§ 5.18.1), `parse_tile_group_structure`,
and `parse_tile_group_framing` — so a coded frame with more than one tile group round-trips. The writer
SHALL emit `is_first_tile_group = 0`, the explicit `frame_header_present_flag`, and — when that flag is
set — the recorded first header's `NumFrameHeaderBits` `frame_header_copy()` bits verbatim, then the
shared § 5.19 structure (with no `tg_start == 0` restriction) and § 5.20.1 payload framing. It SHALL be
reject-before-write and SHALL never panic on a constructed model, rejecting a non-byte-aligned writer, a
`frame_header_present_flag` that disagrees with whether copy bits are supplied, and every reject the
delegated structure / payload sub-writers raise.

#### Scenario: a non-first tile group round-trips

- **WHEN** a non-first `tile_group_obu()` payload (with `frame_header_present_flag` set or clear, and a
  `tg_start` that may be non-zero) is written by the continuation composer and the bytes are reparsed
  into their prefix / structure / framing pieces
- **THEN** the reparsed pieces SHALL equal the originals, byte-exact on the canonical subset, and the
  `frame_header_copy()` region SHALL match the recorded first header bit-for-bit.

#### Scenario: a non-canonical constructed model is rejected, not panicked

- **WHEN** the continuation composer is given inputs the parser could never produce (a
  `frame_header_present_flag` vs copy-bits mismatch, or a degenerate / out-of-range structure or
  framing)
- **THEN** it SHALL return a typed `WriteError` and write no bit, never panicking.
