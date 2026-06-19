## Context

The ordinary non-FSC coefficient branch now derives geometry, transform-size
dimensions, adjusted base-context dimensions, `txSzCtx`, and scan order before
delegating to the state-backed ordinary pass. The remaining staged fact in that
chain is `PlaneTxType`, which AV2 §5.20.7.27 obtains from §5.20.7.29
`compute_tx_type(plane, txSz, x4, y4)`.

The full `compute_tx_type` function is broad: lossless handling, luma `TxTypes`
state, inter chroma luma lookup, `get_tx_set`, directional wide-angle mapping,
and frame-level derivation of `enable_chroma_dctonly` / `reduced_tx_set` all
remain outside the current runtime frontier. The non-lossless intra chroma
non-directional path is much smaller and now has its table prerequisite: generated
`splot-core::tables::conversion::MODE_TO_TXFM`.

## Goals / Non-Goals

**Goals:**

- Add a loaded-but-unwired ordinary branch wrapper that derives `PlaneTxType`
  for non-lossless intra chroma non-directional `UVMode` from generated
  `Mode_To_Txfm`.
- Honor the caller-resolved `enable_chroma_dctonly` short-circuit before the
  `Mode_To_Txfm` lookup.
- Apply the inline AV2 §5.20.7.29 `Tx_Type_In_Set_Intra` membership table and
  fall back to `DCT_DCT` when the mapped transform is not allowed by the
  caller-resolved `txSet`.
- Preserve all-zero behavior and fail before mutating CDF rows, symbol state, or
  coefficient context state on unsupported/out-of-domain inputs.
- Keep the existing caller-resolved `PlaneTxType` path available for all other
  staged tests and future runtime integration.

**Non-Goals:**

- No full implementation of §5.20.7.29 `compute_tx_type`.
- No `get_tx_set` derivation from frame state.
- No luma `TxTypes` state, inter chroma luma lookup, lossless handling, or
  directional `wide_angle_mapping`.
- No runtime `coeffs()` wiring, output changes, dequantization, reconstruction,
  reference refresh, public API changes, or external decoder invocation.

## Decisions

1. Add a new wrapper instead of changing existing inputs in place.

   Existing staged tests still need a caller-resolved `PlaneTxType` path for
   luma, inter, lossless, and directional cases. A new wrapper lets this feature
   prove the `Mode_To_Txfm` subset without over-constraining future work.

2. Keep `txSet` caller-resolved for this brick.

   `get_tx_set` is a separate §5.20.8.3 function with frame-level inputs. This
   change focuses on the transform-type selection step that was unblocked by
   generated `Mode_To_Txfm`; a later brick can derive `txSet` and
   `enable_chroma_dctonly` from frame state.

3. Hand-write only the inline membership table needed by the helper.

   `Tx_Type_In_Set_Intra` is inline in §5.20.7.29 rather than in the generated
   §9 attachment. A small cited `const` table in `splot-decode` keeps provenance
   explicit and avoids changing the table generator for a non-attachment table.

4. Treat unsupported branches as typed errors.

   The helper is intentionally limited to non-lossless intra chroma
   non-directional mode selection. Luma, inter, lossless, directional modes,
   invalid `UVMode`, and invalid `txSet` fail before any mutable decode state is
   touched.

## Risks / Trade-offs

- Wrong inline table transcription -> Mitigation: table spot tests cover
  allowed and rejected mappings, including fallback to `DCT_DCT`.
- Overclaiming `compute_tx_type` support -> Mitigation: names, docs, matrix
  notes, and OpenSpec requirements call this a `Mode_To_Txfm` subset and list
  every deferred branch.
- Extra wrapper layering -> Mitigation: follow the existing ordinary-branch
  handoff pattern and keep the older caller-resolved path unchanged.
