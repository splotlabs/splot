## 1. Implementation

- [x] 1.1 Add a shared decode-local § 5.20.7.30 coefficient scan-order helper and keep the ordinary branch behavior/error surface unchanged.
- [x] 1.2 Add the crate-private FSC branch scan-order wrapper that derives scan from `txSz` and `PlaneTxType`, validates generated table values, then delegates to the scan-extent wrapper.

## 2. Tests

- [x] 2.1 Add positive FSC scan-order equivalence coverage against the explicit scan-extent branch.
- [x] 2.2 Add fail-atomic FSC scan-order tests for all-zero routing, invalid transform-size table domain, and invalid derived scan shape.
- [x] 2.3 Run focused coefficient-loop tests.

## 3. Tracking and Documentation

- [x] 3.1 Add `DECODE-COEFF-FSC-BRANCH-SCAN-ORDER` to implementation, decoder-support, conformance coverage, and roadmap tracking.
- [x] 3.2 Regenerate feature/status and decoder support/conformance docs.

## 4. Validation

- [x] 4.1 Run OpenSpec validation for this change and all specs.
- [x] 4.2 Run feature/support/conformance drift checks and `git diff --check`.
- [x] 4.3 Run `env RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin cargo xtask ci`.
