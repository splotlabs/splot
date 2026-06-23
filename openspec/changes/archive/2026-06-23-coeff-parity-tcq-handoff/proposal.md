## Why

The staged coefficient branch still asks callers to provide `parity_hiding` and
`use_tcq` after `DECODE-COEFF-FRAME-FACTS-HANDOFF` has already carried the parsed
frame and sequence facts needed to derive them. AV2 § 5.20.7.27 defines those
booleans from frame `allow_*` flags, lossless state, plane, transform type, and
`useFsc`, so deriving them in the wrapper removes two more caller-resolved facts
before runtime `coeffs()` wiring.

## What Changes

- Add Feature ID `DECODE-COEFF-PARITY-TCQ-HANDOFF`.
- Extend the crate-private coefficient frame-facts packet with parsed
  `allow_tcq` and `allow_parity_hiding` from the frame-header lossless tail.
- Derive ordinary lower-branch `parity_hiding` and `use_tcq` from the parsed
  frame flags plus existing nonzero block facts before delegating to the base-q
  wrapper.
- Preserve all-zero routing: all-zero inputs still bypass frame/ordinary facts.
- Thread the parsed `allow_*` facts through tile payload frame facts and work
  units for future runtime `coeffs()` wiring.
- Add focused tests proving derivation, suppression by lossless/chroma/IDTX/FSC,
  equivalence to explicit lower inputs, and parser-fact propagation.
- Update implementation matrix, decoder support matrix, roadmap, generated
  status docs, decoder conformance coverage metadata, and the audit ledger.
- Non-goals: runtime `coeffs()` wiring, full `compute_tx_type`, runtime block
  syntax traversal, segment-map derivation, dequantization, inverse transform,
  residual add, reconstruction, output, reference refresh, encoder changes,
  dependency graph changes, and AVM/dav2d invocation.

## Capabilities

### New Capabilities

- `coeff-parity-tcq-handoff`: crate-private loaded-but-unwired coefficient
  derivation for § 5.20.7.27 `parityHiding` and `useTcq` before the staged
  base-q and `useFsc` handoffs.

### Modified Capabilities

- `decoder-support`: extend staged coefficient decode support with a partial row
  for deriving coefficient parity-hiding and TCQ branch facts from parsed frame
  flags.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/`, focused coefficient
  branch and tile-payload derivation tests, `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/DECODER-ROADMAP.md`, generated
  status/coverage docs, and decoder conformance coverage metadata.
- Public API impact: none; helpers and propagated facts remain crate-private.
- Diagnostics impact: none; runtime validation diagnostics remain unchanged
  because the runtime `coeffs()` loop still does not call this wrapper.
- Dependencies and licensing: no new dependencies and no licensing changes.
