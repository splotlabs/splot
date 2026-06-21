## Context

> **Historical note.** This change predates `decode-minimal-fixture-avm-skip-polarity`, which retired the frozen `base_q_idx == 255` committed minimal fixture (`syn-flat-intra-64x64-minimal.ivf`) and replaced it with an AVM/dav2d-conformant `base_q_idx` 210 luma-skip stream that routes through the general intra path. References below to the committed minimal fixture as the frozen `base_q_idx == 255` anchor — and to keeping that committed fixture's hash byte-identical — are historical; the routing rule (a `base_q_idx == 255` frame falls through to the frozen gate) still holds.

The general intra frame path decodes mode info and then returns
`general_intra_residual_decode_unimplemented`. The coefficient-loop machinery
(`apply_coeff_use_fsc_branch_from_frame_facts` and the ordinary nonzero pass)
already exists but has only ever been reached by the frozen synthetic trace's
all-zero shortcut. AV2 § 5.20.7.27 `coeffs()` reads `all_zero` then, when
`all_zero == 0`, decodes the end-of-block and coefficient symbols.

## Goals / Non-Goals

**Goals:**
- Decode the single non-partitioned 64x64 luma transform block's § 5.20.7.27
  `coeffs()` syntax on the general intra path.
- Read `all_zero` with the spec-derived § 8.3.2 `txb_skip` context.
- Route the nonzero pass through the existing coefficient-loop entry and return
  the decoded `Quant[]` and end-of-block.
- Keep the frozen `base_q_idx == 255` minimal hash contract byte-identical.

**Non-Goals:**
- No chroma coefficient decode, dequantization, inverse transform, residual
  add, reconstruction, or output (the decoded `Quant[]` is returned, not
  reconstructed; its value is verified by the later reconstruction brick).
- No tile context-line commit, split transform partitions, or skipped-block
  side effects beyond returning the `all_zero` decision.
- No in-repo AVM/dav2d dependency.

## Decisions

1. Read `all_zero` in the caller, reusing the existing nonzero coefficient pass.

   Rationale: AV2 § 5.20.7.27 reads `all_zero` before branching, and
   `apply_coeff_use_fsc_branch_from_frame_facts` takes the already-decoded
   `AllZero` / `NonZero` choice. The caller derives the `txb_skip` selector
   (`coeff_cdf_q_ctx` from `base_q_idx`, `txSzCtx`, luma context) and reads the
   symbol, then delegates the nonzero pass to the existing machinery, so the EOB,
   scan, coefficient base/range/sign, and `read_quant` logic is not duplicated.

2. Derive `txSzCtx` from the generated § 9.2 tables.

   Rationale: `txSzCtx = (Tx_Size_Sqr[txSz] + Tx_Size_Sqr_Up[txSz] + 1) >> 1`
   matches the formula the ordinary branch uses internally; using the generated
   `splot-core` tables keeps the two consistent. For the single 64x64 block it
   resolves to 4.

3. Return the decoded `Quant[]` rather than reconstructing.

   Rationale: this brick is the entropy decode; dequantization, inverse
   transform, residual add, and prediction are the reconstruction brick. The
   `Quant[]` value is verified end-to-end there (luma plane equals avmdec).

## Risks / Trade-offs

- [Risk] The coefficient entropy decode is only fully verifiable end-to-end
  (reconstructed luma equals the avmdec oracle), so a subtly wrong context could
  pass this brick's checks but fail at reconstruction.
  -> Mitigation: the `all_zero` context inputs are transcribed from § 5.20.7.27
  and § 8.3.2; the CLI test proves the q80 fixture's luma coefficients decode
  through the real coefficient-loop machinery without error and reach the chroma
  step; a unit test pins the `txSzCtx` derivation; and the reconstruction brick
  provides the strong oracle check.
- [Risk] The decoded `Quant[]` is returned but not yet consumed.
  -> Mitigation: it advances the bitstream past the luma block and is the input
  the reconstruction brick will consume; it is exercised by the CLI test.
