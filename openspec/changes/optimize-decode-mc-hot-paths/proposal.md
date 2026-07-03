# Optimize decode motion-compensation hot paths

## Why

First-output-frame latency profiling showed two avoidable § 7.13.3.18
costs: the two-pass sub-pel convolution runs its full 8-tap filter for
whole-pel motion (where every tap row is the pure `{ .., 128, .. }`
entry), and every predicted block linearizes its whole reference plane
into an owned widened buffer before reading it.

## What Changes

- The § 7.13.3.18 convolution takes an exact zero-phase unscaled fast
  path: with unit steps and both sub-pel phases zero, the output is the
  clipped reference sample scaled by the residual rounding shift, which
  is exact because each partial product is a multiple of the rounding
  divisor.
- Reference-plane readers borrow `u16` plane storage through a strided
  `ReferencePlaneView` instead of copying the plane per predicted block;
  narrower storage keeps the widening copy. `ReconSample` exposes the
  identity `u16` reinterpretation for the borrow.

## Impact

- Affected specs: decoder-support (no behavior change; decoded output is
  byte-identical — pinned by the existing zero-MV dispatcher tests, the
  sub-pel unit tests, and the conformance corpus)
- Affected code: `splot-recon` sub-pel convolution and reference view,
  `splot-decode` inter motion-compensation plumbing
