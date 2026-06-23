## Context

The current coefficient path has two staged wrappers above the ordinary and FSC
branch implementations. `apply_coeff_use_fsc_branch` accepts an explicit
nonzero `use_fsc` boolean and pre-built ordinary/FSC lower inputs.
`apply_coeff_use_fsc_branch_from_condition` derives that boolean from
caller-resolved AV2 section 5.20.7.27 facts, but still requires both lower input
packets to exist before branch selection.

The eventual runtime `coeffs()` call-site should not duplicate transform-block
geometry, q-context, and branch facts. This change narrows that surface without
claiming runtime integration: one shared nonzero fact packet is enough to derive
the condition and lazily build the selected lower input.

## Goals / Non-Goals

**Goals:**

- Add a crate-private `DECODE-COEFF-USE-FSC-SHARED-FACTS-HANDOFF` wrapper that
  accepts decoded all-zero input or one shared nonzero coefficient fact packet.
- Preserve all-zero ordering: all-zero inputs SHALL delegate to the ordinary
  all-zero branch without requiring nonzero shared facts.
- Derive the AV2 section 5.20.7.27 `useFsc` condition from shared nonzero facts.
- Build only the selected lower branch input, so non-selected branch facts cannot
  fail validation or mutate tile CDF, symbol, or coefficient context state.
- Keep the existing explicit selector and condition wrapper available for focused
  lower-boundary tests.

**Non-Goals:**

- Do not implement full `compute_tx_type`, runtime `transform_type`,
  `cctx_type`, `EobU`, `coeffs()`, dequantization, inverse transforms,
  residual add, reconstruction, output, reference refresh, or inter prediction.
- Do not derive `PlaneTxType`, `is_inter`, `fsc_mode`, `enable_fsc`,
  `coeff_cdf_q_ctx`, scan inputs, or transform-block geometry from runtime
  frame/tile syntax.
- Do not change public APIs, crate dependencies, or encoder behavior.

## Decisions

1. Keep the shared-facts wrapper in `coeff_loop/use_fsc_branch.rs`.

   Rationale: this module already owns the `useFsc` branch boundary and the
   condition wrapper. Keeping the shared-facts handoff there avoids a new module
   for one tightly related dispatch layer.

   Alternative considered: add a separate `use_fsc_shared_facts.rs` module. That
   would be useful if runtime `coeffs()` state entered the wrapper, but this
   brick is still a narrow crate-private staging API.

2. Dispatch directly to the selected lower branch instead of delegating through
   `apply_coeff_use_fsc_branch_from_condition`.

   Rationale: the existing condition wrapper takes pre-built lower ordinary and
   FSC inputs. Using it would keep the duplicated-fact and eager-construction
   surface this change is meant to remove. Direct dispatch keeps validation and
   mutation constrained to the selected branch.

   Alternative considered: make the existing condition wrapper generic over
   lazy closures. That would add abstraction around a two-branch crate-private
   helper and complicate focused tests without improving the current runtime
   handoff.

3. Derive the FSC block input from shared geometry only after `useFsc` is true.

   Rationale: AV2 section 5.20.7.27 uses `Tx_Width[txSz]` and
   `Tx_Height[txSz]` for the coefficient block geometry. Looking those generated
   table values up only on the FSC path prevents false ordinary paths from
   failing on non-selected FSC-only facts.

   Alternative considered: add a public helper to the lower FSC module for block
   derivation. The existing lower module helpers are intentionally private; a
   small local derivation keeps this wrapper independent and focused.

## Risks / Trade-offs

- [Risk] The shared fact packet can still contain caller-resolved facts that
  runtime syntax has not proven. Mitigation: keep the row partial and explicitly
  defer runtime fact derivation in docs, matrix notes, and tests.
- [Risk] Branch-local table lookup duplication can drift from lower FSC behavior.
  Mitigation: derive only `AllZeroCoeffBlockInput` fields from the same generated
  `Tx_Width` / `Tx_Height` tables used by sibling wrappers and compare behavior
  against the explicit lower FSC path in tests.
- [Risk] Future runtime integration could treat this as full `coeffs()` support.
  Mitigation: the OpenSpec requirement, matrix row, and roadmap all state that
  the wrapper remains loaded-but-unwired and output-neutral.
