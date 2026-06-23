## Why

The staged coefficient branch still requires callers to hand-copy frame and
sequence facts (`enable_fsc`, `enable_chroma_dctonly`, `reduced_tx_set`,
per-segment `Lossless`, and `base_q_idx`) into lower branch packets. Those facts
are already parsed before tile payload planning, so this change removes another
runtime caller surface before wiring real `coeffs()`.

## What Changes

- Add Feature ID `DECODE-COEFF-FRAME-FACTS-HANDOFF`.
- Add a crate-private loaded-but-unwired coefficient wrapper above
  `DECODE-COEFF-CDF-Q-CONTEXT-HANDOFF`.
- Add a small frame-facts packet derived from parsed frame/sequence facts:
  `enable_fsc`, `enable_chroma_dctonly`, frame `reduced_tx_set`,
  `LosslessArray[]`, and `base_q_idx`.
- Derive ordinary lower-branch `reduced_tx_set`, `enable_chroma_dctonly`,
  nonzero `lossless`, shared `enable_fsc`, and q-context `base_q_idx` inputs
  before delegating to the existing base-q wrapper.
- Preserve all-zero ordering: all-zero inputs bypass frame facts and continue
  through the existing ordinary all-zero path.
- Thread the parsed facts through crate-private tile-payload frame facts so a
  future runtime `coeffs()` caller can obtain them from the tile work unit.
- Add focused equivalence and parser-fact derivation tests.
- Update implementation matrix, decoder support matrix, roadmap, generated
  status docs, decoder conformance coverage metadata, and the audit ledger.
- Non-goals: runtime `coeffs()` wiring, full `compute_tx_type`, runtime block
  syntax traversal, CDF lifecycle refactors, dequantization, inverse transform,
  residual add, reconstruction, output, reference refresh, encoder changes,
  dependency graph changes, and AVM/dav2d invocation.

## Capabilities

### New Capabilities

- `coeff-frame-facts-handoff`: crate-private loaded-but-unwired coefficient
  frame-facts derivation before the staged base-q and `useFsc` handoffs.

### Modified Capabilities

- `decoder-support`: extend staged coefficient decode support with a partial
  row for deriving parsed frame/sequence facts before the coefficient branch
  handoff.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/`, focused coefficient
  branch and tile-payload derivation tests, `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/DECODER-ROADMAP.md`, generated
  status/coverage docs, and decoder conformance coverage metadata.
- Public API impact: none; helpers and propagated facts remain crate-private.
- Diagnostics impact: none; runtime validation diagnostics remain unchanged
  because the runtime `coeffs()` loop still does not call this wrapper.
- Dependencies and licensing: no new dependencies and no licensing changes.
