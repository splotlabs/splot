## Context

`splot-encode` now has a private residual primitive that computes signed
source-minus-prediction samples for a checked 8-bit input block. The next stage
in the encoder pipeline is a forward transform, but AV2 does not normatively
define encoder transform search or forward-transform policy. Decoded
reconstruction remains normative and lives in `splot-recon`.

This change therefore introduces only a narrow private arithmetic handoff:
4x4 DCT_DCT DC-only transform for uniform residual blocks. It is sufficient to
prove the first transform/dequant/inverse round trip through `splot-recon`
without copying generated transform tables into `splot-encode` or adding a
direct `splot-tables` dependency.

No packet-producing path exists yet. This change must not emit syntax, mutate
writer state, choose modes, or claim Baseline Encoder Profile v1 support.

## Goals / Non-Goals

**Goals:**

- Add a stable `ENC-FORWARD-TRANSFORM-FOUNDATION` matrix row.
- Add a private `crates/splot-encode/src/forward_transform.rs` module.
- Support a crate-private 4x4 DCT_DCT DC-only transform for uniform residual
  blocks.
- Use explicit checked arithmetic and typed errors for unsupported input.
- Prove the no-op quant/dequant handoff by feeding the produced coefficient
  block into `splot-recon::inverse_transform_2d_outer`.
- Preserve the existing no-output encoder lifecycle.

**Non-Goals:**

- No broad DCT/ADST/FDST/identity/DDT forward transforms.
- No direct `splot-tables` dependency and no copied generated transform tables.
- No transform selection, quantization policy, coefficient scan/tokenization,
  CDF selection, range encoding, tile-body emission, packet output, CLI success
  path, rate control, speed-policy consumption, or public Baseline Encoder
  Profile v1 claim.
- No changes to `splot-core`, `splot-recon`, `splot-decode`, `splot-validate`,
  `splot-cli`, or the crate dependency graph.

## Decisions

### DC-only 4x4 DCT_DCT first

The first supported forward-transform input is a 4x4 residual block whose
16 samples are identical. The helper returns a 4x4 coefficient block with only
the DC coefficient populated. For the existing `splot-recon` 4x4 DCT_DCT inverse
path (`row_shift = 7`, `col_shift = 10`), a DC coefficient of
`residual_sample * 32` reconstructs the uniform residual exactly under a no-op
quant/dequant handoff.

This is deliberately not a broad transform. Non-uniform residual blocks return a
typed unsupported-transform error until a later PR introduces a full scalar
forward DCT or a direct generated-table dependency is approved.

### Keep forward transform in `splot-encode`

Forward transform is encoder policy/math, not decoder-visible reconstruction.
The implementation belongs in `splot-encode`. Tests may call `splot-recon`
inverse transform APIs to prove the produced coefficients reconstruct the
intended residual, but the production helper does not depend on decoder runtime
state or packet syntax.

### No table dependency in this slice

The full transform kernels are already generated in `splot-tables` and consumed
by `splot-recon`. Adding a direct `splot-encode -> splot-tables` dependency would
be a dependency-graph change and is not needed for a DC-only 4x4 proof. This PR
therefore avoids a new dependency and documents the narrow arithmetic scale.

## Flight manifest

- Change ID: `encoder-forward-transform-foundation`
- Feature IDs: `ENC-FORWARD-TRANSFORM-FOUNDATION`
- Base commit: `c91f92110b1658bcf8b1491d48488f5cad7e35e6`
- Depends on merged changes: `encoder-program-contract`,
  `encoder-recon-dependency`, `encoder-frame-input-views`,
  `encoder-context-state-machine`, `range-encoder-complete`,
  `encoder-syntax-ir`, `encoder-minimal-header-plan`,
  `encoder-speed-presets`, `encoder-residual-foundation`
- Exact files/directories owned by this PR:
  - `crates/splot-encode/src/forward_transform.rs`
  - `crates/splot-encode/src/lib.rs`
  - `crates/splot-encode/src/error.rs`
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `openspec/changes/encoder-forward-transform-foundation/**`
  - `openspec/specs/encoder-tools/spec.md`
- Exact files/directories forbidden to this PR:
  - `Cargo.toml`
  - `Cargo.lock`
  - `crates/splot-core/**`
  - `crates/splot-recon/**`
  - `crates/splot-decode/**`
  - `crates/splot-validate/**`
  - `crates/splot-cli/**`
  - `crates/splot-tables/**`
  - `fuzz/**`
  - `docs/spec/av2/**`
- Public APIs/types owned: none
- Matrix rows owned: `ENC-FORWARD-TRANSFORM-FOUNDATION`
- Generated files owned: `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`
- Open sibling PRs audited: none (`gh pr list --state open` returned `[]`)
- Changed-file intersection with each sibling PR: none
- Semantic overlap with each sibling PR: none
- Can build/test/merge directly onto main without another open PR: yes

## Risks / Trade-offs

- The helper intentionally rejects non-uniform blocks. Mitigation: matrix and
  docs describe the DC-only scope, and a later transform PR can add the broad
  scalar DCT path with its own proof.
- The scale factor is tied to the current 4x4 DCT_DCT inverse path. Mitigation:
  tests prove the exact no-op quant/dequant inverse round trip through
  `splot-recon`, and no public API commits to this internal type.
- This PR does not add `splot-tables` to `splot-encode`. Mitigation: avoid
  dependency-graph churn until a broad transform implementation actually needs
  the generated kernels.
