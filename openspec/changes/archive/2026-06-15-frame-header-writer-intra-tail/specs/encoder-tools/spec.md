# encoder-tools delta: frame-header-writer-intra-tail

## ADDED Requirements

### Requirement: frame-header intra-tail writers

`splot-core` SHALL provide writers that are the exact inverse of the § 5.18.2 intra-tail
parsers — `read_tx_mode()` (§ 5.18.8.1), `film_grain_config()` (§ 5.18.10.1), and the composed
intra tail (`read_tx_mode()`, the no-bit intra inferences, `reduced_tx_set`, the no-bit intra arm
of `global_motion_params()`, and `film_grain_config()`). For every model the writer accepts,
reparsing the written bits with the corresponding parser and the same gating inputs SHALL yield
the original (`parse(write(x)) == x`). The writers SHALL be additive (no model or parser-error
change) and SHALL never panic: a model the parser could not have produced SHALL be rejected with a
typed writer error before any bit is written.

The composed intra-tail writer SHALL validate the whole tail — including the `tx_mode` lossless
consistency and the `film_grain_config()` model — before writing the first bit, so a reject can
never leave a partial buffer.

#### Scenario: each intra-tail structure round-trips across every branch

- **WHEN** a parsed `read_tx_mode()` / `film_grain_config()` / intra tail is written with the same
  gating inputs and reparsed
- **THEN** the reparsed structure SHALL equal the original, across every branch (the lossless
  `ONLY_4X4` inference vs `tx_mode_select`; the three-way `apply_grain` gate and the `fgm_id` /
  `grain_seed` presence; the five no-bit intra inferences and `reduced_tx_set`).

#### Scenario: a non-reproducible intra-tail model is rejected before any bit

- **WHEN** a model carries an `ONLY_4X4` `tx_mode` on a non-lossless frame (or a non-`ONLY_4X4`
  on a lossless one), a `true` for any no-bit intra inference, an `apply_grain` disagreeing with
  its inferred value, a wrong `fgm_id` / `grain_seed` presence, or an `fgm_id` / `reduced_tx_set`
  outside its field
- **THEN** the writer SHALL return a typed `WriteError` and write no bit.
