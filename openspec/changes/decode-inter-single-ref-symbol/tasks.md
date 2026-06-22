# Tasks

## 1. OpenSpec and Feature Tracking
- [x] 1.1 Validate the `decode-inter-single-ref-symbol` OpenSpec artifacts.
- [x] 1.2 Add `DECODE-INTER-SINGLE-REF-SYMBOL` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 1.3 Add the corresponding decoder support row.

## 2. Spec transcription (derive from the mirror + AVM)
- [x] 2.1 Transcribe the § 5.20.7.12 `read_single_ref` tree from
      `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-12`: loop
      `ref ∈ 0..NumTotalRefs - 1`, read `single_ref`, return `ref` on the first
      `1`, else `NumTotalRefs - 1`.
- [x] 2.2 Transcribe the § 8.3.2 CDF indexing from
      `docs/spec/av2/1.0.0/08-parsing-process.md#s-8-3-2` (line 1094):
      `TileSingleRefCdf[ctx][ref]`, `ctx` computed as for `comp_ref`.
- [x] 2.3 Cross-check against AVM `read_single_ref` (`av2/decoder/decodemv.c`) and
      `av2_get_pred_cdf_single_ref` (`av2/common/pred_common.h`), which index
      `single_ref_cdf[ctx][ref]` identically.

## 3. CDF rows + reader
- [x] 3.1 Add `TileSingleRefCdf` to the tile CDF subset from
      `DEFAULT_SINGLE_REF_CDF`, mirroring `TileDrlModeCdf` (selector, row/row_mut,
      averaging, frame-end scaling).
- [x] 3.2 Add the crate-private `read_single_ref(...)` § 5.20.7.12 tree reader with
      caller-supplied contexts, typed errors only, panic-free.
- [x] 3.3 Do not wire it into the runtime decode path; do not relax the
      `NumTotalRefs == 1` gate.

## 4. Round-trip proof (asymmetric values)
- [x] 4.1 `SymbolEncoder` <-> `read_single_ref` round-trip across every selectable
      `RefFrame[0]` value (0..=NumTotalRefs - 1) and distinct per-decision
      contexts, asserting the decoded selection equals the encoded one and
      `exit_symbol()` is consistent.
- [x] 4.2 Sweep `NumTotalRefs` from 2 to REFS_PER_FRAME with distinct contexts.
- [x] 4.3 Add a context-indexing falsifiability witness, a `NumTotalRefs < 2`
      reject, a missing-context typed error, an out-of-range context typed error,
      and a short-buffer panic-free case.

## 5. Verify + gate
- [x] 5.1 Confirm falsifiability: a transposed CDF-row index (`ctx` <-> `ref`)
      breaks the round-trip tests; revert.
- [x] 5.2 All existing inter and intra fixtures decode byte-identical (no runtime
      output change).
- [x] 5.3 `cargo xtask ci` passes; `openspec validate --all` clean.

## 6. Deferred (out of scope, named follow-on)
- [ ] 6.1 The § 8.3.2 neighbour-derived `single_ref` context derivation
      (`av2_get_ref_pred_context` / `count_refs`).
- [ ] 6.2 Runtime wiring: relaxing the `NumTotalRefs == 1` gate, the § 7.7
      two-valid-slot reference feed, and the >= 3 frame reference-retention loop.
- [ ] 6.3 `read_compound_ref` (§ 5.20.7.11), the compound-reference sibling.
