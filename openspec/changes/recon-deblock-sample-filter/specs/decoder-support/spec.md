## ADDED Requirements

### Requirement: Deblocking sample filter process

The repository SHALL provide a scheduler-free `splot-recon` primitive for the AV2
§ 7.17.7.1 deblocking sample-filter process, tracked by
`RECON-DEBLOCK-SAMPLE-FILTER`. The `deblock_sample_filter(line, params)` function
SHALL, over a perpendicular sample `line` whose current-side base index is
`boundary` (so `q0 = line[boundary]`, `q1 = line[boundary + 1]`, `p0 =
line[boundary - 1]`, `p1 = line[boundary - 2]`), compute `deltaM2 =
Clip3(-qThrClamp, qThrClamp, (p1 - q1 + 3*(q0 - p0)) * 4)` with `qThrClamp =
q_thr * q_thresh_mult`, then for `i` from `0` to `Max(max_width_neg,
max_width_pos) - 1` set the current-side sample at `boundary + i` to `Clip1(sample
- Round2(deltaM2 * w_mult_pos * (max_width_pos - i), 3 + DF_SHIFT))` unless
`curr_lossless`, and, for `i < max_width_neg`, the previous-side sample at
`boundary - 1 - i` to `Clip1(sample + Round2(deltaM2 * w_mult_neg * (max_width_neg
- i), 3 + DF_SHIFT))` unless `prev_lossless`, reading `q0`/`q1`/`p0`/`p1` from the
original line before any write (`DF_SHIFT = 8`). It SHALL take `boundary`,
`q_thr`, the per-side widths, the three pre-indexed `Q_Thresh_Mults` / `W_Mult`
weights, the lossless flags, and `bit_depth` as caller-resolved facts and SHALL
NOT implement the § 7.17 edge traversal, the filter-size/strength/choice
derivation, the other loop filters, or runtime decode wiring. It SHALL be total
and panic-free for valid inputs and SHALL reject a per-side width outside `1..=8`
or a line too short for the samples around `boundary` with a typed `ReconError`
before modifying any sample.

#### Scenario: Sample filter succeeds with self-contained tests

- **WHEN** `cargo test -p splot-recon deblock_filter --locked` runs
- **THEN** the test suite covers `Round2` rounding, a hand-computed symmetric
  width-2 case (`[10, 20, 60, 50]` to `[18, 36, 44, 42]`), an asymmetric /
  lossless / clamped reference match, a both-lossless no-op, a `Clip1` bit-depth
  clamp at 8 and 10 bits, and an i32-extreme totality sweep
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: Invalid input is rejected fail-atomically

- **WHEN** `deblock_sample_filter` is called with a per-side width outside `1..=8`
  or a `line` too short to hold the previous- and current-side samples around
  `boundary`
- **THEN** it returns `ReconError::DeblockFilterInvalidWidth` or
  `ReconError::DeblockFilterLineTooShort` respectively and leaves `line`
  unmodified
