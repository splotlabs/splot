## Context

The coefficient path now has two staged branch targets:

- `apply_coeff_ordinary_branch_from_lossless` handles the loaded ordinary non-FSC branch and its all-zero arm.
- `apply_coeff_fsc_branch_from_tx_size` handles the loaded FSC/IDTX nonzero branch after caller-resolved `PlaneTxType`, `is_inter`, `coeff_cdf_q_ctx`, block geometry, and `txSz`.

AV2 § 5.20.7.27 evaluates `all_zero` before deriving `PlaneTxType`, `scan`, and `useFsc`, and then dispatches nonzero blocks with `if (useFsc)`. This change adds only that staged dispatch boundary; it does not derive the runtime `useFsc` expression.

## Goals / Non-Goals

**Goals:**

- Add a crate-private `DECODE-COEFF-USE-FSC-BRANCH-HANDOFF` selector that accepts caller-resolved `use_fsc`.
- Preserve spec ordering by routing all-zero inputs through the ordinary all-zero branch and by only applying `use_fsc` to nonzero inputs.
- Reuse the existing ordinary lossless and FSC tx-size wrappers without copying their table, scan, CDF, or context logic.
- Keep errors typed and branch-local so lower ordinary/FSC errors are surfaced without panics or broad mutation.

**Non-Goals:**

- Do not derive `useFsc = enable_fsc && PlaneTxType == IDTX && plane == 0 && (fsc_mode || is_inter)`.
- Do not implement full `compute_tx_type`, `transform_type`, `cctx_type`, `EobU`, runtime `coeffs()`, dequantization, inverse transforms, residual add, reconstruction, output, reference refresh, or inter prediction.
- Do not change public APIs, crate dependencies, or encoder behavior.

## Decisions

1. Keep the selector in a new focused `coeff_loop/use_fsc_branch.rs` module.

   Rationale: the existing coefficient-loop files are near the source-line advisory budget. A focused module keeps the new boundary auditable without growing `fsc_quant_pass.rs` or `ordinary_pass/geometry.rs`.

   Alternative considered: extend `fsc_quant_pass.rs` or `ordinary_pass/geometry.rs`. That would make the branch selector look owned by one branch and would add more pressure to files already near the soft line limit.

2. Model nonzero input as caller-prepared ordinary and FSC lower-boundary inputs plus `use_fsc`.

   Rationale: this selector is the AV2 § 5.20.7.27 branch point, not a new tx-size, transform-type, or frame-state derivation layer. The lower wrappers already validate their own table, geometry, CDF, and symbol facts. Passing lower-boundary inputs avoids duplicating that logic.

   Alternative considered: derive the FSC block input from the ordinary `coeffs()` geometry inside the selector. That would duplicate tx-size table validation already owned by the FSC tx-size wrapper and widen the scope beyond the branch choice.

3. Return a small result enum that distinguishes ordinary and FSC branch outputs.

   Rationale: the future runtime `coeffs()` integration must know which local pass result was produced for later dequant/reconstruction handoff. Wrapping existing result types preserves the lower APIs and avoids flattening unrelated ordinary/FSC state.

4. Wrap lower errors in a selector-specific typed error.

   Rationale: callers can report whether failure came from ordinary or FSC routing while preserving the existing detailed typed errors. The selector itself should not invent new AV2 semantics.

## Risks / Trade-offs

- Lower-boundary inputs can contain contradictory unused facts. Mitigation: tests SHALL prove that only the selected branch is executed and that the selected lower wrapper's validation controls mutation.
- The selector is still loaded-but-unwired. Mitigation: matrix, support, roadmap, and conformance coverage SHALL keep runtime `coeffs()` and byte-exact decode incomplete.
- Result typing adds one more crate-private enum. Mitigation: keep it local and thin, only wrapping existing branch result types.
