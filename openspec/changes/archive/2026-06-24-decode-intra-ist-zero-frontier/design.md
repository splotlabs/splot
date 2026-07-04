## Context

The local decoder mission stream has advanced through selectable transform-record
handoff and DCT_DCT luma transform-type admission. It now reaches the AV2
§5.20.7.29 secondary-transform branch for intra IST and the current residual
guard rejects before reading `sec_tx_type`. That is too coarse for the next
decode brick: a stream that selects no secondary transform can remain
synchronized and continue through the DCT-only coefficient path.

The generated AV2 §9.3 default CDF rows already exist in `splot-core`. The
decoder tile CDF subset needs to expose the relevant §8.3.2 rows before
`general_intra_residual` can consume the syntax safely.

## Goals / Non-Goals

**Goals:**

- Add `TileSecTxTypeCdf` plus adjacent most-probable IST set CDF rows to the
  tile CDF subset, default copy, mutation, block-symbol read, and tile lifecycle
  coverage.
- Consume intra IST `sec_tx_type` for the DCT_DCT residual subset when AV2
  §5.20.7.29 conditions require it.
- Admit only `sec_tx_type == 0`; for non-zero intra IST, consume the
  `most_probable_stx_set` symbol in spec order and then fail closed with a
  stable unsupported reason.
- Keep inter IST and unsupported transform-type/reconstruction paths fail-closed.

**Non-Goals:**

- No secondary inverse-transform implementation.
- No IST coefficient semantics, reconstruction output, reference refresh, or
  successful local decoder mission decode claim.
- No encoder behavior, new dependencies, dependency graph changes, AVM/dav2d
  invocation, or broad AV2 transform-tool support.

## Decisions

- Use the existing `BlockCdfRows` path for IST CDF rows.
  The active transform-type CDFs already live there, so adding `sec_tx_type` and
  most-probable set rows to the same block-symbol subset keeps row selection,
  mutation, update-mode tests, and lifecycle averaging consistent. A local
  ad-hoc CDF row in `general_intra_residual` would bypass that coverage.

- Admit only zero secondary transform.
  `sec_tx_type == 0` preserves the existing DCT_DCT-only coefficient and
  reconstruction assumptions. Any non-zero value changes transform semantics, so
  the decoder reads the required follow-up intra symbol and then reports a
  precise unsupported feature.

- Keep the active-IST gate in the residual admission layer.
  The branch depends on transform size, EOB, inter/intra state, lossless state,
  luma mode, and selected transform type. Keeping it next to the existing
  transform-tool residual checks avoids creating a second partial transform
  dispatcher.

## Risks / Trade-offs

- [Risk] The real stream may select a non-zero secondary transform immediately.
  → Mitigation: read the dependent symbol in order and fail closed with a new
  diagnostic boundary rather than advancing with wrong coefficient semantics.

- [Risk] CDF shape mistakes could desynchronize symbol reads.
  → Mitigation: copy generated defaults by shape, add selector bounds tests, and
  include the rows in block-symbol update-mode and lifecycle coverage.

- [Risk] This remains an specific frontier and not broad AV2 support.
  → Mitigation: matrix and decoder-support rows stay partial and explicitly
  exclude successful decode, secondary inverse transforms, and output equality.
