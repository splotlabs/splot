## Why

The § 7.15 residual-math transform stack is complete; the next reconstruction
stage on the path to bit-exact decode is the loop filters (§ 7.17 deblocking,
§ 7.18 CDEF, § 7.19 loop restoration, § 7.20 GDF). The first and most
self-contained piece is the § 7.17.7.1 deblocking *sample* filter — the per-edge
sample math, separable from the § 7.17 edge traversal and filter-strength
derivation that need frame/block state.

## What Changes

- Add Feature ID `RECON-DEBLOCK-SAMPLE-FILTER`.
- Add `crates/splot-recon/src/deblock_filter.rs` with
  `deblock_sample_filter(line, params)` and the `DeblockSampleFilter` params
  struct.
- Implement § 7.17.7.1 over a caller-supplied perpendicular sample `line`: the
  `deltaM2 = Clip3(-qThrClamp, qThrClamp, (p1 - q1 + 3*(q0 - p0)) * 4)` ramp, the
  per-side `Round2(deltaM2 * w_mult * (maxWidth - i), 3 + DF_SHIFT)` deltas, and
  the § 4.8 `Clip1` updates of the current and previous sides, gated by
  `curr_lossless` / `prev_lossless`.
- Take `boundary`, `q_thr`, `max_width_neg`, `max_width_pos`, the three
  pre-indexed `Q_Thresh_Mults` / `W_Mult` weights, the lossless flags, and
  `bit_depth` as caller-resolved facts. The § 7.17.1/§ 7.17.2 edge traversal and
  § 7.17.3-§ 7.17.7.2 filter-size/strength/choice derivation stay with the caller.
- Pass the table weights as scalars rather than depending on the tables: the
  § 9.2 deblock tables live in `splot-core`'s `conversion` module (which has core
  consumers and so cannot be wholesale-moved to `splot-tables`), so the scalar
  handoff avoids a gen-tables split.
- Keep it total and panic-free (i64 ramp, `qThrClamp` clamped non-negative so
  `Clip3` never inverts, validated line bounds and per-side widths) with two new
  typed `ReconError` variants.
- Preserve the current runtime `splot decode` behavior and all output bytes (a
  `pub` primitive with no runtime rewiring).
- Add tests: a `Round2` rounding test, a hand-computed symmetric width-2 case, an
  asymmetric/lossless/clamped reference match, a both-lossless no-op, a `Clip1`
  bit-depth clamp, fail-atomic rejection, and an i32-extreme totality sweep.
- Update the implementation matrix, decoder support matrix, roadmap, generated
  status/coverage docs, the decoder-conformance-coverage group, and the crate
  `//!` docs.

Non-goals:

- No § 7.17 edge traversal, no filter-size/strength/choice derivation, no other
  loop filters (CDEF, CCSO, loop restoration, GDF), no runtime wiring, no
  dependency-graph change, and no AVM/dav2d invocation.

## Capabilities

### Modified Capabilities

- `decoder-support`: add a supported row for the § 7.17.7.1 deblocking sample
  filter, the first loop-filter primitive.

## Impact

- Affected code: `crates/splot-recon/src/deblock_filter.rs`,
  `crates/splot-recon/src/error.rs`, `crates/splot-recon/src/lib.rs`,
  `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated status/coverage docs, and
  `xtask/src/decoder_conformance_coverage.rs`.
- Public API impact: one additive `pub fn` and `pub struct`, plus two additive
  error variants; no breaking changes.
- Diagnostics impact: none; existing minimal runtime diagnostics and output bytes
  remain unchanged.
- Dependencies and licensing: no new dependencies and no licensing changes.
