## Context

`RECON-DEBLOCK-SAMPLE-FILTER` / `-FILTER-MAX-WIDTH` / `-ADAPTIVE-STRENGTH` cover the
deblock filter's sample math and parameter derivations. § 7.17.7.2 is the width
decision that sits between them: given the § 7.17.5 thresholds and the edge
samples, it returns how many samples the sample filter should modify.

## Goals / Non-Goals

**Goals:** a total, panic-free transcription of the § 7.17.7.2 cascade over
caller-resolved sample lines, widths, thresholds, and the `Q_First` table.

**Non-Goals:** the § 7.17.6 filter-level selection, the § 7.17.1 / § 7.17.2 edge
traversal, the per-edge sample gathering, and any runtime wiring.

## Decisions

- **Caller-prepared `s` / `t` lines with a `boundary` offset.** The spec prepares
  `s` (the first edge row/column) and `t` (the last, `count - 1`) from `CurrFrame`
  with `dx` / `dy` / `count`; that gathering is the caller's job, matching the
  sample filter's `boundary`-indexed perpendicular-line contract. The primitive
  takes the two prepared lines and the shared `boundary` (the index of the spec
  `s[0]` / `t[0]`).
- **`Q_First` as a fixed-size array.** `Q_First` is a § 9.2 table in `splot-core`,
  which `splot-recon` cannot reach, so the caller passes it. Using a fixed-size
  `[i32; DBL_REG_DECIS_LEN]` (not a slice) makes the `Q_First[dist - 4]` lookup
  total with no length check.
- **Robust bounds for any width.** The spec's `s[3]` read at the
  positive-`endThr` step is unconditional once `maxWidthPos != 1`; the § 7.17.3
  widths are `{1, 3, 4, 6, 8}` so it never exceeds `maxSamplesPos - 1` there, but
  the validation requires the positive span to cover index `3` for every
  `maxWidthPos > 1` so the function is panic-free for any caller width in `1..=8`.
- **Asymmetric gradient term.** The negative-side directional derivative uses
  `s[-1] - s[-2]` and the positive side uses `s[0] - s[1]`; the helper takes the
  gradient neighbour explicitly so each call passes the correct one.

## Risks / Trade-offs

- The cascade is long and index-dense, so correctness rests on a faithful
  transcription. Mitigated by a 4000-case property test against an independent
  in-test re-trace of the spec pseudocode, hand-anchored deterministic cases, and
  an adversarial spec re-trace of all seven cascade dimensions (the gradient
  asymmetry, the comparison operators, the loop order, and the bounds). It is
  loaded ahead of its runtime caller, matching the established pattern.
