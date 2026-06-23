## 1. Implementation

- [x] 1.1 Add a crate-private FSC tx-size handoff input and wrapper that derives
      EOB context, adjusted level config, context geometry, and scan order before
      delegating to the existing scan-order wrapper.
- [x] 1.2 Validate generated transform-size table values and raw block geometry
      before any symbol/CDF/context mutation.

## 2. Tests

- [x] 2.1 Add positive equivalence coverage against the explicit scan-order FSC
      branch.
- [x] 2.2 Add fail-atomic coverage for all-zero routing, non-luma routing,
      invalid `txSz`/table values, and inconsistent block geometry.
- [x] 2.3 Run focused FSC/coefficient-loop tests.

## 3. Tracking and Documentation

- [x] 3.1 Add `DECODE-COEFF-FSC-BRANCH-TX-SIZE-HANDOFF` to implementation,
      decoder-support, conformance coverage, and roadmap tracking.
- [x] 3.2 Regenerate feature/status and decoder support/conformance docs.

## 4. Validation

- [x] 4.1 Run OpenSpec validation for this change and all specs.
- [x] 4.2 Run feature/support/conformance drift checks and `git diff --check`.
- [x] 4.3 Run `env RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin cargo xtask ci`.
