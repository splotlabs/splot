# Tasks: § 5.20 tile-payload framing boundaries

## 1. Bookkeeping

- [x] 1.1 Matrix rows confirmed; `openspec_change` set; read § 5.20.1
  verbatim (05 mirror :8549-8640), the § 6.x semantics for tile sizes,
  and § 8.2's init_symbol/exit_symbol for what is checkable without
  decoding.

## 2. Implementation

- [x] 2.1 Per-tile framing parse on completed § 5.19 structures
  (tile_size_minus_1 le(TileSizeBytes), bookkeeping, lastTile/IsBridge
  arms).
- [x] 2.2 Framing-defect diagnostics with citations; § 8 init residual
  decision.
- [x] 2.3 inspect surfaces the tile framing; Unknown routing.

## 3. Docs

- [x] 3.1 Registry; matrix proof; named residuals (decode_tile, BRU
  arms, § 8 beyond framing); generated docs; roadmap.

## 4. Verification

- [x] 4.1 Positive/negative/EOF per arm; multi-tile and single-tile;
  bridge; proptests.
- [x] 4.2 `check-feature-status` + `check-diagnostic-registry` pass.
- [x] 4.3 `cargo xtask ci` (bare, exit checked) passes.
