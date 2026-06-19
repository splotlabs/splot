## ADDED Requirements

### Requirement: Deblocking filter-choice width derivation

The repository SHALL provide a scheduler-free `splot-recon` derivation for the AV2
§ 7.17.7.2 filter-choice process, tracked by `RECON-DEBLOCK-FILTER-CHOICE`. The
`deblock_filter_choice(s, t, params) -> Result<usize>` function SHALL choose the
deblocking filter width (`0..=maxWidthPos`) from the two perpendicular edge sample
lines `s` and `t`: it SHALL return `0` immediately when `qThr` or `sideThr` is `0`,
estimate `secondDeriv[-2..=1]` from both lines, and walk the § 7.17.7.2 threshold
cascade (`sideThr`, `sideThr >> 2`, `sideThr >> 3`, `(sideThr * 3) >> 4`, and the
per-distance `(sideThr * dist) >> 4` / `qThr * Q_First[dist - 4]` loop), with the
negative-side directional derivative using `s[-1] - s[-2]` and the positive side
using `s[0] - s[1]`. The § 7.17.3 widths, the § 7.17.5 thresholds, the `s` / `t`
lines, and the § 9.2 `Q_First` array SHALL be caller-resolved. The function SHALL
be total and panic-free — validating the widths (`1..=8`) and the line lengths
before any sample read and keeping every access inside the
`[boundary - maxSamplesNeg, boundary + maxSamplesPos - 1]` window — and SHALL NOT
implement the § 7.17.6 filter-level selection, the § 7.17.1 / § 7.17.2 edge
traversal, the per-edge sample gathering, the other loop filters, or runtime decode
wiring.

#### Scenario: Filter choice matches the spec

- **WHEN** `cargo test -p splot-recon deblock_filter --locked` runs
- **THEN** the test suite covers the hand-anchored deterministic cases (flat
  widens to the full width, a high-curvature spike returns `0`, widths `1` and `3`,
  zero thresholds, and the error cases) and a property test comparing
  `deblock_filter_choice` against an independent in-test re-trace of the
  § 7.17.7.2 pseudocode
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation
