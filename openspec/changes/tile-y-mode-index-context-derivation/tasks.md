## 1. Planning And Feature Tracking

- [x] 1.1 Validate the OpenSpec change before implementation.
- [x] 1.2 Advance `DECODE-TILE-CDF-SELECTION-BOUNDARY` in the implementation matrix (module/proof/notes; status stays partial).

## 2. Context Derivation Implementation

- [x] 2.1 Add `cdf/block_context.rs::YModeIndexContext` deriving the § 8.3.2 `y_mode_index` context.
- [x] 2.2 Model the single-block tile-origin out-of-frame case (`get_joint_mode` -> `DC_PRED` -> ctx 0); mark the in-frame neighbour lookup deferred with a `TODO(spec)` marker.
- [x] 2.3 Thread the derived ctx into `block_symbol.rs::minimal_trace_items` in place of the literal.

## 3. Tests

- [x] 3.1 Add unit tests for the tile-origin ctx (0), directional-neighbour ctx (1 and 2), and the non-directional boundary.
- [x] 3.2 Confirm the no-output-change snapshot `block_symbol_frontier_accepts_minimal_fixture_trace` (since retired by `decode-minimal-fixture-avm-skip-polarity`) stayed green.
- [x] 3.3 Run focused `splot-decode` tests plus clippy, doc, and decoder checks.

## 4. Documentation, Review, And PR Discipline

- [x] 4.1 Update implementation matrix, feature status, spec coverage, and OpenSpec artifacts.
- [ ] 4.2 Run `openspec validate tile-y-mode-index-context-derivation --strict` and required local gates before commit/PR.
- [ ] 4.3 Create a ready PR only; do not create a draft PR.
- [ ] 4.4 After the final commit, request review and wait for completed latest-head review before merge.
