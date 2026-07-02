## Why

The `ac0ej3` full-stream mission requires decoding real AVM-encoded inter
frames. Every such stream sets `enable_flex_mvres = 1` (AVM's default), and the
inter frontier rejected the whole stream on that single sequence flag — `splot
decode ac0ej3.ivf` emitted ZERO output frames even for `--limit=1`, because
output frame 0 is triggered by an immediate-output inter frame in the first
temporal unit. Two further entropy desyncs sat behind the gate: the § 8.3.2
neighbour contexts for `is_inter`/`skip_flag`/`use_amvd`/`comp_mode`/
`single_ref` were derived from the superblock-row-restricted `NPos` list where
the spec uses the unrestricted `NPosBuf` (first divergent symbol: `skip_txfm`
at MI(32,0) of ac0ej3 frame 1), and the § 5.20.7.14 `read_motion_mode`
SIMPLE-path `inter_intra` flag was never read (desyncing every interintra-
enabled frame with 8x8..=64x64 single-reference blocks).

## What Changes

- Implement § 5.20.7.13 flexible MV resolution: per-block
  `use_most_probable_precision` / `pb_mv_precision` reads (new
  `TileUseMostProbablePrecisionCdf[3]` / `TilePbMvPrecisionCdf[2][3]` wiring,
  § 9.3 defaults already generated), the `adjustedPrecision` derivation with
  the `MV_PRECISION_TWO_PEL` skip rule, sub-one-pel `read_mv` shell classes
  (`TileJointShell0/1Class0/1Cdf`), `lower_mv_precision` predictor rounding
  for NEWMV-family blocks below `MV_PRECISION_HALF_PEL`, the dual
  frame-vs-block precision condition in `is_mvd_sign_derive_allowed`, and the
  `UseMostProbablePrecisions` / `MvPrecisions` neighbour grids.
- Carry both § 5.20.7.2 neighbour lists: `NPosBuf` (any in-frame neighbour)
  for `is_inter`/`skip_flag`/`use_amvd`/`comp_mode`/`single_ref` (with
  `count_refs` over BOTH reference lists), `NPos` (drops the row above the
  superblock) for `interp_filter` and the new precision contexts.
- Read the § 5.20.7.15 `inter_intra` flag on the SIMPLE path (frame-enabled
  INTERINTRA, single-reference, 8x8..=64x64), frontier-rejecting a set flag.
- Drop `enable_flex_mvres` from the inter frame-tools gate.
- Split the § 5.20.7.23 inter residual reads into
  `runtime_minimal/inter/block/residual.rs` (source-line allowance headroom).
- Add the ignored `ac0ej3_full_stream_avm_compare` harness (env-driven avmdec
  byte-compare with per-frame digests and first-mismatch coordinates).

## Impact

- `syn-2frame-inter-64x64-10bit.ivf` and `syn-grid-inter-128x128-q80.ivf` now
  decode byte-identical to `avmdec --i420 --rawvideo` (hash-pinned tests);
  `ac0ej3.ivf --limit=1` reproduces the AVM frame-0 sentinel
  `974f3db7f82ae57168fb38b83922ed7d` through the production CLI.
- The ac0ej3 frontier moves from byte 8307 (zero output) to byte 8345: the
  first inter frame with the in-loop filter chain enabled (deblock/CDEF/LR/
  CCSO + tx_mode Select), the next mission family.
- Touches `splot-decode` (inter runtime, CDF wiring), `splot-core` (public
  `INTERINTRA` index), `splot-cli` (harness + gate-pin test refresh),
  `docs/IMPLEMENTATION-MATRIX.toml` notes. No dependency-graph changes.
