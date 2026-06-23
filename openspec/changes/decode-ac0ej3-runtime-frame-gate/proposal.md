## Why

The current `splot decode /Users/bartosztomczyk/Documents/SplotLabs/ac0ej3.ivf`
gate is still a coarse minimal-runtime shape check: the byte planner traverses
the mission-scale stream, but `ensure_multiframe_plan_shape` rejects any plan
with more than three frame candidates before the decoder can reach the first
actual unsupported AV2 feature.

The three-frame cap was useful while the only proven multi-reference fixture was
`syn-3frame-multiref-64x64.ivf`, but the runtime now has more precise
fail-closed gates while iterating inter frames: TIP / non-tile-group candidates,
more than two valid references, `NumTotalRefs > 2`, adapted-CDF loads, temporal
MVs, unsupported tools, and unsupported frame geometry. Rejecting solely because
the full plan is long hides those diagnostics and slows the ac0ej3 mission.

## What Changes

- Remove the up-front `MAX_MULTIFRAME_CANDIDATES == 3` runtime preflight.
- Keep the strict leading key-frame shape and container checks, while allowing
  only the parser's terminal trailing partial IVF header warning.
- Keep the existing verified-subset inter-frame gates; a fourth frame or broader
  stream still rejects before output when it first needs unproven reference or
  tool behavior.
- Add regression coverage proving a four-frame stream reaches
  `inter_too_many_valid_references` instead of the old
  `unsupported_frame_candidate_count` gate.
- Pin the current `ac0ej3.ivf` diagnostic so future bricks can show concrete
  progress through the gate stack.

## Impact

- Touches the minimal runtime shape gate, tests, and decoder support tracking.
- Does not add new AV2 syntax, prediction, filtering, residual, reference,
  output, or conformance support.
- Does not make partial output visible: the runtime still collects decoded frames
  internally and returns a structured error before hash/raw/Y4M publication on
  unsupported streams.
