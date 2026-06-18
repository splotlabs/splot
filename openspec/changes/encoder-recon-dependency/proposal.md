## Why

The encoder program now requires an explicit closed-loop reconstruction boundary
before any real input model or encode lifecycle can reuse reconstruction types.
This change makes the single authorized dependency decision measurable and
enforced by the repository dependency gates.

## What Changes

- Add the direct `splot-encode -> splot-recon` dependency edge.
- Update dependency-direction policy and tests so the new graph is accepted while
  decoder, validator, CLI, and reverse recon edges remain forbidden.
- Update architecture and encoder docs to record that `splot-recon` is reusable
  lower-level reconstruction foundation for future encoder work.
- Add a Feature ID for this dependency-boundary change and regenerate status
  outputs.
- Keep `splot-encode` behavior unchanged: no frame input redesign, no public encode
  success, no reconstruction loop, and no entropy/tile implementation.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-program`: the reserved `encoder-recon-dependency` decision becomes an
  explicit approved dependency edge with no encode behavior change.
- `encoder-tools`: the closed-loop reconstruction reuse gate moves from "future
  dependency decision" to "dependency edge exists, integration still future".
- `process`: dependency-direction policy accepts the exact
  `splot-encode -> splot-recon` edge while continuing to reject broader graph
  changes.

## Impact

- Affected crates and manifests: `crates/splot-encode/Cargo.toml`, workspace
  dependency-direction expectations, and dependency-policy tests.
- Affected docs/status: `AGENTS.md`, `docs/ARCHITECTURE.md`,
  `docs/ENCODER-ROADMAP.md`, `docs/ENCODER-GAP-AUDIT.md`,
  `docs/IMPLEMENTATION-MATRIX.toml`, generated status/coverage outputs, and the
  OpenSpec encoder specs.
- Validator impact: none; this does not change parsing, validation diagnostics, or
  bitstream semantics.
- User-facing behavior: none; `splot encode` remains unimplemented.
