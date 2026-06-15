## Context

`splot-recon` already exposes scheduler-free square DC intra prediction and a
current-frame workspace. The decoder roadmap and support matrix still list
rectangular DC prediction as planned, which blocks progress toward reusable
scalar intra reconstruction for encoder roundtrips.

The source behavior is AV2 v1.0.0 §7.13.2.10 in
`docs/spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-10`. Square blocks can
specialize the both-edge average to `Round2(sum, log2_size + 1)`. Rectangular
blocks cannot do that when `w + h` is not a power of two, so the implementation
must use the §7.13.3.22 resolve-divisor path through an `approx_divide`
equivalent for the both-edge case.

The PR #101 concurrency model remains binding: `splot-decode` owns runtime
parallel orchestration through `DecodeContext` and `splot_parallel::WorkerPool`;
`splot-recon` stays scheduler-free and must not grow worker-pool, Rayon,
crossbeam, global-pool, or queue dependencies.

## Goals / Non-Goals

**Goals:**

- Add `splot-recon` rectangular DC prediction for AV2 §7.13.2.10 over typed
  rectangular block geometry.
- Validate bit depth, sample type, edge lengths, edge sample ranges, output
  stride, output length, and allocation failure with `ReconError`.
- Add workspace helpers that extract in-storage left/above edges for rectangular
  blocks and write rectangular DC prediction into the workspace.
- Update the decoder support matrix, generated status, implementation matrix,
  and roadmap to record rectangular DC as supported.
- Keep all tests self-contained and independent of AVM/dav2d.

**Non-Goals:**

- No non-DC intra prediction modes, block availability semantics, tile syntax
  traversal, dequantization, inverse transforms, residual addition, runtime
  decoded-frame hashes, runtime Y4M output, reference refresh, or public runtime
  `splot decode` success path.
- No `splot-decode -> splot-recon` dependency edge in this change.
- No AVM/dav2d source, snippets, binaries, wrappers, scripts, build probes,
  dependencies, CI jobs, mandatory tests, or runtime invocation.
- No scheduler state in `splot-recon`.

## Decisions

1. Add rectangular geometry next to square geometry in `intra.rs`.

   Introduce an additive `IntraRectBlockSize` type with `log2_width`,
   `log2_height`, `width`, `height`, and `sample_count`. It will accept the same
   source-backed log2 range currently modeled by square DC (4 through 64
   samples per dimension) while allowing `log2_width != log2_height`. Keep
   `IntraSquareBlockSize` as the existing public type and add conversion from a
   square size to the rectangular type. This avoids a breaking API change.

   Alternative considered: replace `IntraSquareBlockSize` with a single
   rectangular type everywhere. That would create churn in existing callers and
   tests without improving the immediate decoder mission slice.

2. Reuse `IntraDcEdges` for rectangular edges.

   The existing edge container already stores optional left and above slices;
   rectangular validation can check left length against block height and above
   length against block width. This keeps the API small and preserves future
   encoder reuse.

   Alternative considered: add separate `IntraDcRectEdges`. That would duplicate
   the same edge availability model and complicate workspace edge conversion.

3. Implement both-edge rectangular averaging through local `approx_divide`.

   For `haveLeft && haveAbove`, compute `sum(LeftCol[0..h]) +
   sum(AboveRow[0..w])`, approximate-divide by `w + h`, and clip to the active
   bit depth before converting to storage. The local implementation will include
   the AV2 `Div_Lut` table needed by §7.13.3.22, typed as a private constant in
   `splot-recon`. The one-edge cases keep the existing `Round2(sum, log2H)` and
   `Round2(sum, log2W)` behavior, and no-edge uses the bit-depth midpoint.

   Alternative considered: use normal integer division. That would be simpler,
   but it would not be the AV2 approximate division process and would make future
   reconstruction evidence harder to trust.

4. Add allocation-free rectangular output APIs without changing square names.

   New APIs:

   - `predict_intra_dc_rect_value(bit_depth, size, edges)`
   - `predict_intra_dc_rect_into(bit_depth, size, edges, output, stride_samples)`

   Existing square APIs become thin wrappers over the rectangular implementation
   and keep their current names and behavior. A new owned rectangular block type
   is deferred until a real caller needs it; future encoder reuse primarily needs
   the scalar value and caller-owned output paths.

5. Extend the workspace with rectangular helpers.

   Add rectangular edge extraction, rectangular block writes, and
   `predict_intra_dc_rect` convenience methods using `IntraRectBlockSize`. Keep
   the existing square helpers as wrappers. The workspace will still only read
   in-storage edges; AV2 block availability, tile boundaries, and subsampled DC
   prediction remain future decode logic.

## Risks / Trade-offs

- [Risk] Hand-transcribing `Div_Lut` incorrectly would make rectangular both-edge
  output wrong. -> Mitigation: cite §7.13.3.22, keep the private table isolated,
  add tests for non-power-of-two denominators and exact square compatibility,
  and run clippy/tests.
- [Risk] New generic rectangular APIs could imply full intra reconstruction. ->
  Mitigation: docs and matrix explicitly state this is only DC prediction and
  runtime decode remains unsupported.
- [Risk] Workspace helpers might be mistaken for AV2 availability semantics. ->
  Mitigation: method docs state they only use in-storage neighboring samples and
  do not model block/tile availability.
- [Risk] `splot-recon` could accidentally gain scheduler dependencies. ->
  Mitigation: run `cargo xtask check-concurrency-policy` and
  `cargo xtask check-dependency-direction`.

## Migration Plan

This is an additive library change. Existing square APIs and tests remain valid.
Rollback is deletion of the new rectangular APIs, tests, and matrix/docs rows.

## Open Questions

None for this slice. AVM/dav2d local evidence is not required because the change
is a small source-backed primitive with self-contained tests and no committed
fixture generation.
