## 1. Planning Artifacts

- [x] 1.1 Add the `ENC-SYNTAX-IR` matrix row with scope, status, and proof placeholders.
- [x] 1.2 Keep proposal, design, and delta spec valid for the private non-emitting IR scope.

## 2. Syntax IR Implementation

- [x] 2.1 Add a private `splot-encode` syntax IR module and wire it into the crate without public re-exports.
- [x] 2.2 Implement typed indices/newtypes and bounded constructors for sequence, frame, tile, superblock, block, coefficient, and event planning records.
- [x] 2.3 Implement deterministic ordered storage and debug rendering for plans and syntax/token events.
- [x] 2.4 Return typed planning errors for out-of-order children, duplicate or zero coefficient entries, count-limit violations, and overflow.
- [x] 2.5 Preserve the existing encoder context lifecycle so packet production remains unimplemented.

## 3. Tests

- [x] 3.1 Add positive construction tests for sequence/frame/tile/superblock/block/token plans.
- [x] 3.2 Add negative tests for ordering, duplicate coefficients, zero coefficients, and count-limit/overflow failures.
- [x] 3.3 Add regression tests showing repeated construction produces stable debug output and invalid construction returns no partially mutated plan.
- [x] 3.4 Add a context regression test proving `receive_packet` still returns the existing unimplemented result.

## 4. Docs And Verification

- [x] 4.1 Update encoder roadmap and gap-audit docs for `ENC-SYNTAX-IR`.
- [x] 4.2 Regenerate `docs/FEATURE-STATUS.md` and `docs/SPEC-COVERAGE.md`.
- [x] 4.3 Run `openspec validate encoder-syntax-ir --no-interactive`.
- [x] 4.4 Run `cargo xtask feature-status`.
- [x] 4.5 Run `cargo xtask check-feature-status`.
- [x] 4.6 Run `cargo xtask ci`.
