## 1. Tracking

- [x] 1.1 Add `DECODE-GENERAL-INTRA-DEEP-SPLIT` to the implementation matrix.
- [x] 1.2 Add decoder support and conformance coverage rows for `general-intra-deep-split`.
- [x] 1.3 Add the `syn-deep-intra-64x64-q120.ivf` fixture, conformance manifest entry, and reciprocal LOCAL-REFERENCE-EVIDENCE entry.

## 2. Verification

- [x] 2.1 Confirm via temporary instrumentation that the fixture's top-left 32x32 SPLITs into four square 16x16 DC_PRED leaves over three flat 32x32 DC quadrants, and that a 16x16 leaf DC-predicts from a reconstructed sibling neighbour inside the parent 32x32.
- [x] 2.2 Confirm `splot decode --output-format raw` equals avmdec `--rawvideo --i420` AND dav2d `--demuxer ivf` byte-for-byte and pin the frame hash in a decode test.
- [x] 2.3 Confirm all existing general intra fixtures still decode bit-exact and a non-DC / rectangular-leaf deeper split still rejects with `decode/unsupported-feature`.

## 3. Documentation And Verification

- [x] 3.1 Regenerate feature/status/coverage docs.
- [x] 3.2 Validate OpenSpec, feature tracking, decoder support, decoder conformance coverage, reference evidence, and the Rust acceptance gate.
