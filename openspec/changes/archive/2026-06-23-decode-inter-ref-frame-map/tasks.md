# Tasks: decode-inter-ref-frame-map

## 1. OpenSpec And Feature Scope

- [x] 1.1 Validate `decode-inter-ref-frame-map` with strict OpenSpec checks.
- [x] 1.2 Add Feature ID `AV2-7.7-GET-REF-FRAMES` to the matrix and reference it
      from `AV2-5.18.2-FRAME-HEADER-INFO`; document the at-most-one-valid-reference
      modeled subset and the deferred ≥ 2-reference scoring state.

## 2. § 7.7 Model

- [x] 2.1 Add `crates/splot-core/src/headers/frame/get_ref_frames.rs` with the
      typed `RefSlot` / `GetRefFramesInput` / `GetRefFrames` and the total,
      panic-free `get_ref_frames(checkRes)` implementing the full § 7.7 ranking
      (distinct-ref detect, resolution gate, scoring, drop, bubble sort, cut,
      restricted append) derived from the spec text, not AVM.
- [x] 2.2 Register the module in `crates/splot-core/src/headers/frame/mod.rs`.

## 3. Inter Parser Wiring

- [x] 3.1 Thread `OrderHint` into `InterFrameContext` (info.rs construction
      sites).
- [x] 3.2 Add `derive_implicit_ref_map` in inter.rs gating § 7.7 to the
      at-most-one-valid-reference case; wire it into the `get_ref_frames(0)`
      (mirror :4607) and `get_ref_frames(1)` (mirror :4647) call sites so the
      implicit map advances past `InterStop::UnmodeledDerivation`.

## 4. Tests

- [x] 4.1 Unit-test § 7.7 worked examples (minimal post-key single ref, distinct
      ranking, `ActiveNumRefFrames` cap, restricted append, resolution gate,
      `AllowedFrames` masking, `get_relative_dist` sentinels, FloorLog2).
- [x] 4.2 Add inter.rs control-level tests (one / zero valid slots reach the
      shared tail; two valid slots stay unmodeled).
- [x] 4.3 Add the inter.rs fixture-bytes proof on `syn-key-inter-64x64.ivf`
      (driving the public `parse_frame_header_core`): the inter header reaches
      `InterStop::ReachedSharedTail` with `NumTotalRefs == 1`,
      `ref_frame_idx == [0]`; and the two-valid-slots variant stays
      `InterStop::UnmodeledDerivation`.

## 5. Status Docs And Gates

- [x] 5.1 Regenerate `docs/FEATURE-STATUS.md` and `docs/SPEC-COVERAGE.md`.
- [x] 5.2 `cargo xtask ci` green; `openspec validate --all --no-interactive`
      green; zero deletions; intra + existing parse tests unaffected.
