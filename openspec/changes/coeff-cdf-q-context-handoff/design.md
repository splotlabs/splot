## Context

The current staged coefficient branch stack has reached the `useFsc`
shared-facts wrapper. That wrapper is still loaded-but-unwired and still accepts
`coeff_cdf_q_ctx` as a caller-resolved scalar. AV2 derives the active
coefficient CDF q-context during `init_coeff_cdfs()` from frame `base_q_idx`
using four threshold buckets in § 6.17.2
(`docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-2`), while
§ 3 defines `COEFF_CDF_Q_CTXS = 4`
(`docs/spec/av2/1.0.0/03-symbols.md`).

The runtime frame/tile boundary already carries parsed `base_q_idx`, but the
runtime `coeffs()` loop is not wired to this branch stack yet. This change
therefore adds a narrow crate-private handoff layer: derive q-context from
`base_q_idx` and delegate to the existing shared-facts wrapper without changing
runtime output.

## Goals / Non-Goals

**Goals:**

- Add a total crate-private helper for the AV2 coefficient CDF q-context bucket:
  `0` for `base_q_idx <= 90`, `1` for `<= 140`, `2` for `<= 190`, and `3`
  otherwise.
- Add a crate-private wrapper above `apply_coeff_use_fsc_branch_from_shared_facts`
  that accepts nonzero shared facts carrying `base_q_idx` instead of
  `coeff_cdf_q_ctx`.
- Preserve all-zero ordering: all-zero inputs SHALL delegate directly to the
  existing all-zero path without requiring `base_q_idx`.
- Prove equivalence between the new base-q wrapper and the existing explicit
  q-context shared-facts wrapper across ordinary and FSC selected paths.
- Keep the existing explicit q-context wrapper available for lower-boundary
  tests and staged callers.

**Non-Goals:**

- Do not refactor tile CDF storage to keep only one active q-context row.
- Do not implement runtime `coeffs()` wiring, CDF save/load lifecycle changes,
  full `compute_tx_type`, dequantization, inverse transforms, residual add,
  reconstruction, output, reference refresh, or inter prediction.
- Do not change public APIs, crate dependencies, encoder behavior, or runtime
  validation diagnostics.

## Decisions

1. Put the helper and wrapper in `coeff_loop/use_fsc_branch.rs`.

   Rationale: the module already owns the staged `useFsc` selector, condition
   wrapper, and shared-facts wrapper. The q-context handoff is the next layer
   immediately above that boundary.

   Alternative considered: add a `cdf` module helper. That would be reasonable
   when CDF initialization/lifecycle storage is refactored, but this brick only
   adapts one loaded coefficient-branch caller surface.

2. Keep `base_q_idx` as `u32` and derive q-context infallibly.

   Rationale: the parsed frame fact is already carried as `u32`, and the AV2
   threshold expression has a total "otherwise" bucket. Values beyond the syntax
   domain still map to bucket 3 without panicking, which keeps staged helpers
   total while leaving syntax-domain validation at the parser/frame-header layer.

   Alternative considered: reject values above the bitstream syntax maximum.
   That would duplicate parser validation inside a loaded-but-unwired handoff
   and could introduce ordering differences before runtime integration.

3. Derive q-context only for nonzero inputs.

   Rationale: AV2 § 5.20.7.27 handles `all_zero` before the nonzero branch work
   this wrapper prepares. Keeping all-zero independent avoids forcing frame q
   facts into tests and callers that only exercise the all-zero path.

   Alternative considered: require `base_q_idx` on the enum even for all-zero
   inputs. That would shrink one type but weaken the ordering proof.

## Risks / Trade-offs

- [Risk] Future runtime integration could mistake this as complete CDF
  initialization. Mitigation: keep matrix, OpenSpec, and roadmap notes explicit
  that this only derives the active q-context scalar and still leaves CDF
  lifecycle and runtime `coeffs()` unwired.
- [Risk] Existing tests might not observe q-context differences if payloads take
  q-invariant branches. Mitigation: add boundary tests that compare explicit
  q-context delegation for all four threshold buckets and include at least one
  selected-row mutation/position proof for ordinary and FSC selected paths.
- [Risk] The helper's thresholds could drift from the spec. Mitigation: cite
  § 6.17.2 next to the helper and test exact boundary values at 90/91,
  140/141, and 190/191.
