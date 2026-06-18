## Context

`splot-decode` has a crate-private nonzero coefficient branch that initializes
the local § 5.20.7.27 `Level[]`, `QuantSign[]`, and `Quant[]` state and reads the
nonzero EOB syntax. The next coefficient-loop steps in AV2 § 5.20.7.27 use
`scan[c]` and `get_tx_row_col(pos, txSz)` before reading `coeff_base`,
`coeff_br`, signs, and `read_quant`.

`splot-recon` already owns a `get_scan` implementation, but the repository's
dependency rule limits the `splot-decode -> splot-recon` edge to runtime
reconstruction/hash/Y4M handoff. Entropy/CDF code must not import recon types.
This change therefore adds only a decode-local scan-walk boundary over
caller-resolved scan positions.

## Goals / Non-Goals

**Goals:**

- Add `DECODE-COEFF-SCAN-WALK`.
- Walk the non-FSC § 5.20.7.27 loops in reverse scan-index order
  (`c = eob - 1` down to `0`) using a caller-provided scan slice.
- Validate that `eob` is nonzero, fits the provided scan slice, and every visited
  position fits the initialized `TransformCoeffBlockState`.
- Return checked `c`, raster `pos`, `row`, and `col` facts for later
  base/BR/sign/read-quant bricks.
- Keep output unchanged and keep the helper crate-private.

**Non-Goals:**

- Do not derive or move AV2 `get_scan` tables in this brick.
- Do not import `splot-recon` from coefficient entropy/CDF code.
- Do not read `coeff_base`, `coeff_br`, signs, or `read_quant`.
- Do not write nonzero `Level[]`, `QuantSign[]`, or `Quant[]` values.
- Do not implement FSC/IDTX-specific traversal or transform-type computation.

## Decisions

1. Accept caller-provided scan positions instead of calling `splot-recon`.

   Rationale: the scan producer belongs to a later bridge that can resolve
   transform type and scan order without violating the crate dependency contract.
   This helper only enforces the local decode-side boundary that later symbol
   readers need.

   Alternative considered: move `get_scan` from `splot-recon` to
   `splot-tables`. That may still be appropriate later, but it has a broader
   ownership impact than this coefficient-loop brick needs.

2. Walk reverse scan indexes and expose facts, not callbacks.

   Rationale: returning a small vector of checked entries keeps tests direct and
   avoids introducing an abstraction around symbol reads before those reads exist.
   The maximum adjusted coefficient block has 1024 entries, matching the existing
   bounded local state.

   Alternative considered: an iterator borrowing the scan slice and block state.
   That would reduce allocation, but later symbol readers need mutable CDF and
   block state access; a simple owned entry list avoids borrow-shape churn in this
   preparatory brick.

3. Validate against `TransformCoeffBlockState` dimensions.

   Rationale: § 5.20.7.27 maps each raster `pos` through
   `get_tx_row_col(pos, txSz)`. For this helper, the initialized adjusted block
   state is the authoritative local extent. Out-of-range positions become typed
   coefficient-loop errors before any future symbol consumption.

## Risks / Trade-offs

- [Risk] The helper can only prove traversal over a supplied scan slice, not that
  the slice is the spec's `get_scan(txSz, txClass)` output. -> Mitigation: document
  that scan derivation remains a future caller responsibility and keep tests
  focused on traversal/order/bounds.
- [Risk] Returning a vector allocates per nonzero block. -> Mitigation: this is a
  private preparatory boundary with at most 1024 entries; a later output-changing
  coefficient loop can replace it with a streaming implementation once symbol
  readers are wired.
- [Risk] FSC uses a different forward walk after `bob = segEob - eob`. ->
  Mitigation: explicitly scope this brick to the ordinary non-FSC reverse walk and
  leave FSC for a separate feature row.
