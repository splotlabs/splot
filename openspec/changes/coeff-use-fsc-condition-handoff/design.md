## Context

The current coefficient path has a crate-private selector,
`apply_coeff_use_fsc_branch`, that accepts decoded all-zero inputs or a caller
supplied nonzero `use_fsc` boolean, then dispatches to the ordinary or FSC
branch handoff. AV2 section 5.20.7.27 derives that boolean after
`compute_tx_type`, `get_tx_class`, and `get_scan`, and only inside the nonzero
coefficient block path.

This change adds one narrow derivation layer above the selector. It does not
derive `PlaneTxType`, `is_inter`, `fsc_mode`, `enable_fsc`, or any runtime frame
state; those remain caller-resolved facts until broader `coeffs()` integration
lands.

## Goals / Non-Goals

**Goals:**

- Add a crate-private `DECODE-COEFF-USE-FSC-CONDITION-HANDOFF` wrapper that
  derives the AV2 section 5.20.7.27 `useFsc` condition for decoded nonzero
  coefficient blocks.
- Preserve all-zero ordering: all-zero inputs SHALL delegate to the ordinary
  all-zero branch without requiring condition facts.
- Reuse the existing `apply_coeff_use_fsc_branch` selector and its ordinary/FSC
  lower wrappers.
- Keep typed lower errors and selected-branch behavior unchanged.

**Non-Goals:**

- Do not implement full `compute_tx_type`, runtime `transform_type`,
  `cctx_type`, `EobU`, `coeffs()`, dequantization, inverse transforms,
  residual add, reconstruction, output, reference refresh, or inter prediction.
- Do not derive `PlaneTxType`, `is_inter`, `fsc_mode`, `enable_fsc`, or
  `coeff_cdf_q_ctx` from runtime frame/tile syntax.
- Do not change public APIs, crate dependencies, or encoder behavior.

## Decisions

1. Keep the condition derivation in `coeff_loop/use_fsc_branch.rs`.

   Rationale: the existing selector owns the `useFsc` branch boundary. Extending
   the same focused module keeps the derived-condition wrapper adjacent to the
   lower selector while avoiding new module churn.

   Alternative considered: add a separate `use_fsc_condition.rs` module. That
   would be reasonable if the derivation needed more state, but the condition is
   one boolean expression over caller-resolved facts.

2. Represent nonzero condition facts separately from lower branch inputs.

   Rationale: the condition facts (`enable_fsc`, `plane_tx_type`, `plane`,
   `fsc_mode`, and `is_inter`) are not the same ownership boundary as the
   ordinary/FSC lower inputs. Keeping them separate makes it clear that this
   wrapper only derives `use_fsc` and then delegates.

   Alternative considered: mutate the existing `CoeffUseFscBranchNonZeroInput`
   to replace `use_fsc` with condition facts. That would remove a useful lower
   explicit selector boundary and force existing tests to exercise the derived
   path even when they need to compare direct true/false dispatch.

3. Treat `PlaneTxType == IDTX` as the only transform-type predicate.

   Rationale: the spec expression names `IDTX` directly. This wrapper should not
   infer related transform-class behavior or widen the condition.

## Risks / Trade-offs

- [Risk] Caller facts can still be contradictory because runtime syntax/state is
  not yet wired. Mitigation: tests SHALL prove only the derived selected branch
  executes, while docs and matrix notes keep runtime derivation incomplete.
- [Risk] Future runtime integration could confuse this two-way wrapper with the
  full spec block body. Mitigation: doc comments SHALL state that the wrapper
  derives only the `useFsc` boolean and delegates to existing staged branches.
