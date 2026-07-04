## Context

The previous local decoder mission slice advanced the live Wiener NS LR path into selectable
transform-record derivation. The local stream now stops at AV2 §5.20.5.6 because
one block selects active `UV_CFL_PRED`; the mode parser currently treats a true
`is_cfl` symbol as unsupported before consuming §5.20.7.32
`read_cfl_alphas()`.

The generated default CfL CDF tables already exist in `splot-core`, but the
tile CDF subset only exposes `TileIsCflCdf` and
`TileUVModeCflNotAllowedCdf`. The runtime also stores chroma mode facts as the
decoded no-CfL `uv_mode` index, which is insufficient for coefficient parsing
when the typed mode is `UV_CFL_PRED`.

## Goals / Non-Goals

**Goals:**
- Consume active `is_cfl` mode-info and `read_cfl_alphas()` in AV2 spec order
  for the non-lossless 4:2:0 local decoder mission path.
- Expose CfL index/sign/alpha/MHCCP CDF rows through the tile CDF subset
  lifecycle using already generated §9.3 defaults.
- Carry the typed `UV_CFL_PRED` mode value into selectable-transform residual
  parsing so the arithmetic decoder stays synchronized.
- Replace the live local decoder mission diagnostic with the next honest unsupported frontier.

**Non-Goals:**
- Implement CfL prediction, `CflRef` capture, alpha application, chroma sample
  reconstruction, loop-restoration filtering, 10-bit output, or reference
  refresh.
- Claim successful local decoder mission decode or AVM/dav2d byte equality.
- Add new dependencies or change crate dependency direction.

## Decisions

1. Model CfL as mode syntax, not prediction support.

   The parser will return a mode fact that can represent `UV_CFL_PRED` and the
   parsed alpha syntax. Runtime prediction remains gated later with a structured
   diagnostic. This follows the current verified-subset discipline: consume
   syntax only when the syntax is grounded, and stop before unverified sample
   semantics.

2. Reuse generated §9.3 CDF defaults.

   The CfL CDF rows in `splot-core::tables::cdf` are generated from the committed
   AV2 table attachment. The implementation will wire those rows into
   `splot-decode` CDF selection, copy/average, and frame-end scaling instead of
   hand-defining constants.

3. Keep MHCCP fail-closed.

   `read_cfl_alphas()` includes `cfl_mhccp` and `cfl_mh_dir` branches. The slice
   may consume the necessary symbol when the stream reaches it, but active MHCCP
   prediction remains unsupported and must not be reported as decoded CfL
   prediction.

## Risks / Trade-offs

- Wrong alpha-context selection could desynchronize the stream -> mitigate with
  focused unit tests for `cfl_alpha_signs` contexts and the live local decoder mission probe.
- `UV_CFL_PRED` may expose the next coefficient or prediction unsupported branch
  quickly -> treat that as success for this brick if the former CfL-mode
  diagnostic is gone and the new diagnostic is structured.
- The existing selectable-transform spec overstates the local stream path after
  the new live blocker was found -> modify that spec to name the CfL prerequisite
  explicitly.
