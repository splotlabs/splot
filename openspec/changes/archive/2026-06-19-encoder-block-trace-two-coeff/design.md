## Context

This is the first MULTI-coefficient (eob > 1) block trace, composing the three
merged building blocks (`coeff_base_lf_luma_context`, `coeff_base_lf_token`, and the
`multi_coeff` accessors). The minimal case is eob = 2: a 4x4 DCT_DCT luma block with
one nonzero AC coefficient (level 1) at scan index 1 (raster position 4 = row 1
col 0 in the 4x4 2D scan order `[0, 4, 1, ...]`, derived via `coefficient_scan_order`)
and a zero DC at scan index 0 (raster position 0).

The AV2 §5.20.7.27 residual for this block, verified vs the decoder `coeff_loop.rs`:

- `all_zero = 0` (the block is coded).
- `eob_pt_16 = 1` → `eobPt = 2` → `eob = 2` (eobPt < 3, no extra bits).
- Base pass `c = eob-1..0`: the AC (c = 1, raster position 4 = row 1 col 0, low-frequency since `row+col = 1 < 4`) reads `coeff_base_eob` at context
  `coeff_base_eob_ctx(c=1) = 1`; the DC (c = 0) reads the non-EOB `coeff_base` at the
  §8.3.2 low-frequency context.
- Sign pass `c = eob-1..0`: the AC (level 1, pos (0,1) — not the luma DC, not a
  directional axis) reads a `sign_bit` bypass literal; the DC is zero, so it carries
  no sign.
- `read_quant` for both is below `maxLevel`, so no golomb bits.

The DC `coeff_base` context is data-dependent: it is the §8.3.2 low-frequency
neighbour-sum context, and the AC of level 1 at raster position 4 (below the DC) is the DC's significant
neighbour. `coeff_base_lf_luma_context(pos 0, …, Level[raster 4] = 1)` returns
`mag = 1` → `ctx = 1` → low-frequency `c == 0` band `ctx.min(8) = 1`. The composer
DERIVES this context (it constructs the AC's `Level[]` and calls the merged context
function) rather than hard-coding `1`, and a test asserts the DC token carries
context 1.

The ten-token trace and symbols:

| token | symbol |
| --- | --- |
| y_mode_set / y_mode_index / uv_mode | 0, 0, 0 |
| luma `all_zero` (coded) | 0 |
| `eob_pt_16` | 1 |
| AC `coeff_base_eob` (ctx 1, level 1) | 0 |
| DC `coeff_base` (ctx 1, level 0) | 0 |
| AC `sign_bit` (bypass) | 0 |
| U `all_zero` / V `all_zero` | 1, 1 |

Normative AV2 v1.0.0 sections:

- §5.20.7.27 `coeffs()` (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`).
- §8.3.2 `coeff_base` / `coeff_base_eob` contexts.
- §8.2.5 bypass `L(1)`.

## Goals / Non-Goals

**Goals:**

- The first multi-coefficient (eob = 2) block trace, with the DC `coeff_base`
  context DERIVED from the AC's `Level[]`, proven through one §8.2 coder.

**Non-Goals:**

- No eob > 2, no higher-magnitude AC (no `coeff_br`/golomb for the AC), no chroma
  multi-coefficient, no high-frequency `coeff_base`, no partition syntax, no
  tile-body emission, no packet output, no Baseline Encoder Profile v1 claim.
- The §8.2 roundtrip proves self-consistency only; conformance of the data-dependent
  context against a real decoder is at the packet milestone (AVM cross-check).

## Decisions

1. **Derive the DC context, do not hard-code it.** The composer constructs the AC's
   `Level[]` and calls `coeff_base_lf_luma_context`, so the trace exercises the merged
   derivation; a test asserts the DC token's routed context is the derived value.

2. **eob = 2 with a zero DC.** The simplest multi-coefficient block has the nonzero
   coefficient at scan pos 1 and a zero DC (which still reads a `coeff_base` symbol 0
   and no sign), keeping the magnitudes minimal (no `coeff_br`, no golomb, no DC
   sign) while still exercising the scan walk, the non-EOB `coeff_base`, and the AC
   `sign_bit`.

## Flight Manifest

- Change ID: `encoder-block-trace-two-coeff`
- Feature IDs: `ENC-INTRA-BLOCK-TRACE-TWO-COEFF`
- Base commit: `56be8645` (`feat(encode): multi-coefficient token accessors (#334)`)
- Depends on merged changes: `encoder-coeff-base-lf-context`,
  `encoder-coeff-base-lf-token`, `encoder-coeff-multi-coeff-tokens`.
- Exact files/directories owned by this PR:
  - `crates/splot-encode/src/block_symbol_trace.rs`
  - `crates/splot-encode/src/block_symbol_trace_tests.rs`
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `openspec/changes/encoder-block-trace-two-coeff/**`
  - `openspec/changes/archive/2026-06-19-encoder-block-trace-two-coeff/**`
  - `openspec/specs/encoder-tools/spec.md`
- Exact files/directories forbidden to this PR:
  - all other crates; `crates/splot-encode/src/coefficient_tokenization.rs` and its
    submodules; `crates/splot-encode/src/error.rs`; `crates/splot-encode/src/lib.rs`;
    `crates/splot-encode/src/closed_loop.rs`
  - workspace manifests and `Cargo.lock`; AV2 spec mirror under `docs/spec/av2/**`
- Public APIs/types owned: none
- Matrix rows owned: `ENC-INTRA-BLOCK-TRACE-TWO-COEFF`
- Generated files owned: `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`
- Open sibling PRs audited: none open at base commit `56be8645`.
- Changed-file intersection with each sibling PR: none. If a decoder-mission PR
  lands first, sync `main`, regenerate the tracking docs (the merge changes the
  feature count), and re-gate BEFORE pushing.
- Semantic overlap with each sibling PR: none; private encoder composition.
- Can build/test/merge directly onto main without another open PR: yes.

## Risks / Trade-offs

- [Risk] The DC `coeff_base` context is data-dependent and the §8.2 roundtrip cannot
  validate it against a real decoder. -> Mitigation: the context is DERIVED via the
  merged, decoder-mirrored `coeff_base_lf_luma_context`; a test asserts the routed
  context; AVM cross-check is at the packet milestone.
