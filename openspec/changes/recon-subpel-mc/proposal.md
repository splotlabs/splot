## Why

The first inter frame decodes bit-exact only for the zero-MV (full-pel) case,
where § 7.13.3.18 reduces to a co-located reference copy. Natural-content motion
vectors are overwhelmingly sub-pel, so the next inter decode milestone needs the
§ 7.13.3.18 separable interpolation-filter convolution — the sub-pel
motion-compensation kernel. Like the § 7.13.2.8 IDIF kernel, that convolution and
its § 9 `Subpel_Filters` table are a self-contained reconstruction primitive that
can land and be unit-tested ahead of the decode wiring (the § 5.20.7 `read_mv`
shell scheme, the `interp_filter` symbol read, and the gate relaxation).

## What Changes

- Add Feature ID `RECON-SUBPEL-MC`.
- Add `crates/splot-recon/src/subpel_mc.rs` with `subpel_predict_block(reference,
  params) -> Result<Vec<u16>>`, the `SubpelPredictParams` / `ReferencePlaneView`
  / `InterpolationFilter` types, and the verbatim § 7.13.3.18
  `SUBPEL_FILTERS[6][16][8]` table.
- Transcribe the § 7.13.3.18 two-pass convolution: the horizontal filter pass
  into an `intermediateHeight * w` array with `Round2(s, InterRound0)`, then the
  vertical pass with `Round2(s, InterRound1)`, with the filter taps selected from
  `SUBPEL_FILTERS` by the § 6 `interp` (with the § 7.13.3.18 small-block 4-tap
  substitution applied per pass) and the sub-pel phase `(p >> 6) & SUBPEL_MASK`.
- Apply the § 7.13.3.16 `InterRound0 = 3` / `InterRound1 = 11` (non-compound)
  rounding and the final § 4.8 `Clip1` for the § 7.13.3 single-reference write
  (`CurrFrame[plane][y + i][x + j] = Clip1(Preds[0][i][j])`).
- Implement the § 7.11.3.x reference-border extension via the § 7.13.3.18
  `Clip3(firstX/firstY/lastX/lastY, ...)` plus the view's own dimension clamp,
  without copying a padded reference plane.
- Take the § 7.13.3.17 `startX`/`startY`/`stepX`/`stepY` scaling, the § 7.13.3.18
  reference-clipping region, the § 6 `interp_filter`, the block dimensions, and
  the reference samples as caller-resolved facts.
- Keep the function total and panic-free: validate the reference buffer length and
  non-zero dimensions, reject a zero / oversized (`> 128`) block and a negative
  step, guarantee the vertical-pass `base + 7` row is in range (with an explicit
  guard), and keep the `i64` filter sums overflow-free for in-range samples.
- Preserve the current runtime `splot decode` behavior and all output bytes (the
  zero-MV inter and intra fixture snapshots stay byte-identical).
- Add tests: hand-anchored worked examples (a full-pel position is a bit-exact
  reference copy, a flat reference reconstructs flat for any phase, a
  hand-computed `EIGHTTAP_SHARP` half-pel, the border-extension corner case,
  10-bit `Clip1`, the small-block 4-tap substitution, the error cases), the
  verbatim-table invariant check (every row sums to 128, all taps even, plus
  distinctive-row spot checks), and a 2000-case property test against an
  independent in-test re-trace of the § 7.13.3.18 pseudocode.
- Update the implementation matrix, decoder support matrix, roadmap, generated
  status/coverage docs, the decoder-conformance-coverage group, and the crate and
  module `//!` docs.

## Impact

- Affected specs: `decoder-support`.
- Affected code: `crates/splot-recon/src/subpel_mc.rs` (new),
  `crates/splot-recon/src/subpel_mc/tests.rs` (new),
  `crates/splot-recon/src/error.rs`, `crates/splot-recon/src/lib.rs`,
  `xtask/src/decoder_conformance_coverage.rs`, and the generated docs.
- No dependency-graph, runtime-output, or licensing change.
