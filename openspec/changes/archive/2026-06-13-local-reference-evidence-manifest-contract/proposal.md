## Why

The decoder roadmap calls for portable local-reference evidence manifests, but
current evidence is still free-form prose in agent logs and matrix notes. Before
future tiny decoder fixtures and AVM/dav2d comparison records are added, the
repo needs a checked, non-executable manifest contract that can prove evidence
metadata is portable without requiring external decoders.

## What Changes

- Add Feature ID `XTASK-LOCAL-REFERENCE-EVIDENCE-MANIFEST` for the manifest
  schema and offline portability checker.
- Define a versioned TOML local-reference evidence manifest for future decoder
  fixtures, reference-tool revisions, sanitized command summaries, fixture
  hashes, decoded-output digests, and comparison assertions.
- Add a self-contained `xtask` checker that validates the manifest shape,
  repo-relative paths, fixture hashes, digest fields, cross-references, and the
  no-local-path / no-external-runner boundary without invoking AVM, dav2d,
  ffmpeg, the network, or `splot decode`.
- Update decoder roadmap, decoder support matrix, implementation matrix, and
  generated status docs so the contract is visible without claiming runtime
  decode, reconstruction, hash, Y4M, or AV2 conformance support.

## Capabilities

### New Capabilities

### Modified Capabilities

- `decoder-support`: add requirements for portable local-reference evidence
  manifests and their offline metadata validation boundary.

## Impact

- Affected docs: `docs/DECODER-ROADMAP.md`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated decoder/feature status docs,
  and implementation matrix entries.
- Affected automation: `xtask` gains an offline checker and unit tests, wired
  into the existing decoder-support gate.
- Affected OpenSpec: `decoder-support` gains a manifest-contract requirement.
- No crate/dependency graph changes, no new Cargo dependencies, no committed
  AVM/dav2d source or binaries, no external decoder wrappers, no scripts, no CI
  jobs that run external decoders, and no runtime decode/hash/Y4M behavior.
