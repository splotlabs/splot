## 1. Planning And Feature Tracking

- [x] 1.1 Validate the OpenSpec change before implementation.
- [x] 1.2 Advance `DECODE-TILE-CDF-SELECTION-BOUNDARY` in the implementation matrix (proof/notes; status stays partial).

## 2. Context Derivation Implementation

- [x] 2.1 Add `txb_skip_ctx_luma` and `v_txb_skip_ctx` implementing the § 8.3.2 `all_zero` formula (plane 0 and plane 2) over caller-supplied level context and geometry.
- [x] 2.2 Use them in `consume_trace` with the first-block level context derived as 0 (and `EobU == 0`); keep the transform-block geometry caller-asserted to the fixture-forced values with a `TODO(spec)`.
- [x] 2.3 Confirm the literals are forced by the fixture (empirical probe) and that the computed contexts match.

## 3. Tests

- [x] 3.1 Add unit tests for the luma context (filling transform -> 0, the Min-clamped level sum, the fsc branch) and the V context (the first-block value 3, the neighbour/chroma/EobU contributions).
- [x] 3.2 Confirm the no-output-change snapshot `block_symbol_frontier_accepts_minimal_fixture_trace` (since retired by `decode-minimal-fixture-avm-skip-polarity`) stayed green.
- [x] 3.3 Run focused `splot-decode` tests plus clippy, doc, and decoder checks.

## 4. Documentation, Review, And PR Discipline

- [x] 4.1 Update implementation matrix, feature status, spec coverage, and OpenSpec artifacts.
- [ ] 4.2 Run `openspec validate tile-txb-skip-context-derivation --strict` and required local gates before commit/PR.
- [ ] 4.3 Create a ready PR only; do not create a draft PR.
- [ ] 4.4 After the final commit, request review and wait for completed latest-head review before merge.
