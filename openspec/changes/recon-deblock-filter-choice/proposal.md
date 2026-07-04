## Why

The deblock filter's parameter derivations are in place — the § 7.17.7.1 sample
filter (`RECON-DEBLOCK-SAMPLE-FILTER`), the § 7.17.3 widths
(`RECON-DEBLOCK-FILTER-MAX-WIDTH`), and the § 7.17.5 `qThr` / `sideThr`
(`RECON-DEBLOCK-ADAPTIVE-STRENGTH`). The § 7.17.7.2 filter-choice process — which
turns those thresholds plus the edge samples into the chosen filter width — is the
last self-contained piece of the deblock filter.

## What Changes

- Add Feature ID `RECON-DEBLOCK-FILTER-CHOICE`.
- Add `deblock_filter_choice(s, t, params) -> Result<usize>` and the
  `DeblockFilterChoice` parameter struct to
  `crates/splot-recon/src/deblock_filter.rs`.
- Transcribe the § 7.17.7.2 process: the immediate `0` on a zero `qThr` / `sideThr`,
  the `secondDeriv[-2..=1]` estimate from the two perpendicular edge sample lines
  `s` / `t`, and the threshold cascade (`sideThr`, `sideThr >> 2`, `sideThr >> 3`,
  `(sideThr * 3) >> 4`, and the per-distance `(sideThr * dist) >> 4` /
  `qThr * Q_First[dist - 4]` loop) that selects the width.
- Transcribe the asymmetric directional-derivative gradient term exactly (the
  negative side uses `s[-1] - s[-2]`, the positive side uses `s[0] - s[1]`).
- Take the § 7.17.3 widths, the § 7.17.5 thresholds, the `s` / `t` sample lines,
  and the § 9.2 `Q_First` array (as a fixed-size `[i32; DBL_REG_DECIS_LEN]`) as
  caller-resolved facts; `Q_First` lives in `splot-core`'s § 9.2 tables, which
  `splot-recon` cannot reach.
- Keep the function total and panic-free: validate the widths (`1..=8`) and the
  line lengths before any sample read, keep every access inside the
  `[boundary - maxSamplesNeg, boundary + maxSamplesPos - 1]` window (the
  unconditional `s[3]` read is covered for every `maxWidthPos > 1`, the deeper
  negative reads are guarded by the matching `maxWidthNeg` conditions), and make
  the `Q_First` lookup in-bounds via the fixed-size array.
- Preserve the current runtime `splot decode` behavior and all output bytes.
- Add tests: hand-anchored deterministic cases (flat widens to the full width, a
  high-curvature spike returns `0`, widths `1`/`3`, zero thresholds, the error
  cases) plus a 4000-case property test against an independent in-test spec
  re-trace.
- Update the implementation matrix, decoder support matrix, roadmap, generated
  status/coverage docs, the decoder-conformance-coverage group, and the crate and
  module `//!` docs.

Non-goals:

- No § 7.17.6 filter-level selection, no § 7.17.1 / § 7.17.2 edge traversal, no
  per-edge sample gathering into `s` / `t`, no other loop filters, no runtime
  wiring, no dependency-graph change.

## Capabilities

### Modified Capabilities

- `decoder-support`: add a supported row for the § 7.17.7.2 filter-choice
  derivation.

## Impact

- Affected code: `crates/splot-recon/src/deblock_filter.rs`,
  `crates/splot-recon/src/lib.rs`, `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`, generated
  status/coverage docs, and `xtask/src/decoder_conformance_coverage.rs`.
- Public API impact: one additive `pub` function plus its parameter struct; no
  breaking changes.
- Diagnostics impact: none.
- Dependencies and licensing: no new dependencies and no licensing changes.
