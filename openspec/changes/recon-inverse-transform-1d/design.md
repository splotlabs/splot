## Context

`splot-recon` has the dequantization quantizer functions but no inverse
transform. AV2 § 7.15.2.1 is the kernel-based 1D inverse transform invoked per
row/column by the § 7.15.4 2D transform. The § 9.6 kernels now live in the
dependency-free `splot-tables` crate (`INFRA-SHARED-SPEC-TABLES`), so
`splot-recon` can consume them without depending on `splot-core`.

## Goals / Non-Goals

Goals:

- Implement § 7.15.2.1 exactly, consuming the shared kernels.
- Keep the primitive scheduler-free, panic-free, and dependency-correct.

Non-Goals:

- The other § 7.15 transforms (Walsh-Hadamard, identity, secondary, 2D
  orchestration),
  dequantization, residual addition, runtime decode, or reference refresh.

## Decisions

- **Model only the kernel types.** The § 7.15.4.1 Table 7.1 1D transform types
  are `DCT`(0), `IDT`(1), `ADST`(2), `FDST`(3), `DDTX`(4), `FDDT`(5). `IDT`
  routes to the separate § 7.15.2.3 identity transform, so
  `InverseTransform1dType` models only `Dct`/`Adst`/`Fdst`/`Ddtx`/`Fddt` — the
  types § 7.15.2.1 actually dispatches on. The caller maps the parsed transform
  type to this enum; the parsing (§ 5.20.8) stays elsewhere.
- **Reproduce the spec dispatch verbatim, including the size-dependent
  branches.** At length 4 the spec `else` routes everything but `DCT`/`ADST` to
  the FDST kernel; at length 32 the DCT kernel is used for every type (AV2
  defines no other length-32 1D kernel); `FDDT` indexes the DDTX kernel column in
  reverse (`kernel[j][sz - 1 - i]`). The implementation mirrors these exactly
  rather than rejecting the (otherwise unreachable) combinations, so it matches
  the spec for every input the § 7.15.4 caller can produce.
- **Caller-supplied shift and `colTx`.** § 7.15.2.1 takes `shift` and `colTx`
  from the § 7.15.4 orchestration (`Transform_Shift`, the row/column pass). This
  brick takes them as inputs; the orchestration is a later row.
- **Totality.** The matrix multiply accumulates in `i64` (no overflow for
  in-range dequantized inputs), and the § 7.15.2.1 `Clip3` bound keeps every
  output inside `i32`. Lengths other than 4/8/16/32 and output/source length
  mismatches return typed `ReconError` instead of panicking.

## Risks / Trade-offs

- The length-4 `else`-to-FDST and length-32 DCT-for-every-type behavior could
  surprise a caller that passes an out-of-spec (type, length) pair, but it is
  exactly what § 7.15.2.1 specifies; the § 7.15.4 caller never produces such
  pairs (`get_transform_1d_type` only yields `DDTX`/`FDDT` for length ≠ 4, and
  length-32 only carries `DCT`/`IDT`).

## Migration Plan

Additive, plus the new `splot-recon -> splot-tables` dependency edge (the first
consumer of the shared crate). No existing API changes; the runtime is
unaffected.

## Open Questions

None.
