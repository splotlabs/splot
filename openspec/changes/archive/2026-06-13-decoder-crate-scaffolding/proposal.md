## Why

The decoder mission now has maintainer approval to add the decoder/reconstruction
crate boundary that earlier roadmap slices deliberately left pending. Before any
runtime decode work lands, the workspace needs explicit `splot-recon` and
`splot-decode` crate scaffolds plus automation and documentation that enforce the
approved dependency direction.

## What Changes

- Add Feature ID `INFRA-DECODER-CRATE-SCAFFOLDING` for the approved decoder and
  reconstruction crate boundary.
- Add workspace library crates `splot-recon` and `splot-decode` with crate-level
  documentation, SPDX headers, workspace lint inheritance, and no public runtime
  reconstruction/decode API yet.
- Update workspace dependency-direction rules, architecture docs, decoder
  roadmap/diagnostics docs, support matrix, implementation matrix, and generated
  status documents to recognize the approved crates without claiming pixel
  reconstruction or decode support.
- Keep `splot decode` behavior unchanged and keep AVM/dav2d evidence local-only.

## Capabilities

### New Capabilities

- `decoder-support`: records the approved decoder/reconstruction crate scaffold
  and its dependency-direction boundary as repository infrastructure.
- `process`: records dependency-direction and coverage-threshold handling for
  the new workspace crates.

### Modified Capabilities

- `decoder-support`: distinguish crate scaffolding from runtime decode,
  reconstruction, hash output, Y4M output, or AV2 conformance support.

## Impact

- Affected crates: new `crates/splot-recon` and `crates/splot-decode` library
  scaffolds.
- Affected automation: root workspace manifest, dependency-direction rules, and
  coverage threshold exclusion regex for crates outside `splot-validate`.
- Affected docs: `AGENTS.md`, `docs/ARCHITECTURE.md`,
  `docs/DECODER-ROADMAP.md`, `docs/DECODER-DIAGNOSTICS.md`,
  decoder/feature status docs, and implementation/support matrices.
- No new external dependencies, no encoder-facing code changes, no CLI behavior
  changes, no AVM/dav2d source or runner, and no runtime decode implementation.
