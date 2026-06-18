## Why

The encoder now has validated input frames, a deterministic lifecycle, and a
private syntax IR, but it still lacks a typed bridge from encoder-owned frame
state to the existing `splot-core` header writer surface. The next safe step is
to define that bridge without emitting bytes or claiming a public encode path.

## What Changes

- Add Feature ID `ENC-MINIMAL-HEADER-PLAN` for private minimal header planning.
- Introduce private `splot-encode` planning records for the Baseline v1
  sequence-header, first-frame-header, and single-tile tile-group/header
  decisions needed before coded tile payload work.
- Validate that header planning is limited to the current supported encoder
  input subset and rejects mismatched frame/config metadata with typed errors.
- Add focused tests proving the header plan is deterministic, bounded, and does
  not enable packet output through `Context::receive_packet`.
- Update the implementation matrix, generated status docs, encoder roadmap, and
  gap audit to record the new private planning layer.

Non-goals:

- No AV2 bytes, OBUs, Annex B, IVF, coded packets, or public `splot encode`
  success path.
- No coded tile body, coefficient tokenization, entropy CDF selection,
  reconstruction, transform, quantization, rate control, or mode decision.
- No `splot-core` writer API changes, dependency-graph changes, new
  dependencies, Cargo manifest edits, or spec-mirror edits.

## Capabilities

### New Capabilities

- `encoder-minimal-header-plan`: Private minimal header planning for the future
  encoder writer handoff.

### Modified Capabilities

- None.

## Impact

- Affected code: `crates/splot-encode/src/**` only.
- Affected docs/specs: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`,
  `docs/ENCODER-ROADMAP.md`, `docs/ENCODER-GAP-AUDIT.md`, and this OpenSpec
  change.
- Public API impact: none. New planning types stay private to `splot-encode`.
- Validator impact: none. No new bitstream is emitted and no validator behavior
  changes.
- Dependencies: none.
