## Why

The local-reference evidence manifest now contains real entries, and the
decoder support matrix cites those entries by `docs/LOCAL-REFERENCE-EVIDENCE.toml::id`.
Today the manifest checker verifies that manifest entries name existing
decoder-support rows, but the matrix checker does not verify that each matrix
pointer resolves to a manifest entry. Adding that reverse check keeps evidence
links from silently drifting after review.

## What Changes

- Extend the offline decoder-support/reference-evidence checks so matrix
  `local_reference_evidence` pointers of the form
  `docs/LOCAL-REFERENCE-EVIDENCE.toml::<evidence-id>` must resolve to committed
  manifest entries.
- Require cited evidence entries to point back at the row that cites them.
- Add focused xtask tests for valid links, missing evidence IDs, and
  non-reciprocal row references.
- Keep all checks metadata-only: no AVM/dav2d lookup, execution, network access,
  runtime decode, reconstruction, hash computation, or Y4M output.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: the portable local-reference evidence manifest requirement
  gains matrix-to-manifest cross-reference validation.

## Impact

- Affected automation: `xtask` decoder-support/reference-evidence validation.
- Affected specs/docs: `openspec/specs/decoder-support/spec.md`, archived
  OpenSpec change artifacts, and possibly generated decoder support status docs
  only if rendering text changes.
- Feature ID: `XTASK-LOCAL-REFERENCE-EVIDENCE-MANIFEST`.
- No crate dependency graph change.
- No validator, decode runtime, reconstruction, hash output, Y4M output, AVM,
  dav2d, script, or CI integration change.
