## 1. Syntax Handoff

- [x] 1.1 Resolve active luma transform-type syntax to a retained `PlaneTxType` for the LR tx-skip record handoff.
- [x] 1.2 Thread the retained luma `PlaneTxType` into the staged ordinary coefficient branch so scan/class derivation is no longer DCT-only.
- [x] 1.3 Preserve fail-closed behavior for reconstruction-safe callers and unsupported transform-tool branches.

## 2. Tracking And Tests

- [x] 2.1 Add focused positive and negative tests for LR handoff admission, safe-policy rejection, and staged branch `PlaneTxType` propagation.
- [x] 2.2 Update the local decoder mission ignored CLI probe expectation to the next structured frontier.
- [x] 2.3 Update implementation matrix, decoder-support matrix, generated docs, and OpenSpec specs.

## 3. Validation

- [x] 3.1 Run the local `local-decoder-mission.ivf` probe and focused decode tests.
- [x] 3.2 Run OpenSpec validation, feature-status checks, decoder-support checks, and `cargo xtask ci`.
