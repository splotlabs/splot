## Context

The FSC/IDTX coefficient branch currently has staged helpers through EOB, FSC level/sign/quant passes, context commit, branch composition, and `segEob` derivation from scan extent. The remaining synthetic inputs before runtime `coeffs()` can call this path include `useFsc`, scan order, transform facts, and context geometry.

The ordinary coefficient branch already derives `scan = get_scan(txSz, txClass)` from generated AV2 § 9.2 transform-size tables and a decode-local implementation of AV2 § 5.20.7.30. The FSC branch should use that same scan-order logic instead of keeping a separate caller-supplied scan table or duplicating the scan algorithm.

## Goals / Non-Goals

**Goals:**
- Add `DECODE-COEFF-FSC-BRANCH-SCAN-ORDER` as a loaded-but-unwired FSC/IDTX coefficient branch wrapper.
- Derive raw `Tx_Width[txSz]` and `Tx_Height[txSz]` from generated AV2 § 9.2 tables, derive `txClass = get_tx_class(PlaneTxType)`, and build the § 5.20.7.30 scan table before calling the existing scan-extent FSC wrapper.
- Share the scan-order algorithm with the ordinary branch while preserving the ordinary branch's existing error surface.
- Prove equivalence against the explicit scan-extent path and fail-atomic behavior for invalid table/shape inputs.

**Non-Goals:**
- Do not derive runtime `useFsc`, full § 5.20.7.29 `compute_tx_type`, `PlaneTxType`, FSC level config, or context geometry.
- Do not wire runtime `coeffs()`, dequantization, inverse transform, residual add, reconstruction, output, or reference refresh.
- Do not add dependencies, public APIs, CLI behavior, external decoder invocation, or broad conformance claims.

## Decisions

- Move the § 5.20.7.30 scan-order algorithm into a small shared coefficient-loop helper.
  - Rationale: ordinary and FSC paths must agree exactly on scan order; a shared helper prevents duplicated scan algorithms from drifting.
  - Alternative considered: duplicate the ordinary private helper in `fsc_quant_pass.rs`. Rejected because two copies of the same spec algorithm would make later bug fixes and audits riskier.
- Keep generated `txSz` table validation local to the FSC wrapper.
  - Rationale: this brick only needs raw width/height for scan order. Adjusted dimensions, `txSzCtx`, and context geometry remain separate staged facts.
  - Alternative considered: importing the ordinary tx-size dimension wrapper directly. Rejected because it carries ordinary-specific adjusted-size, EOB-context, base-context, and transform-type concerns that are not part of this FSC step.
- Preserve fail-atomic ordering.
  - Rationale: invalid `txSz`, invalid table values, invalid scan shapes, all-zero FSC routing, and non-luma FSC routing must fail before unintended symbol/CDF/context mutation.

## Risks / Trade-offs

- Shared helper changes ordinary branch internals → mitigate by retaining the ordinary wrapper's existing error variants and running focused ordinary/FSC coefficient tests.
- The FSC wrapper still trusts caller-resolved `PlaneTxType`, level config, and context geometry → mitigate by documenting those remaining gaps in the matrix and roadmap.
- The derived full scan length changes FSC tests from short synthetic scan tables to real scan tables → mitigate with equivalence tests against the scan-extent wrapper using matching 8x8 block geometry.
