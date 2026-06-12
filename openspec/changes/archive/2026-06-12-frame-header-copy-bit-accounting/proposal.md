# Proposal: frame_header_copy and NumFrameHeaderBits accounting

## Feature IDs

- `AV2-5.18.1-FRAME-HEADER-GENERAL` (frame_header(isFirst) dispatch,
  NumFrameHeaderBits, frame_header_copy)
- `AV2-5.19-TILE-GROUP` (the non-first tile group's header-copy region —
  confirm the row id)
- `AV2-5.18.2-FRAME-HEADER-INFO` (the recorded bit length consumes the
  completed intra parse)

## Why

`frame_header(isFirst=1)` records
`NumFrameHeaderBits = get_position() - startBitPos` over
`frame_header_info()` (mirror `05-syntax-structures.md`:3924);
`frame_header(isFirst=0)` is `frame_header_copy()` — exactly
`NumFrameHeaderBits` raw bits, a bit-identical copy of the first header
(:3964-3979). The parsers only handle the isFirst=1 path and the tile-group
prefix records-but-skips non-first header copies. PR #59's completed intra
parse makes the exact bit length available for intra streams — the gate
this change rides on. Landing it also gives the header/tile-data boundary
inside non-first tile groups (the codex PR #59 padding-ambiguity scenario)
and is the named home of the tile-group-side truncation residual.

## What Changes

1. Record `NumFrameHeaderBits` when the first header's
   `frame_header_info()` parses to completion (the IntraHeaderComplete
   path; incomplete/stopped parses record nothing — Unknown routing).
2. Parse `frame_header_copy()` on non-first tile groups when the first
   header's bit length is known: consume exactly that many bits and
   compare BIT-IDENTITY against the first header's bits (ground the
   identity requirement in the § 6 semantics — find the exact clause; if
   the requirement is stated, add the diagnostic with citation; the copy
   length mismatch/EOF is a § 5.18.1 syntax defect).
3. Unknown routing: a first header that did not complete (coverage stops,
   inter paths) leaves the copy region unparsed exactly as today.
4. `inspect` surfaces the copy region's status; the § 5.19 tile-group
   prefix's record-but-skip is replaced by the real parse where decidable.
5. Trailing-bits/tile-data boundary improvements that become decidable for
   non-first tile groups land; what stays undecidable (first-tile-group
   tile-data boundary needs § 5.19/§ 5.20 payload modeling) is named.

## Non-goals

- § 5.19 tile-payload/§ 5.20 parsing (its own backlog change).
- Inter-path first headers (the gate extends when inter parses complete).

## Acceptance criteria

- [ ] NumFrameHeaderBits recorded for completed intra headers;
  frame_header_copy parsed + bit-identity checked on non-first tile
  groups; mismatch/truncation diagnostics with citations;
  positive/negative/EOF tests; Unknown routing for incomplete first
  headers; both arrival orders where state is involved.
- [ ] Matrix proof recorded; `cargo xtask ci` green.
