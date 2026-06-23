## Why

The ordinary and FSC coefficient branches now have separate loaded-but-unwired handoffs, but the AV2 section 5.20.7.27 branch point that chooses between them is still duplicated in callers. This change adds the next small decoder brick: a crate-private selector that consumes a caller-resolved `useFsc` fact and dispatches to the already validated ordinary or FSC branch without deriving broader frame state yet.

## What Changes

- Add Feature ID `DECODE-COEFF-USE-FSC-BRANCH-HANDOFF`.
- Add a crate-private coefficient branch selector in `splot-decode` that preserves the AV2 ordering: decoded `all_zero` remains handled before the `useFsc` split.
- Route all-zero inputs through the ordinary all-zero branch, nonzero `useFsc == false` through `apply_coeff_ordinary_branch_from_lossless`, and nonzero `useFsc == true` through `apply_coeff_fsc_branch_from_tx_size`.
- Add focused tests for all-zero ordinary preservation, ordinary nonzero delegation, FSC nonzero delegation, and failure behavior for contradictory caller facts.
- Update implementation matrix, decoder support matrix, roadmap, generated status docs, and decoder conformance coverage for the new partial decoder-support row.
- Non-goals: runtime `useFsc` derivation from `enable_fsc`, `PlaneTxType`, `plane`, `fsc_mode`, or `is_inter`; full `compute_tx_type`; `transform_type`; `cctx_type`; `EobU`; dequantization; inverse transform; residual add; reconstruction/output/reference integration; encoder changes; dependency graph changes; AVM/dav2d integration.

## Capabilities

### New Capabilities

- `coeff-use-fsc-branch-handoff`: crate-private loaded-but-unwired coefficient branch selector for caller-resolved AV2 `useFsc`.

### Modified Capabilities

- `decoder-support`: extend the staged coefficient decode support requirement with a loaded-but-unwired `useFsc` branch handoff.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/coeff_loop/`, focused coefficient branch tests, `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/DECODER-ROADMAP.md`, generated status/coverage docs, and decoder conformance coverage metadata.
- Public API impact: none; the helper remains crate-private and loaded-but-unwired.
- Diagnostics impact: none; runtime validation diagnostics remain unchanged because the runtime `coeffs()` loop still does not call this selector.
- Dependencies and licensing: no new dependencies and no licensing changes.
