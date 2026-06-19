## Context

§ 7.17 deblocking splits into the edge traversal (§ 7.17.1/§ 7.17.2), the
filter-size/strength/choice derivation (§ 7.17.3-§ 7.17.7.2, needing
`DeblockingTxSizes`, filter levels, and block state), and the § 7.17.7.1 sample
filter — the per-edge sample math. The sample filter is the same shape as the
other `splot-recon` primitives: a pure operation over a sample line and
caller-resolved spec-derived values.

## Goals / Non-Goals

**Goals:**

- A total, panic-free `splot-recon` primitive for the § 7.17.7.1 sample filter.
- Avoid a new crate edge or gen-tables change.

**Non-Goals:**

- The edge traversal, the filter-size/strength/choice derivation, and the other
  loop filters; any runtime wiring.

## Decisions

- **Take a 1D perpendicular sample line with a `boundary` index.** § 7.17.7.1
  walks `CurrFrame[plane][y ± k*dy][x ± k*dx]` — a row (vertical edge) or column
  (horizontal edge). The caller linearizes that into `line` with `q0` at
  `boundary`; the primitive needs no `dx`/`dy` and no frame buffer.
- **Caller-resolved table weights, not a table dependency.** `Q_Thresh_Mults` /
  `W_Mult` are in `splot-core`'s `conversion` module, which has core consumers,
  so it cannot be moved to `splot-tables` wholesale, and splitting a deblock
  sub-module out of `conversion` in gen-tables would be a larger change. Passing
  `q_thresh_mult`, `w_mult_neg`, `w_mult_pos` as the three pre-indexed scalars
  keeps `splot-recon` free of the tables and matches the caller-resolves-values
  contract.
- **Implement the q-side gating literally.** § 7.17.7.1 gates the current-side
  update only on `!currLossless` (so it runs for all `i` in `0..width`), and the
  previous-side update on `i < maxWidthNeg && !prevLossless`. When
  `maxWidthPos < maxWidthNeg`, the current side modifies samples beyond
  `maxWidthPos` with `(maxWidthPos - i) < 0` (a sign-flipped taper); the
  `asymmetric` reference test pins this exactly as written rather than guessing a
  `maxWidthPos` clamp.
- **`qThrClamp.max(0)` for totality.** `qThrClamp = q_thr * q_thresh_mult` is
  non-negative for conformant inputs; clamping it non-negative keeps `Clip3`
  well-formed (and the filter inert) for any caller input instead of panicking on
  an inverted clamp range.

## Risks / Trade-offs

- There is no conformance fixture for the sample filter yet, so the tests are an
  independent in-place re-trace plus a hand-computed anchor: the
  `matches_hand_computed_symmetric_width_2` test pins `[10,20,60,50] →
  [18,36,44,42]` from the spec arithmetic (deltaM2 320, weights 25/51, Round2
  shift 11), catching a transcription or wiring error regardless of the re-trace.
- It is loaded ahead of its runtime caller and the rest of § 7.17, matching the
  established pattern of building recon primitives before the edge traversal and
  runtime wiring; the matrix and roadmap keep those out of scope.
