## 1. Frontier Diagnosis

- [x] 1.1 Re-run the live `ac0ej3.ivf` decode probe and capture the current structured unsupported diagnostic.
- [x] 1.2 Add temporary local instrumentation to identify the failing record, plane, transform size, scan length, EOB, CCTX state, and call order.
- [x] 1.3 Remove temporary instrumentation after the root cause is captured in tests or structured diagnostics.

## 2. Residual Handoff Implementation

- [x] 2.1 Correct the Wiener NS LR transform-record residual geometry/order derivation with citations to AV2 §5.20.7.24, §5.20.7.25, §5.20.7.27, and §5.20.7.30.
- [x] 2.2 Preserve fail-closed behavior for invalid EOB/scan combinations and reconstruction-safe residual callers.
- [x] 2.3 Add focused positive and negative tests for the corrected transform-record residual subcase.

## 3. Tracking And Validation

- [x] 3.1 Update implementation matrix, decoder-support matrix, and generated status docs with the new live frontier evidence.
- [x] 3.2 Re-run the live `ac0ej3.ivf` probe and focused tests.
- [x] 3.3 Run OpenSpec validation, feature-status checks, and the repository acceptance gate.
