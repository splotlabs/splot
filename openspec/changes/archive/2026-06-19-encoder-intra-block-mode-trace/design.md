## Context

`ENC-INTRA-MODE-SYMBOL-EMISSION` (luma `y_mode_set`/`y_mode_index`) and
`ENC-UV-MODE-SYMBOL-EMISSION` (chroma `uv_mode`) each emit a token plan tested in
isolation with fresh CDF state. A real coded intra block reads these as one
ordered sequence through a single entropy decoder, so the mode emitters must be
proven *composed*: the right order and shared CDF state across symbols.

AV2 §5.20.5.3 `intra_frame_mode_info()` calls `read_intra_y_mode()` then
`read_intra_uv_mode()` (before `residual()`), so the mode-info prefix is
`y_mode_set`, `y_mode_index`, `uv_mode`. This change composes that prefix and
proves the combined sequence through the in-tree AV2 §8.2 coder. Coefficient
symbols (which follow in `residual()`) are out of scope here and join the trace
in a later change.

Normative AV2 v1.0.0 sections:

- §5.20.5.3 Intra frame mode info syntax
  (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-5-3`) — the
  `read_intra_y_mode()`-before-`read_intra_uv_mode()` order.
- §8.2 symbol coding — the shared-CDF-state roundtrip proof.

## Goals / Non-Goals

**Goals:**

- Add `compose_minimal_intra_dc_block_mode_trace` for
  `ENC-INTRA-BLOCK-MODE-TRACE`, returning the ordered `y_mode_set`,
  `y_mode_index`, `uv_mode` token sequence by reusing the merged mode emitters.
- Prove the composed sequence writes through one §8.2 `SymbolEncoder` and decodes
  back through one `SymbolDecoder` to the same ordered symbols with shared CDF
  state.
- Preserve the no-packet invariant in `Context`.

**Non-Goals:**

- No coefficient/all-zero symbols, partition syntax, lossless/CfL predecessors,
  tile CDF lifecycle, tile-body emission, packet output, CLI success, or Baseline
  Encoder Profile v1 claim.
- No new mode coverage beyond the DC_PRED luma + DC chroma minimal tier.
- No dependency graph change and no AVM/dav2d evidence; this helper emits no
  stream.

## Decisions

1. **New `block_symbol_trace` module.** This is the home for the growing ordered
   block-symbol trace; it starts with the mode-info prefix and will add the
   coefficient symbols (in `residual()` order) in later changes. Keeping it
   separate from the per-symbol emitters makes the composition boundary explicit.

2. **Reuse the merged emitters and roundtrip.** The trace composes the outputs of
   `emit_minimal_dc_luma_intra_mode` and `emit_minimal_dc_chroma_uv_mode` and
   proves them with the existing `roundtrip_intra_mode_tokens`, which already uses
   a single `SymbolEncoder`/`SymbolDecoder` over a token slice with shared CDF
   state. No new CDF machinery or error variants are needed.

3. **Spec order, not the decoder minimal-trace order.** The composition follows
   AV2 §5.20.5.3 (`y_mode_set`, `y_mode_index`, `uv_mode`). The decoder's minimal
   block-symbol trace reads a luma transform symbol between `y_mode_index` and
   `uv_mode`, which diverges from §5.20.5.3; the encoder follows the spec.

## Flight Manifest

- Change ID: `encoder-intra-block-mode-trace`
- Feature IDs: `ENC-INTRA-BLOCK-MODE-TRACE`
- Base commit: `9ee332a5` (`feat(encode): add minimal chroma uv_mode symbol emission (#308)`)
- Depends on merged changes: `encoder-intra-mode-symbol-emission`,
  `encoder-uv-mode-emission`
- Exact files/directories owned by this PR:
  - `crates/splot-encode/src/block_symbol_trace.rs`
  - `crates/splot-encode/src/lib.rs`
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `openspec/changes/encoder-intra-block-mode-trace/**`
  - `openspec/changes/archive/2026-06-19-encoder-intra-block-mode-trace/**`
  - `openspec/specs/encoder-tools/spec.md`
- Exact files/directories forbidden to this PR:
  - all other crates; `crates/splot-encode/src/error.rs` (no new error variants);
    `crates/splot-encode/src/intra_mode_emission.rs` (reused read-only via its
    existing `pub(crate)` API)
  - workspace manifests and `Cargo.lock`; AV2 spec mirror under `docs/spec/av2/**`
- Public APIs/types owned: none
- Matrix rows owned: `ENC-INTRA-BLOCK-MODE-TRACE`
- Generated files owned: `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`
- Open sibling PRs audited: none open at base commit `9ee332a5`.
- Changed-file intersection with each sibling PR: none. If a decoder-mission PR
  lands first, sync `main`, regenerate the tracking docs, and re-gate.
- Semantic overlap with each sibling PR: none; private encoder composition.
- Can build/test/merge directly onto main without another open PR: yes.

## Risks / Trade-offs

- [Risk] The composition is small (three symbols). -> Mitigation: it proves a
  real new property — the merged emitters interleave correctly through one coder
  with shared CDF state — and establishes the trace module the full block trace
  grows into.
- [Risk] Reusing `intra_mode_emission` internals couples the modules. ->
  Mitigation: it consumes only the existing `pub(crate)` emit/roundtrip API
  read-only; it does not modify that module.
