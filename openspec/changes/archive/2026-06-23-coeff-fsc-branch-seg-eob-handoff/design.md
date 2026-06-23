## Context

`DECODE-COEFF-FSC-BRANCH-HANDOFF` added a loaded-but-unwired FSC/IDTX branch
that still accepts `segEob` and `scan` as separate caller facts. AV2 §5.20.7.27
defines `segEob = Min(32, Tx_Width[txSz]) * Min(Tx_Height[txSz], 32)`, and
§5.20.7.30 `get_scan(txSz, txClass)` returns exactly that many scan positions.

This change removes the independent `segEob` input at the next FSC wrapper
boundary by deriving it from the caller-resolved scan slice length. The scan
itself remains caller-resolved until a later transform-size/scan-order FSC
handoff derives it from `txSz` and `txClass`.

## Goals / Non-Goals

**Goals:**

- Add a crate-private FSC branch wrapper that accepts the existing branch facts
  except explicit `segEob`.
- Derive `segEob` as `scan.len()` immediately before delegating to
  `apply_coeff_fsc_branch`.
- Preserve existing mutation boundaries: all-zero and non-luma fail before EOB
  consumption; scan/segment failures occur after EOB start but before FSC
  level/sign/quant symbol reads.
- Record the new boundary in the implementation matrix, decoder support matrix,
  decoder conformance coverage, roadmap, and generated status docs.

**Non-Goals:**

- Do not derive or validate the scan order from `txSz` in this brick.
- Do not derive `useFsc`, `PlaneTxType`, `txClass`, `txSz`, frame flags, or
  runtime block syntax facts.
- Do not wire runtime `coeffs()` or change decoded output.
- Do not enter dequantization, inverse transform, residual add, reconstruction,
  reference refresh, inter prediction, filters, or public APIs.

## Decisions

1. Add a wrapper instead of changing `CoeffFscBranchNonZeroInput`.

   Rationale: the explicit-`segEob` boundary remains useful for focused lower
   layer tests and for preserving the already-reviewed mutation contract. The new
   wrapper becomes the higher-level staged entry point.

   Alternative considered: remove `seg_eob` from the existing input. That would
   churn the lower-level tests and make it harder to test invalid segment values
   directly.

2. Derive `segEob` from `scan.len()` rather than importing transform-size table
   helpers in this brick.

   Rationale: §5.20.7.30 scan length equals the §5.20.7.27 capped transform
   extent, so a caller-resolved scan already carries the segment extent. The next
   scan-order brick can derive the scan from generated tables once that scope is
   isolated.

   Alternative considered: move the ordinary branch transform-size helpers into
   a shared module and derive scan order now. That is a broader refactor and
   should be a separate brick.

3. Keep errors typed at the FSC branch boundary.

   Rationale: the wrapper delegates to the existing `CoeffFscBranchError` so
   callers keep a single error type for FSC branch staging. A short scan naturally
   surfaces through the existing checked scan-walk error after EOB start.

## Risks / Trade-offs

- [Risk] A caller can still provide a scan slice that is not the true
  `get_scan(txSz, txClass)` order.
  -> Mitigation: document that scan remains caller-resolved and keep runtime
  scan-order derivation as an explicit remaining gap.
- [Risk] Deriving `segEob` from a malformed short scan moves the failure to the
  EOB-vs-segment check.
  -> Mitigation: add regression coverage proving no FSC level/sign/quant symbols
  are read after that failure.
