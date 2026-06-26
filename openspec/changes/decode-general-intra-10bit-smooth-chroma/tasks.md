# Tasks

## Reconstruction Gate

- [x] 1.1 Relax the 10-bit chroma admission gate in `general_intra.rs` to also
      admit `SupportedChromaMode::Smooth` at the no-neighbour top-left block
      (`frontier.r == 0 && frontier.c == 0`), keeping the DC_PRED-luma
      single-64x64 square-leaf shape unchanged.
- [x] 1.2 Keep a 10-bit non-DC luma block, a non-DC / non-(top-left SMOOTH)
      chroma block, and a neighbour-having SMOOTH chroma block
      (frame-MI `c != 0`) rejected with `unsupported_10bit_non_dc_intra` before
      any coefficient read or sample write.

## Tests And Tracking

- [x] 2.1 Add the `syn-smchroma-intra-64x64-10bit-q160.ivf` conformance fixture
      and a positive decode test pinning the `DecodedFrame<u16>` shape
      (`BitDepth::Ten`, `Yuv420`, 64x64) and the
      `splot-dfh-sha256-v1` frame hash
      `4fe932e5e5dea4a1830eae4853b198c738e8d1919049736d2f4a234c491d5397`.
- [x] 2.2 Confirm the existing 8-bit corpus and the three positive / four
      negative 10-bit fixtures still behave (positives bit-exact; negatives still
      emit their `unsupported_*` reason).
- [x] 2.3 Add matrix, decoder-support, LOCAL-REFERENCE-EVIDENCE, and conformance
      manifest entries for `DECODE-GENERAL-INTRA-10BIT-SMOOTH-CHROMA`.
- [x] 2.4 Regenerate generated docs and run the required checks
      (`cargo xtask ci`, `conformance`, `check-fixtures`).
