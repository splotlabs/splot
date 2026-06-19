## Context

`DECODE-COEFF-FSC-BRANCH-SCAN-ORDER` leaves the FSC/IDTX branch with a smaller
but still inconsistent caller surface: `NonZeroCoeffBlockStartInput.eob` carries
tx-size EOB facts, `CoeffFscLevelPassConfig` carries adjusted dimensions and
`txSzCtx`, `CoeffFscContextCommitConfig` carries block geometry, and the wrapper
itself carries `txSz` for scan derivation. In AV2 § 5.20.7.27, all of those
facts are derived from the same `coeffs(plane, startX, startY, txSz)` setup.

## Goals / Non-Goals

**Goals:**

- Add `DECODE-COEFF-FSC-BRANCH-TX-SIZE-HANDOFF` as a loaded-but-unwired
  FSC/IDTX coefficient branch wrapper.
- Derive `Tx_Width[txSz]`, `Tx_Height[txSz]`, `Tx_Width_Log2[txSz]`,
  `Tx_Height_Log2[txSz]`, `Adjusted_Tx_Size[txSz]`, `Tx_Size_Sqr[txSz]`,
  `Tx_Size_Sqr_Up[txSz]`, `txSzCtx`, EOB context, FSC level config, context
  geometry, and scan order before calling the existing branch.
- Fail before symbol/CDF/context mutation on invalid `txSz`, invalid generated
  table values, non-luma routing, or block geometry inconsistent with raw
  transform dimensions.
- Prove equivalence against the existing scan-order wrapper when supplied with
  matching explicit facts.

**Non-Goals:**

- Do not derive runtime `useFsc`, full § 5.20.7.29 `compute_tx_type`, or
  `PlaneTxType`.
- Do not wire runtime `coeffs()`, dequantization, inverse transform, residual
  add, reconstruction, output, or reference refresh.
- Do not add dependencies, public APIs, CLI behavior, external decoder
  invocation, or broad conformance claims.

## Decisions

- Keep this wrapper in `fsc_quant_pass.rs`.
  - Rationale: it composes existing FSC branch boundaries and does not need a
    reusable public module.
- Validate block geometry against raw `Tx_Width[txSz] >> 2` /
  `Tx_Height[txSz] >> 2` before EOB symbol reads.
  - Rationale: the local block allocation is derived from block geometry in the
    lower EOB-start helper, so inconsistent geometry must be rejected before the
    branch consumes symbols or mutates CDFs.
- Derive level/sign config dimensions from `Adjusted_Tx_Size[txSz]`.
  - Rationale: AV2 `get_tx_row_col` and `idtx_sign` contexts use adjusted
    transform width/height, while scan order and `segEob` still use raw capped
    transform dimensions.

## Risks / Trade-offs

- The wrapper duplicates some transform-size table lookup logic already present
  in the ordinary branch.
  - Mitigation: keep it scoped to FSC facts and reuse existing FSC error variants
    where possible. A later shared tx-size utility can consolidate both branches
    once the runtime coefficient boundary is clearer.
- The wrapper still trusts caller-resolved `PlaneTxType`, `is_inter`, and
  `coeff_cdf_q_ctx`.
  - Mitigation: document those remaining runtime gaps in the matrix and roadmap.
