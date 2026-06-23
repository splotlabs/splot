## 1. Tracking

- [x] 1.1 Add `RECON-WIENERNS-CHROMA-FILTER-PRIMITIVE` to the implementation matrix.
- [x] 1.2 Add decoder support/status coverage for `wienerns-chroma-filter-primitive`.

## 2. Implementation

- [x] 2.1 Add AV2 §7.20.3 chroma Wiener NS constants and parameter types to `splot-recon`.
- [x] 2.2 Implement `wiener_ns_filter_chroma_block` with chroma tap accumulation, luma tap contribution, luma downsampling, `Round2`, and `Clip1`.
- [x] 2.3 Keep validation fail-atomic for dimensions, output shape, subsampling, luma bounds, sample type, and source sample range.
- [x] 2.4 Export the additive chroma primitive without changing runtime decode behavior.

## 3. Verification

- [x] 3.1 Add focused tests for zero coefficients, chroma taps, luma taps, 4:2:0 downsampling, non-subsampled luma reads, clipping, and fail-atomic errors.
- [x] 3.2 Run `openspec validate recon-wienerns-chroma-filter-primitive --no-interactive`, focused recon tests, feature/support checks, conformance, and `cargo xtask ci`.
- [x] 3.3 Sync and archive the completed OpenSpec change before merging the PR.
