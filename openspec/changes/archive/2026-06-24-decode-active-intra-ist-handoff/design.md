## Context

The local decoder mission stream now reaches the AV2 §5.20.7.29 intra IST branch during Wiener
NS LR transform-record derivation. The existing zero-frontier code reads
`sec_tx_type` and, for active values, reads `most_probable_stx_set` before
returning `unsupported_dctonly_residual_intra_sec_tx_type`.

For the LR tx-skip grid, the record handoff only needs the parsed transform
record's skip flag and EOB-derived zero/non-zero state. AV2 §7.20.4 consumes
the resulting `LrTxSkip` values for loop-restoration classification. Active
secondary inverse transforms change coefficient/sample reconstruction, but they
do not change whether `LrTxSkip[row][col]` is derived from
`skip_flag || (eob == 0)` after the transform syntax has been consumed.

## Goals / Non-Goals

**Goals:**

- Add a residual policy mode for LR tx-skip record derivation that consumes
  active intra IST `sec_tx_type` and the required intra
  `most_probable_stx_set` follow-up in AV2 §5.20.7.29 order.
- Preserve the current fail-closed active-IST diagnostic for paths that would
  use decoded coefficients, inverse transforms, reconstructed samples, or
  output.
- Carry active-IST metadata in the luma coefficient handoff so focused tests can
  prove the syntax was read.
- Advance the local decoder mission probe to the next honest unsupported runtime gate and
  record that evidence in tracking docs.

**Non-Goals:**

- No secondary inverse-transform runtime wiring, coefficient modification, or
  reconstructed sample output.
- No broad AV2 IST support for inter blocks, ADST_ADST kernels, CCTX, decoded
  frame samples, reference refresh, raw/Y4M output, or AVM/dav2d byte equality.
- No encoder work, new dependencies, crate dependency-graph changes, or
  invocation of external reference decoders from repo code.

## Decisions

- Split active IST handling by residual policy.
  `TransformToolResidualPolicy::AdmitDctOnly` remains reconstruction-safe and
  rejects active secondary transforms after consuming syntax. The LR tx-skip
  record path opts into an explicit active-IST handoff policy because it only
  derives skip/EOB records, not reconstructed samples.

- Preserve syntax synchronization before every unsupported diagnostic.
  When `sec_tx_type != 0`, the decoder reads `most_probable_stx_set` for intra
  blocks exactly where §5.20.7.29 requires it, then either records the metadata
  for LR tx-skip handoff or emits the existing unsupported reason for unsafe
  paths.

- Store only syntax metadata in `LumaCoeffBlock`.
  The handoff should expose `sec_tx_type` and `most_probable_stx_set` evidence
  without pretending to apply the §7.15.3 secondary inverse transform. The
  coefficient values remain the parsed pre-reconstruction quantized syntax.

- Keep the branch in `splot-decode`.
  The decision depends on tile syntax, transform type, EOB, luma mode, and the
  runtime policy. Pulling it into `splot-recon` would blur the crate boundary:
  `splot-recon` already owns pure secondary-transform math, not tile syntax
  parsing or local decoder mission runtime frontier policy.

## Risks / Trade-offs

- [Risk] A future caller could accidentally use active-IST pre-transform
  coefficients for output.
  -> Mitigation: make active IST admission opt-in through an explicit policy and
  keep existing output/reconstruction gates rejecting active IST.

- [Risk] Metadata without coefficient transform support may look broader than it
  is.
  -> Mitigation: matrix, decoder-support, OpenSpec specs, and diagnostic text
  keep the row partial and explicitly exclude secondary inverse transforms and
  successful local decoder mission output.

- [Risk] The live stream may immediately hit a larger reconstruction/output
  blocker after this handoff.
  -> Mitigation: run the local probe after implementation and update the gate to
  the next real unsupported reason rather than predicting it.
