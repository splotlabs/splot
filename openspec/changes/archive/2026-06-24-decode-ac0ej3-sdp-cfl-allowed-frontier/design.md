## Context

The active ac0ej3 runtime path is inside the key-frame Wiener NS LR selectable
transform-record handoff. The previous slice moved past the narrow luma
`BLOCK_4X32` record gate; the live probe now stops in §5.20.5.6 chroma
mode-info at `unsupported_wienerns_lr_live_transform_record_uv_mode`.

The root cause is state, not a new valid `uv_mode` value. AV2 §5.20.3.1 derives
`CflAllowedInSdp` during SDP partition traversal from the top 64x64 luma and
chroma partition decisions. AV2 §5.20.5.6 then disables both CfL and MHCCP in
`CHROMA_PART` intra leaves when `CflAllowedInSdp == 0`. The current runtime
only checks chroma tools and block dimensions, so it can read an `is_cfl` symbol
that the bitstream did not code, shifting the following `uv_mode` read.

## Goals / Non-Goals

**Goals:**

- Retain enough §5.20.3.1 SDP state in partition traversal to expose
  `CflAllowedInSdp` on chroma-part block frontiers.
- Use that state in §5.20.5.6 mode-info decoding to skip `is_cfl` and MHCCP
  syntax when the spec disables them for SDP chroma leaves.
- Prove the local ac0ej3 probe advances past the current
  `unsupported_wienerns_lr_live_transform_record_uv_mode` gate.
- Keep unsupported features fail-closed and structured.

**Non-Goals:**

- No broad CfL prediction, MHCCP prediction, chroma reconstruction, decoded
  sample population, loop-restoration filtering, output, reference refresh, or
  ac0ej3 success claim.
- No dependency graph changes.
- No changes to AV2 syntax tables or the spec mirror.

## Decisions

1. **Store `CflAllowedInSdp` in partition traversal, not in chroma mode decode.**
   The value depends on luma root partition shape, chroma root partition shape,
   and the top-luma follow state defined in §5.20.3.1. A dimension-only helper
   in `general_intra_block.rs` cannot derive it correctly.

2. **Expose a conservative frontier boolean.** `DecodeBlockFrontier` will expose
   `cfl_allowed_in_sdp()`, defaulting to `true` for non-SDP or non-chroma leaves
   and carrying the derived value for SDP `CHROMA_PART` descendants. That keeps
   existing non-SDP callers unchanged.

3. **Thread mode-info context through a small input struct.** Add a
   `GeneralIntraChromaModeContext` or equivalent to `decode_general_intra_chroma_block_mode`
   so future tree-type facts can be added without widening the argument list
   repeatedly.

4. **Keep diagnostics honest.** If the live probe moves to a later unsupported
   reason, update only the expected frontier and tracking rows. Do not claim
   decoded output.

## Risks / Trade-offs

- **Risk:** Incorrect `CflAllowedInSdp` state can still desynchronize symbols.
  **Mitigation:** Add focused traversal tests for a top luma/chroma partition
  combination that sets the value to `0`, and verify the live ac0ej3 probe.

- **Risk:** The traversal state is global in the spec but implemented through
  iterative stack calls. **Mitigation:** Keep the retained state local to each
  superblock-rooted walk and propagate the derived value through child calls.

- **Risk:** The next live gate may be a broader unsupported chroma prediction or
  residual path. **Mitigation:** Preserve structured unsupported diagnostics and
  update matrix notes to identify the new frontier without output claims.
