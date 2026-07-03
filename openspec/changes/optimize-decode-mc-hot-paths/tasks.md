# Tasks

## 1. Motion-compensation hot paths
- [x] 1.1 Zero-phase unscaled fast path in the § 7.13.3.18 convolution
      (exactness argument documented at the fast path).
- [x] 1.2 Strided borrowing `ReferencePlaneView` + `ReferencePlaneSource`
      (u16 zero-copy, widening copy for narrower storage; BAWP served by
      on-demand linearization).

## 2. Verification
- [x] 2.1 Decoded output byte-identical: full test suite, zero-MV
      dispatcher tests, 22-stream AVM differential corpus, first-frame
      raw sha256 unchanged.
- [x] 2.2 First-frame benchmark: ~372 ms -> ~332 ms median on the
      reference stream (default threads).
