## Context

`splot-recon` currently owns immutable decoded frame/plane models, hash input,
Y4M writing for caller-supplied frames, and a scheduler-free reference-slot
container. It does not yet provide any scalar prediction primitive. The decoder
roadmap therefore still records scalar intra reconstruction as the only pure
tier1 `todo` area.

AV2 §7.13.2.10 defines DC intra prediction from available left and above edge
samples. The rectangular both-edge case uses `approx_divide`, which delegates to
the §7.13.3.22 `resolve_divisor` process and the `Div_Lut` table. That table is
not currently generated into the repository. This change therefore models only
the square transform-block subset where the both-edge denominator is
`w + h == 2 * w`, a power of two, so the result is equivalent to the spec's
power-of-two `resolve_divisor` path without adding a hand-transcribed table.

## Goals / Non-Goals

**Goals:**

- Add a source-backed `splot-recon` square DC intra prediction primitive for
  AV2 §7.13.2.10.
- Keep the API deterministic, allocation-bounded, panic-free, and scheduler-free.
- Validate edge lengths and sample values before producing output.
- Update decoder support and feature tracking so full scalar intra
  reconstruction remains incomplete while this first square DC primitive is
  recorded as supported.

**Non-Goals:**

- No rectangular DC prediction until `resolve_divisor` and `Div_Lut` are modeled.
- No full `predict_intra()` dispatcher or non-DC prediction modes.
- No dequantization, inverse transforms, residual addition, tile syntax
  traversal, runtime decode success, frame hashes, Y4M output, or reference
  refresh.
- No AVM/dav2d source, dependency, wrapper, script, CI job, or required local
  reference run.
- No scheduler state, worker pool, Rayon/crossbeam dependency, or decode queue
  in `splot-recon`.

## Decisions

### Public `splot-recon` Primitive

Add a small public prediction module owned by `splot-recon`. The API shape is:

- `IntraSquareBlockSize` for `log2_size` plus derived `width`, `height`, and
  sample count.
- `IntraDcEdges<'a, T>` for optional left and above edge sample slices.
- `predict_intra_dc_square_value(bit_depth, size, edges)` as the no-allocation
  scalar entry point for encoder RDO and future workspace callers.
- `predict_intra_dc_square_into(bit_depth, size, edges, output, stride)` for
  writing the predicted square into caller-owned strided storage.
- `SquareIntraPredictionBlock<T>` and
  `predict_intra_dc_square(bit_depth, size, edges)` as an owned convenience
  wrapper for tests and callers that want row-major samples.

This keeps future decoder orchestration free to call the primitive inside
`DecodeContext::pool().install(...)` without making `splot-recon` aware of
worker pools.

### Square-Only DC Math

For left-only and above-only cases, §7.13.2.10 uses `Round2(sum, log2H)` and
`Round2(sum, log2W)`. In the square subset these are both
`Round2(sum, log2_size)`.

For both-edge square blocks, `w + h == 1 << (log2_size + 1)`. The
`resolve_divisor` power-of-two branch in §7.13.3.22 yields the same result as
`Round2(sum, log2_size + 1)`, followed by `Clip1`. The implementation should
document this specialization and leave rectangular both-edge prediction
unsupported rather than approximating with ordinary integer division.

### Error Handling

Extend `ReconError` with prediction-specific, typed variants:

- invalid square block log2 size;
- edge length mismatch;
- prediction sample out of range;
- prediction allocation failure;
- output stride too small;
- output buffer too small;
- sample value not representable by the requested storage type.

These variants keep library code panic-free and avoid reusing plane-specific
errors for edge-sample inputs that are not yet tied to a concrete plane.

### No Decode Or CLI Wiring

This change should not add a `splot-decode -> splot-recon` dependency yet. The
primitive is a reusable reconstruction building block; runtime decode remains
plan-only unsupported until tile syntax, residual handling, output ordering, and
limit adaptation land in later changes.

## Risks / Trade-offs

- **Partial spec coverage:** square-only DC prediction is intentionally narrow.
  Mitigation: name the feature and support-matrix row as square DC, and record
  rectangular DC as a residual.
- **Future API churn:** a later workspace/frame API may need a mutable write
  surface. Mitigation: provide scalar and caller-owned strided output APIs now,
  with the owned block kept as a square-specific convenience wrapper.
- **Allocation behavior:** the owned convenience wrapper can fail to allocate.
  Mitigation: provide no-allocation scalar/strided APIs and use fallible
  reservation before resizing the owned block.
- **Reference evidence confusion:** AVM/dav2d raw MD5 evidence is not proof of
  this primitive. Mitigation: keep validation self-contained and record no new
  local reference evidence.
