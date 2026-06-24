## Why

The local ac0ej3 mission stream now proves the zero-IST frontier was too small:
the next real blocker is an active intra IST `sec_tx_type` while deriving Wiener
NS LR tx-skip records. For LR tx-skip handoff, that syntax must be consumed in
spec order so the decoder can advance to the next honest runtime frontier
without claiming secondary-transform reconstruction.

Feature ID: `DECODE-AC0EJ3-ACTIVE-INTRA-IST-HANDOFF`.

## What Changes

- Add a policy-scoped residual admission mode for ac0ej3 LR tx-skip record
  derivation that consumes active intra IST `sec_tx_type` and its required
  `most_probable_stx_set` follow-up.
- Preserve fail-closed behavior for general reconstruction/output paths when
  active intra IST would require secondary inverse transform semantics.
- Surface active intra IST metadata from the luma coefficient handoff so tests
  can prove the path read the syntax it is claiming.
- Update the implementation matrix, decoder-support matrix, local ac0ej3 CLI
  runtime gate, and OpenSpec support specs without claiming decoded samples,
  raw/Y4M output, reference refresh, AVM/dav2d byte equality, or broad IST
  support.

## Capabilities

### New Capabilities

- `ac0ej3-active-intra-ist-handoff`: Tracks the ac0ej3-specific active intra IST
  syntax handoff needed for LR tx-skip record derivation while reconstruction
  remains unsupported.

### Modified Capabilities

- `decoder-support`: Records the new partial ac0ej3 support row and associated
  local probe evidence without claiming successful decode output.

## Impact

- Affects `crates/splot-decode` transform-tool residual policy and the Wiener NS
  LR transform-record derivation path.
- Affects focused `splot-decode` residual/LR tests and the ignored local ac0ej3
  CLI gate test if the live stream advances.
- Affects `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated status documents, and OpenSpec
  specs.
- No new dependencies, dependency-graph changes, encoder behavior,
  secondary-inverse-transform runtime wiring, decoded output claim, or
  AVM/dav2d invocation.
