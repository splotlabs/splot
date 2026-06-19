## Context

The single-DC bricks used the position-only `coeff_base_eob` context. Multi-
coefficient blocks (eob > 1) read `coeff_base` (non-EOB) for the lower-scan
coefficients, whose §8.3.2 context is a neighbour-sum of the already-decided
`Level[]` magnitudes. This is the hardest piece of the multi-coefficient path, so
it lands as an isolated, unit-tested primitive before the consuming trace brick.

The derivation mirrors the decoder's `CoeffBaseContext::select` low-frequency luma
branch (`crates/splot-decode/src/tile_payload/cdf/coeff_context.rs:365-451`),
verified against AV2 §8.3.2:

- `row = pos >> bwl`, `col = pos - (row << bwl)`.
- For each of `SIG_REF_DIFF_OFFSET_NUM = 5` luma neighbours at
  `SIG_REF_DIFF_OFFSET[classIdx][idx]`: `refRow = row + off[0]`,
  `refCol = col + off[1]`; if `refRow < txh && refCol < txw` and the flat index is
  in range, add `min(Level[flat], magLimit)`.
- `magLimit = 5` for the LF near-DC samples (`(classIdx == 0 || idx < 2)` and not
  the parity-hidden DC), else `3`.
- `ctx = (mag + 1) >> 1`.
- LF luma 2D mapping: `c == 0` → `ctx.min(8)`; `row + col < 2` → `ctx.min(6) + 9`;
  else → `ctx.min(4) + 16`. LF luma horiz/vert (`classIdx != 0`): keying on `col`
  (horiz) or `row` (vert), `lidx == 0` → `21 + ctx.min(6)`, else → `21 + 7 +
  ctx.min(4)` (`LF_SIG_COEF_CONTEXTS_2D = 21`).

Shared table: `splot_core::tables::conversion::SIG_REF_DIFF_OFFSET`
(`[[[i32; 2]; 5]; 3]`, 2D = `[[0,1],[1,0],[1,1],[0,2],[2,0]]`) — the same § 9 table
the decoder uses, so the encoder mirror cannot drift from the offsets.

Worked case the consuming brick uses (eob=2, AC level 1 at pos 1, DC at pos 0):
the DC's neighbour `(0,1) = pos 1` has `Level = 1` → `mag = min(1,5) = 1`,
`ctx = (1+1)>>1 = 1`, LF `c == 0` → `ctx.min(8) = 1`. So the DC `coeff_base_lf`
context is `1`.

## Goals / Non-Goals

**Goals:**

- A total, panic-free `coeff_base_lf_luma_context` mirroring the decoder LF luma
  branch, with unit tests pinning representative `Level[] → ctx` cases.

**Non-Goals:**

- No `coeff_base` token, CDF row, or emission (the consuming trace brick adds
  those). No chroma (UV) or parity-hidden DC context. No high-frequency (`coeff_base`
  non-LF) context — only the LF luma branch. No packet output.
- Conformance vs a real decoder is established when the context is wired into a
  trace and cross-checked vs AVM at the packet milestone; this brick only mirrors
  the §8.3.2 formula and validates it with unit tests.

## Decisions

1. **Luma LF scope.** The eob > 1 minimal trace (4x4 DCT_DCT luma, both coefficients
   low-frequency) needs only the LF luma branch. The function is scoped to that and
   documents chroma / parity-hidden / high-frequency as out of scope, keeping the
   surface small and the test burden focused.

2. **Import the shared offset table.** Using `splot_core`'s `SIG_REF_DIFF_OFFSET`
   (not a hand-copied table) means the encoder mirror tracks the same § 9 data as
   the decoder, eliminating an entire class of drift.

3. **Loaded but unread.** No caller yet; the eob > 1 trace brick consumes it. This
   isolates the complex derivation for focused review before it affects any emitted
   bits.

## Flight Manifest

- Change ID: `encoder-coeff-base-lf-context`
- Feature IDs: `ENC-COEFF-BASE-LF-CONTEXT`
- Base commit: `f12f1f27` (`feat(encode): golomb-prefix tail completes the luma DC magnitude vocabulary (#329)`)
- Depends on merged changes: none beyond the current `splot-core` § 9 tables.
- Exact files/directories owned by this PR:
  - `crates/splot-encode/src/coefficient_tokenization.rs`
  - `crates/splot-encode/src/coefficient_tokenization_tests.rs`
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `openspec/changes/encoder-coeff-base-lf-context/**`
  - `openspec/changes/archive/2026-06-19-encoder-coeff-base-lf-context/**`
  - `openspec/specs/encoder-tools/spec.md`
- Exact files/directories forbidden to this PR:
  - all other crates; `crates/splot-encode/src/block_symbol_trace.rs`;
    `crates/splot-encode/src/error.rs`; `crates/splot-encode/src/lib.rs`
  - workspace manifests and `Cargo.lock`; AV2 spec mirror under `docs/spec/av2/**`
- Public APIs/types owned: none
- Matrix rows owned: `ENC-COEFF-BASE-LF-CONTEXT`
- Generated files owned: `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`
- Open sibling PRs audited: none open at base commit `f12f1f27`.
- Changed-file intersection with each sibling PR: none. If a decoder-mission PR
  lands first, sync `main`, regenerate the tracking docs (the merge changes the
  feature count), and re-gate BEFORE pushing.
- Semantic overlap with each sibling PR: none; private encoder primitive.
- Can build/test/merge directly onto main without another open PR: yes.

## Risks / Trade-offs

- [Risk] The §8.3.2 LF context mapping is intricate. -> Mitigation: a faithful
  line-by-line mirror of the decoder's `select()` LF luma branch, the shared offset
  table, and unit tests pinning the bands and the consuming brick's exact case.
- [Risk] A loaded-but-unread primitive's proof is self-referential (mirror vs
  reading). -> Mitigation: documented as such; conformance vs AVM is established
  when wired at the packet milestone.
