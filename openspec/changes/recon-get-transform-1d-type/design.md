## Context

The § 7.15.4 outer 2D inverse transform leaves `row_type` / `col_type`
caller-resolved. The spec derives them with `get_transform_1d_type(dir, sz)`:
`t = Transform_1d_Type[PlaneTxType][dir]`, then if
`useDdt && (t == ADST || t == FDST) && sz != 4` return `(t == ADST) ? DDTX : FDDT`.
`splot-recon` already has the target type — `InverseTransform2dDim` (`Identity` |
`Kernel(InverseTransform1dType)`) — as the `row_type` / `col_type` field type of
`InverseTransform2dOuter`.

## Decisions

- **Return `InverseTransform2dDim` directly.** The base table holds only the four
  types `DCT`, `ADST`, `FDST`, `IDT`; `DDTX` / `FDDT` arise solely from the
  `useDdt` substitution. `IDT` maps to `InverseTransform2dDim::Identity` and the
  kernel types to `InverseTransform2dDim::Kernel`, so the result drops straight
  into the existing `InverseTransform2dOuter` fields with no adapter.

- **Hand-write `Transform_1d_Type`; do not route it through `gen-tables`.** Like
  `Transform_Shift`, `Transform_1d_Type` is a § 7.15.4 process-body constant and
  is **not** present in the generated `all_tables.h` § 9 attachment (a `grep`
  confirms zero hits), so it is a hand-written, spec-cited constant, transcribed
  verbatim from `07-decoding-process.md#s-7-15-4` (lines 10679-10696).

- **`TransformPass` enum for `dir`.** Rather than a bare `0` / `1`, a
  `TransformPass { Row, Col }` enum names the spec `dir` and reads clearly at the
  (future) two call sites (`get_transform_1d_type(0, w)` /
  `get_transform_1d_type(1, h)`).

- **`size` is the adjusted pass dimension.** The spec passes `w` / `h` (the
  adjusted `1 << adjLog2`) and only ever compares `sz != 4`, so the function takes
  the sample size and uses it solely in that comparison; it is not otherwise
  validated (any non-4 size enables the substitution, matching the spec).

- **`use_ddt` is caller-resolved.** The spec `useDdt =
  enable_inter_ddt && !use_intrabc && is_inter` is a frame/block-state boolean;
  consistent with the crate's caller-resolves-state contract, the function takes
  the resolved `use_ddt` rather than the three flags. For the intra tier `is_inter`
  is false, so `use_ddt` is false and the base table is returned unchanged.

- **`const fn` + strictly additive.** Like `transform_shift`, the function is a
  `const fn` (a fixed `PlaneTxType` resolves at compile time) and rewires no
  existing path, so the minimal flat-intra fixture snapshots are byte-identical.
  Correctness is proven by unit tests against the verbatim spec table.

## Risks / Trade-offs

- **Transcription risk** on the 16-row table — mitigated by an
  independently-transcribed per-`PlaneTxType` table test (both passes) and the
  spec-mirror gate.
- **Substitution-eligibility risk** (the `&& size != 4` and `ADST`/`FDST`-only
  conditions) — mitigated by a test that checks the eligible cases, the `size == 4`
  and `use_ddt == false` disables, and that `DCT` / `IDT` are never substituted.
