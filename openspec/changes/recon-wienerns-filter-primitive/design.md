## Context

The local decoder mission decoder frontier now reaches a parsed frame-level Wiener NS filter
bank and rejects before loop-restoration reconstruction. AV2 §7.20.3 splits the
non-separable Wiener math from the surrounding §7.20 restoration-unit traversal,
source-sample clipping, PC-Wiener classification, temporal filter-bank state, and
chroma luma-sample downsampling. That per-sample luma math is the next safe
`splot-recon` primitive to build before runtime wiring.

## Goals / Non-Goals

**Goals:**

- Add a scheduler-free, panic-free `splot-recon` primitive for the AV2 §7.20.3
  luma non-separable Wiener tap accumulation.
- Take source samples, subclasses, coefficients, dimensions, output stride, and
  bit depth as caller-resolved facts.
- Keep invalid inputs fail-atomic: validation and source-sample range failures
  must not partially mutate the caller output.

**Non-Goals:**

- Full §7.20 loop-restoration traversal, §7.20.2 source-sample clipping/stripe
  handling, §7.20.4 PC-Wiener classification, `SubclassLookup`, chroma luma
  downsampling, temporal/reference Wiener state, restoration-unit syntax,
  runtime decode wiring, or local decoder mission output.

## Decisions

- **Implement the luma table first.** The local decoder mission frontier is a luma-relevant
  frame-level Wiener NS bank, while chroma adds the §7.20.3 luma-sample loop and
  subsampling rules. The primitive therefore exposes `WIENER_NS_LUMA_COEFFS = 16`
  and `WIENER_NS_LUMA_TAPS = 32`, using the §7.20.3 `Wiener_Ns_Config_Y` table.
- **Caller-resolved sample addressing.** The primitive receives a `source_sample`
  callback over block-relative `(x, y)` coordinates. The caller owns frame
  coordinates, §7.20.2 source clipping, restoration-unit boundaries, stripe
  treatment, and border extension.
- **Caller-resolved subclasses.** The primitive accepts either one coefficient
  class for the whole block or an optional `width * height` subclass map. Later
  runtime code can derive that map from §7.20.4 `FilterClass` and `SubclassLookup`
  without changing the arithmetic primitive.
- **Temporary output for fail-atomicity.** The function computes into a temporary
  vector, validates each source sample against the active bit depth, then copies
  into the caller's strided output only after all samples are valid.

## Risks / Trade-offs

- This brick does not unblock local decoder mission by itself. The runtime must still wire
  frame/restoration-unit traversal, the source-sample process, and frame-level
  filter-bank selection before the `unsupported_wienerns_filter_bank` gate can
  move.
- The source callback can encode the wrong §7.20.2 source-sample behavior. The
  primitive docs and support row keep that boundary explicit, and later runtime
  integration needs fixture-level evidence before claiming full loop restoration.
