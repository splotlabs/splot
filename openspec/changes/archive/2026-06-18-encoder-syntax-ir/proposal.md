## Why

The encoder now has validated frame input, a real push/pull lifecycle, a
closed-loop reconstruction dependency, and a generic §8.2 symbol encoder, but
`splot-encode` still has no typed representation for planned syntax decisions.
Before header planning, transforms, quantization, or tile emission can be wired,
the encoder needs a private deterministic IR that separates bitstream-affecting
decisions from runtime scheduling and prevents writer mutation during planning.

Feature ID: `ENC-SYNTAX-IR`.

## What Changes

- Add a private `splot-encode` syntax-planning module with strongly typed
  structures for `SequencePlan`, `FramePlan`, `TilePlan`, `SuperBlockPlan`,
  `BlockDecision`, `PredictionDecision`, `TransformDecision`,
  `QuantizedCoefficients`, and ordered syntax/token events.
- Keep the IR deterministic: stable ordering, explicit indices/newtypes, and
  debug rendering that does not depend on thread count or map iteration order.
- Add bounded constructors and tests for ordering, dimension/count arithmetic,
  and failure-before-mutation behavior.
- Update `docs/IMPLEMENTATION-MATRIX.toml`, generated status/coverage docs, and
  encoder roadmap/gap audit notes for `ENC-SYNTAX-IR`.
- Do not generate headers, tile bytes, packets, or a public `splot encode`
  success path in this change.

## Capabilities

### New Capabilities

- `encoder-syntax-ir`: private deterministic encoder syntax-planning IR for
  future sequence/frame/tile/block/token planning.

### Modified Capabilities

- None.

## Impact

- Affected code: `crates/splot-encode/src/*` plus focused unit/property tests.
- Affected tracking/docs: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`,
  `docs/ENCODER-ROADMAP.md`, and `docs/ENCODER-GAP-AUDIT.md`.
- Public API: no new public packet-producing behavior; any exported types must
  remain planning-only and must not imply an encoded stream can be produced.
- Dependencies: no new crate dependencies and no dependency-graph changes.
- Diagnostics: no new user-facing CLI diagnostics; errors remain typed
  `splot-encode` library errors for invalid planning inputs if exposed through
  testable constructors.
