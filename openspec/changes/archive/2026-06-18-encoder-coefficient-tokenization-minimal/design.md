## Context

`ENC-RESIDUAL-FOUNDATION`, `ENC-FORWARD-TRANSFORM-FOUNDATION`, and
`ENC-QUANTIZATION-V0` have landed as private, non-emitting `splot-encode`
arithmetic stages. The next missing bridge is the coefficient-token facts that a
future tile-body writer can consume. The generic AV2 section 8.2 symbol/range
encoder already exists in `splot-core`, and `splot-recon` already provides the
4x4 two-dimensional coefficient scan order for the current DCT_DCT subset.

This change is still not packet output. It covers the AV2 v1.0.0 coefficient
syntax frontier at `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`
and `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-28` only far enough
to prove ordered token facts for the existing 4x4 DCT_DCT DC-only quantized
block. Section 8.3 CDF selection is represented by explicit default-CDF row
selectors used in tests, not by a tile CDF lifecycle or full syntax planner.

## Goals / Non-Goals

**Goals:**

- Add a private `coefficient_tokenization` module for
  `ENC-COEFFICIENT-TOKENIZATION-MINIMAL`.
- Accept the current top-left neutral-spatial-context 4x4 DCT_DCT DC-only
  quantized block subset.
- Derive scan metadata, EOB, begin position, sign/magnitude facts, coefficient
  CDF q-context from qindex, and ordered entropy-token records for all-zero and
  DC-only base-symbol magnitudes.
- Prove token values can be written through the in-tree AV2 section 8.2
  `splot-core` symbol encoder with scoped default CDF rows, including the
  low-frequency EOB base CDF row for the DC coefficient, and decoded back to the
  same values.
- Preserve the no-packet invariant in `Context`.

**Non-Goals:**

- No public API, CLI success path, tile payload writer, packet output, or
  Baseline Encoder Profile v1 claim.
- No non-DC coefficients, chroma, inter blocks, FSC, IDTX, TCQ, parity hiding,
  transform selection, coefficient base-range extension, or `read_quant`
  magnitude extension beyond the declared minimal base-symbol tier.
- No non-top-left blocks or neighbor-derived spatial contexts.
- No tile CDF save/restore lifecycle, adaptive tile CDF ownership, or broad
  section 8.3 CDF selection implementation.
- No dependency graph change and no AVM/dav2d evidence; this helper emits no
  stream.

## Decisions

1. **Keep tokenization private and loaded-but-unwired.**
   The module is loaded from `splot-encode` and exposes only crate-private types
   and helpers. This matches the earlier residual, transform, and quantization
   foundations while avoiding a premature public encoder contract.

2. **Represent token facts instead of bytes.**
   The primary output is a structured tokenization plan containing scan/EOB
   metadata and ordered syntax-token records. Tests can encode those records
   with `splot-core::symbol_encoder::SymbolEncoder`, but the module itself does
   not own a tile payload buffer or packet boundary.

3. **Use recon scan order as the source of truth.**
   The 4x4 DCT_DCT subset uses `splot-recon`'s coefficient scan helper for the
   two-dimensional transform class rather than duplicating scan tables in the
   encoder.

4. **Start with all-zero and DC-only base-symbol magnitudes.**
   The decoder-visible coefficient loop has many branch points. This slice
   proves the first syntax ordering boundary and rejects unsupported non-DC
   coefficients or magnitudes that would require coefficient base-range or
   `read_quant` extension. Later tile-body work can add those branches with
   their own Feature IDs and CDF-state evidence.

5. **Use scoped default CDF rows for roundtrip proof only.**
   The roundtrip tests use generated default CDF rows from `splot-core` to
   encode and decode the token values. The subset derives q-context from qindex
   and uses the low-frequency EOB base CDF row for the DC coefficient, while
   keeping neighbor-derived spatial contexts, tile CDF lifecycle, and broad
   section 8.3 selector completeness out of scope.

## Flight Manifest

- Change ID: `encoder-coefficient-tokenization-minimal`
- Feature IDs: `ENC-COEFFICIENT-TOKENIZATION-MINIMAL`
- Base commit: `8c9c5e27230e7a5764d8e03357054148f0b980c5`
- Depends on merged changes: `encoder-residual-foundation`,
  `encoder-forward-transform-foundation`, `encoder-quantization-v0`,
  `range-encoder-complete`
- Exact files/directories owned by this PR:
  - `crates/splot-encode/src/coefficient_tokenization.rs`
  - `crates/splot-encode/src/error.rs`
  - `crates/splot-encode/src/lib.rs`
  - `crates/splot-encode/src/context.rs`
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `openspec/changes/encoder-coefficient-tokenization-minimal/**`
  - `openspec/changes/archive/2026-06-18-encoder-coefficient-tokenization-minimal/**`
  - `openspec/specs/encoder-tools/spec.md`
- Exact files/directories forbidden to this PR:
  - `crates/splot-core/**`
  - `crates/splot-decode/**`
  - `crates/splot-recon/**`
  - `crates/splot-validate/**`
  - `crates/splot-cli/**`
  - workspace manifests and `Cargo.lock`
  - AV2 spec mirror files under `docs/spec/av2/**`
- Public APIs/types owned: none
- Matrix rows owned: `ENC-COEFFICIENT-TOKENIZATION-MINIMAL`
- Generated files owned: `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`
- Open sibling PRs audited:
  - `#280` `codex/decode-coeff-ordinary-branch-plane-tx-type`
- Changed-file intersection with each sibling PR:
  - `#280`: expected generated/tracking overlap in
    `docs/IMPLEMENTATION-MATRIX.toml`, `docs/FEATURE-STATUS.md`, and
    `docs/SPEC-COVERAGE.md`
- Semantic overlap with each sibling PR:
  - `#280`: coefficient-domain decoder tx-class branch handoff only; this
    change is private encoder tokenization and does not depend on decoder code
- Can build/test/merge directly onto main without another open PR: yes, unless
  `#280` lands first; if it does, rebase and regenerate tracking docs before
  merge.

## Risks / Trade-offs

- [Risk] The tiny DC-only tokenization surface can be mistaken for a usable tile
  body encoder. -> Mitigation: keep it private, preserve `receive_packet`
  behavior, and make the matrix/docs explicitly exclude packet output.
- [Risk] Default-CDF roundtrips are narrower than a real tile CDF lifecycle. ->
  Mitigation: call them proof rows for token/symbol compatibility only and leave
  tile CDF state to later Feature IDs.
- [Risk] Rejecting base-range and `read_quant` extension limits the first
  quantized-block handoff. -> Mitigation: add typed errors and tests so later
  work can expand the accepted magnitude tier deliberately.
