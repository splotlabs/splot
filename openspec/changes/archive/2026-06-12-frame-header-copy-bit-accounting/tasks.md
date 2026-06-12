# Tasks: frame_header_copy and NumFrameHeaderBits

## 1. Bookkeeping

- [x] 1.1 Confirm matrix row ids; `openspec_change` set; re-read § 5.18.1
  (05 mirror :3900-3990) verbatim, the § 6 semantics for the copy's
  bit-identity, and the tile-group prefix's record-but-skip site
  (splot-core tile_group.rs ~73-84).

## 2. Implementation

- [x] 2.1 Record NumFrameHeaderBits on completed first-header parses.
- [x] 2.2 Parse frame_header_copy on non-first tile groups (exact bit
  count + bit-identity comparison); diagnostics with citations.
- [x] 2.3 Unknown routing for incomplete first headers; EOF handling.

## 3. Surfacing and docs

- [x] 3.1 inspect surfaces the copy status; matrix rows advance with
  proof; named residuals for what stays undecidable; generated docs;
  roadmap.

## 4. Verification

- [x] 4.1 Positive/negative/EOF tests; identity mismatch; both orders.
- [x] 4.2 `check-feature-status` + `check-diagnostic-registry` pass.
- [x] 4.3 `cargo xtask ci` (bare, exit checked) passes.
