## Context

`splot-recon` now has source-backed square and rectangular DC prediction in
`intra.rs`, plus workspace helpers. The file is close to the 1000-line soft
budget, and broader intra support should not keep growing it. AV2 §7.13.2.2
basic intra prediction is the next narrow primitive: it consumes already
prepared `LeftCol`, `AboveRow`, and `AboveRow[-1]` samples and produces the
`PAETH_PRED` block.

The PR #101 concurrency model remains binding: `splot-recon` is deterministic
and scheduler-free; future decode orchestration runs through `DecodeContext`
and `splot_parallel::WorkerPool`.

## Goals / Non-Goals

**Goals:**

- Add source-backed AV2 §7.13.2.2 basic/PAETH prediction for rectangular
  transform regions.
- Keep square support as ordinary `IntraRectBlockSize::from(square)` reuse.
- Keep the low-level primitive allocation-free over caller-provided edge slices.
- Add a workspace convenience helper without adding scheduler state or AV2 block
  availability policy.
- Add typed errors for unavailable workspace edges if needed.
- Update decoder support and feature tracking.

**Non-Goals:**

- No full `predict_intra()` dispatcher.
- No directional, smooth, DIP, subsampled DC, IBP, CfL, transform,
  dequantization, residual, filter, runtime output, or reference refresh.
- No runtime `splot decode` success path.
- No AVM/dav2d repo integration or required reference-tool execution.
- No crate dependency changes.

## Decisions

1. Use a new `crates/splot-recon/src/intra_basic.rs` module.

   Rationale: `intra.rs` is already line-budget constrained. PAETH can reuse
   public `IntraRectBlockSize` from the existing intra module while keeping the
   new algorithm and tests isolated.

2. Model prepared edges explicitly.

   Add an `IntraPaethEdges<'a, T>` input with:

   - `left: &'a [T]` of length `h`;
   - `above: &'a [T]` of length `w`;
   - `top_left: T`, corresponding to `AboveRow[-1]`.

   Rationale: §7.13.2.1 owns edge availability and fallback sample preparation.
   This primitive starts at §7.13.2.2 and therefore requires already prepared
   edge inputs rather than deciding `haveLeft`, `haveAbove`, MRL, tile boundary,
   or superblock semantics.

3. Provide caller-owned strided output first.

   Add `predict_intra_paeth_rect_into(bit_depth, size, edges, output, stride)`.
   It validates sample type, edge lengths, edge sample ranges, output stride and
   length, then fills caller storage. A scalar per-sample helper may stay
   private unless tests or workspace code need it.

4. Workspace helper stays local and policy-free.

   Add `CurrentFrameWorkspace::predict_intra_paeth_rect` only for in-storage
   top-left, left, and above neighbors. If the target touches the top or left
   storage boundary, return a typed reconstruction error rather than inventing
   AV2 fallback availability behavior. The helper may compute directly from
   immutable source indices inside the plane before writing target samples, or
   borrow/copy edge slices if the implementation stays bounded and typed.

## Risks / Trade-offs

- [Risk] The name "basic" in the spec corresponds to `PAETH_PRED`, which can be
  ambiguous to API users. → Use `paeth` in API names and cite §7.13.2.2 in docs.
- [Risk] Workspace helper could imply full AV2 edge-availability semantics. →
  Document that it only uses in-storage neighbors and errors at missing
  top/left boundaries.
- [Risk] Integer arithmetic can go negative for `base`. → Use a signed
  intermediate wide enough for all supported sample values and choose output
  samples from validated input values.
- [Risk] File size drift. → Keep new implementation in `intra_basic.rs` and run
  `cargo xtask check-source-lines`.
