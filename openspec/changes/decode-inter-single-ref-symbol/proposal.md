## Why

Multi-reference inter decode (the gate to real content like local decoder mission) needs the AV2
§ 5.20.7.12 `read_single_ref` element: it selects `RefFrame[0]` for a
single-reference inter block when more than one reference is available. Two prior
investigations found that `read_single_ref` CANNOT be runtime-tested in isolation:
it is only read when `NumTotalRefs >= 2`, which requires at least two valid
reference slots — a larger multi-reference runtime brick (the § 7.7 two-valid-slot
feed plus a >= 3 frame reference-retention loop). So `read_single_ref` is the leaf
prerequisite that the multi-reference brick is blocked on.

The smallest verifiable closure is to add the entropy element and prove it
bit-exact through a `SymbolEncoder` round-trip — the established splot pattern for
an entropy element whose runtime is deferred (e.g. the coefficient `read_quant`
round-trips). The `DEFAULT_SINGLE_REF_CDF` table already exists in `splot-core`;
this brick wires it into the tile CDF subset, adds the `read_single_ref` tree
reader, and round-trip-proves it. It is loaded-but-unwired: the § 8.3.2
neighbour-derived context derivation and the runtime wiring (relaxing the
`NumTotalRefs == 1` gate) are the explicit follow-on (the multi-reference brick).

## What Changes

- Add Feature ID `DECODE-INTER-SINGLE-REF-SYMBOL` to the implementation matrix.
- Add `TileSingleRefCdf` (sourced from `DEFAULT_SINGLE_REF_CDF`,
  `[REF_CONTEXTS=3][REFS_PER_FRAME - 1=6][3]`) to the tile CDF subset, mirroring
  the existing `TileIsInterCdf` / `TileSingleModeCdf` / `TileDrlModeCdf` rows
  (selector, row/row_mut, § 8.2.4 averaging, frame-end count scaling).
- Add a crate-private `read_single_ref(...)` in `splot-decode` that reads the
  § 5.20.7.12 binary `single_ref` tree over `TileSingleRefCdf[ctx][ref]` with
  caller-supplied per-decision contexts and returns the selected `RefFrame[0]`
  (`ref` on the first `1` bit, else `NumTotalRefs - 1`), with typed errors only
  and panic-free. The § 8.3.2 neighbour-context derivation is caller-supplied (the
  round-trip drives the contexts directly).
- Prove the element bit-exact via a `SymbolEncoder` <-> `read_single_ref`
  round-trip across the full selection range and distinct per-decision contexts,
  with `exit_symbol()` consistency, plus typed-error and panic-free edge cases.
- Do NOT wire `read_single_ref` into the runtime decode path and do NOT relax the
  `NumTotalRefs == 1` gate; keep every existing fixture byte-identical.

## Capabilities

### New Capabilities
- `decode-inter-single-ref-symbol`: Reads the AV2 § 5.20.7.12 `single_ref`
  entropy element over `TileSingleRefCdf[ctx][ref]`, proven bit-exact by a
  `SymbolEncoder` round-trip; loaded-but-unwired pending the multi-reference
  runtime brick.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row.

## Impact

- Affected code is limited to the crate-private `splot-decode` tile CDF subset
  (`TileSingleRefCdf` rows + selector) and a new
  `runtime_minimal/inter/single_ref.rs` reader + round-trip tests, plus
  feature/support/coverage documentation and this OpenSpec change.
- No public API, dependency-graph, encoder, validator, or diagnostics changes. No
  runtime decode output changes: the element is loaded-but-unwired, so every
  existing inter and intra fixture decodes byte-identically.
- Out of scope (the explicit follow-on, the multi-reference runtime brick): the
  § 8.3.2 neighbour-derived `single_ref` context derivation
  (`av2_get_ref_pred_context`), relaxing the `NumTotalRefs == 1` gate, the § 7.7
  two-valid-slot reference feed, the >= 3 frame reference-retention loop, and
  `read_compound_ref` (the compound-reference sibling). The round-trip pins ONLY
  the § 5.20.7.12 tree shape and the `TileSingleRefCdf[ctx][ref]` indexing.
