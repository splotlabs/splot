## 1. Planning And Feature Tracking

- [x] 1.1 Validate the OpenSpec change before implementation.
- [x] 1.2 Advance `DECODE-TILE-CDF-SELECTION-BOUNDARY` in the implementation matrix (module/proof/notes; status stays partial).

## 2. Context Derivation Implementation

- [x] 2.1 Add `IntraYMode` (`is_directional`), `reconstruct_minimal_y_mode`, and `uv_mode_ctx` to `cdf/block_context.rs`.
- [x] 2.2 Refactor `consume_trace` to a sequential decode that reconstructs `YMode` and derives the `uv_mode` context; add the `UnsupportedYMode` typed error and its diagnostic mapping.
- [x] 2.3 Keep the supported subset to `y_mode_set == 0` non-directional indices; mark the directional / escape / second-mode / in-frame-neighbour paths deferred with `TODO(spec)`.

## 3. Tests

- [x] 3.1 Add unit tests for the `YMode` reconstruction (set 0 index 0 -> DC_PRED; the non-directional subset; rejection of unsupported inputs) and `uv_mode_ctx` (DC_PRED -> 0, directional -> 1).
- [x] 3.2 Confirm the no-output-change snapshot `block_symbol_frontier_accepts_minimal_fixture_trace` stays green.
- [x] 3.3 Run focused `splot-decode` tests plus clippy, doc, and decoder checks.

## 4. Documentation, Review, And PR Discipline

- [x] 4.1 Update implementation matrix, feature status, spec coverage, and OpenSpec artifacts.
- [ ] 4.2 Run `openspec validate tile-uv-mode-context-derivation --strict` and required local gates before commit/PR.
- [ ] 4.3 Create a ready PR only; do not create a draft PR.
- [ ] 4.4 After the final commit, request review and wait for completed latest-head review before merge.
