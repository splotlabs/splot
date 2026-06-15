# Design: frame-header-writer-intra-tail

## Context

The § 5.18.2 intra tail (after `ccso_params()`) reads `read_tx_mode()` (§ 5.18.8.1), a run of
no-bit intra inferences (`reference_select` / `skip_mode_present` / `allow_bawp` /
`allow_warpmv_mode`), `reduced_tx_set` `f(2)`, the no-bit intra arm of `global_motion_params()`
(§ 5.18.9.1, which returns before `use_global_motion`), and `film_grain_config()`
(§ 5.18.10.1). The model (`FrameHeaderTail` / `FilmGrainConfig` / `TxMode`) already records each
field plus the inferred no-bit ones.

## Decisions

- **Additive — direct writes plus inferred-field validation.** No read-but-not-stored value
  needs surfacing: every field is either coded (and the writer emits it after a domain check) or
  inferred no-bit (and the writer re-derives and rejects a disagreeing model). No arithmetic
  (no subtraction / shift / index), so the writer is panic-free by construction once the `f(n)`
  domains (`fgm_id < 8`, `reduced_tx_set < 4`; `grain_seed` is `u16`, so `f(16)` always fits)
  are checked.
- **`ONLY_4X4` is lossless-only.** `read_tx_mode()` only infers `ONLY_4X4` when `CodedLossless`;
  on the non-lossless path it reads `tx_mode_select` `f(1)`. The writer rejects an `ONLY_4X4`
  model when not lossless and a non-`ONLY_4X4` model when lossless.
- **The intra inferences are validated, never coded.** The five no-bit fields are inferred
  `false` on the intra path; a `true` for any of them is not parser-reachable and is rejected.
- **Reject-before-write across the composed tail.** `write_intra_tail` runs
  `check_intra_tail_encodable` — which re-validates `tx_mode` against `coded_lossless` and calls
  `check_film_grain_encodable` — fully before writing the first (`tx_mode`) bit. This is the key
  ordering decision: without it, a model whose `film_grain` is invalid but whose `tx_mode` /
  `reduced_tx_set` are valid would write those bits before the film-grain reject, leaving a
  partial buffer.

## Testing

Round-trip via the public parsers across every branch (tx_mode lossless / largest / select;
film_grain gated-off / not-output / single-picture-inferred / coded-apply true+false; intra_tail
lossless / non-lossless / grain-absent). One reject test per `NonCanonicalFrameHeader` path
(asserting `bit_len() == 0`), including a model that is valid through `reduced_tx_set` but carries
a bad `film_grain` field — confirming the composed tail still rejects before any bit. A round-trip
property test per parser plus a never-panics-on-constructed-models proptest for the film-grain
writer.
