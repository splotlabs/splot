## Context

The dequantization process consumes a per-coefficient quantizer `q2`. For
quantization-matrix-coded transforms, § 7.14.4 step 2 derives
`q2 = Round2(q * m, 5)` where `m` is the quantization-matrix weight. This change
implements that weighting for the built-in `Quantizer_Matrix` and makes the
§ 9.4 tables reachable from `splot-recon`.

## Goals / Non-Goals

Goals:

- Implement the § 7.14.4 step-2 built-in `Quantizer_Matrix` weighting (lookup +
  `Round2(q*m, 5)`), total and panic-free.
- Relocate the § 9.4 quantizer tables to `splot-tables` so `splot-recon` can
  consume them without depending on `splot-core`.

Non-Goals:

- The `useQm` / `useUserQm` / `segLvl` gating, the user-defined `UserQm`
  matrices, the `shift` / `useFsc` derivation, the coefficient entropy decode,
  and the inverse-transform invocation.

## Decisions

- **Relocate § 9.4 via `output_dir_for` (the #210 mechanism).** `splot-recon`
  may not depend on `splot-core`, so the generated `Quantizer_Matrix` / `Qm_Offset`
  tables (which had no `splot-core` consumer) are routed to the dependency-free
  `splot-tables` crate by adding `"quantizer"` to `output_dir_for`'s shared arm —
  exactly as the § 9.6/§ 9.7 kernels were relocated. The move is byte-identical
  (the generator emits the same bytes to a different directory), and
  `gen-tables --check` confirms zero drift; the determinism count stays 236.
- **Weight lookup + combine as separate functions.** `quantization_matrix_weight`
  does the bounds-checked `Quantizer_Matrix[segLvl][plane > 0][Qm_Offset[txSz] +
  i*tw + j]` lookup (returning a typed error on any out-of-range index, so it is
  total). `qm_weighted_quantizer(q, m)` does the § 7.14.4 step-2 `Round2(q*m, 5)`,
  in `i64` and clamped into `u32` so it is total even for extreme inputs. The
  caller composes them and passes the result as `q2` to `dequant_coefficient`,
  mirroring how the dequant arithmetic already takes a resolved `q2`.
- **Gating deferred.** Whether QM is used at all (`useQm`, `useUserQm`, `segLvl`,
  `using_qmatrix`) and the user-defined `UserQm` matrices need frame/segment
  state; they remain caller-resolved / future work. This row provides the
  built-in-matrix weight computation only.

## Risks / Trade-offs

- The relocation touches the shared-tables enumeration in several docs; all are
  updated so the "§ 9.6/§ 9.7 shared, other five in splot-core" statements become
  "§ 9.4/§ 9.6/§ 9.7 shared, other four in splot-core".

## Migration Plan

The table move is byte-identical and has no `splot-core` consumer, so no code
breaks. Additive new functions plus one new `ReconError` variant; no existing API
changes; the runtime is unaffected.

## Open Questions

None.
