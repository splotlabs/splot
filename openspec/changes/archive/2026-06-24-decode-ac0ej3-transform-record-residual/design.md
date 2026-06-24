## Context

The current live `ac0ej3.ivf` probe reaches the active Wiener NS LR
transform-record path and fails while reading §5.20.7.27 `coeffs()` from record
derived transform geometry:

```json
{
  "unsupported_reason": "unsupported_wienerns_lr_live_transform_record_residual_parse",
  "spec_section": "5.20.7.27",
  "matrix_row": "ac0ej3-selectable-transform-records"
}
```

Temporary instrumentation from the previous investigation showed the underlying
coefficient failure is an EOB larger than the scan length, which means the next
fix must explain the transform size, scan derivation, or transform-block
ordering instead of merely relaxing bounds. AV2 §5.20.7.27 and §5.20.7.30 cap
coefficient scans at 32 samples per axis, so `eob > scan.len()` is only valid
evidence of an upstream geometry/order bug.

## Goals / Non-Goals

**Goals:**
- Identify the exact live record, plane, transform size, scan, and CCTX state
  that produces the EOB/scan mismatch.
- Correct the syntax-only Wiener NS LR transform-record residual handoff so it
  consumes the live stream with AV2-derived geometry/order.
- Add focused tests that prove the corrected path and preserve the existing
  fail-closed behavior for unsupported reconstruction/output.
- Advance the local ac0ej3 probe to the next structured unsupported frontier and
  record the evidence in the implementation and decoder-support matrices.

**Non-Goals:**
- Do not implement inverse transforms, residual addition, loop restoration,
  reference refresh, decoded output, or hash/Y4M equality.
- Do not broaden the general reconstruction-safe residual policy.
- Do not make unsupported EOB/scan combinations silently accepted.
- Do not change crate dependency direction, public APIs, or add dependencies.

## Decisions

1. **Debug before relaxing any residual guard.** The ordinary coefficient scan
   walker correctly rejects `eob > scan.len()`. The implementation will first
   locate the upstream plane/record/transform mismatch and only change the
   derivation or call ordering when it can be tied to AV2 §5.20.7.24,
   §5.20.7.25, §5.20.7.27, or §5.20.7.30.

2. **Keep the scope at the transform-record residual boundary.** This PR can
   include both luma record residual geometry and chroma/CCTX transform-block
   ordering if the live probe proves they are the same frontier. It will not
   cross into decoded sample production.

3. **Use policy-scoped admission.** The Wiener NS LR tx-skip handoff may consume
   residual syntax needed for stream sync and LR metadata. Reconstruction-safe
   callers must continue rejecting active syntax surfaces that are not yet
   sample-correct.

4. **Test the observed geometry/order fact, not only the live file.** The local
   `ac0ej3.ivf` probe is useful evidence, but committed tests must be
   self-contained and focused on the parser/runtime boundary so CI does not need
   local AVM/dav2d assets.

## Risks / Trade-offs

- **Risk:** A syntax-only admission could later be mistaken for reconstruction
  support.
  **Mitigation:** Keep row status partial, keep output paths unsupported, name
  non-goals in matrix/docs, and assert fail-closed reconstruction-safe behavior
  in tests.

- **Risk:** The live mismatch may reveal a larger transform partitioning issue.
  **Mitigation:** Keep this PR to the smallest complete geometry/order fix that
  advances the stream, and leave separate feature rows for broader transform
  partition or reconstruction work.

- **Risk:** Temporary instrumentation may drift into the final patch.
  **Mitigation:** Remove debug prints before committing and rely on structured
  diagnostics plus focused tests for durable evidence.
