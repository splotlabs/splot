## Context

`RECON-INVERSE-TRANSFORM-2D-OUTER` applies the § 7.15.4 DPCM cumulative sum via
its `DpcmDirection` enum and `apply_dpcm`, but the *selection* of that direction
(or `None`) is caller-resolved. `RECON-TRANSFORM-SHIFT-LOOKUP`,
`RECON-GET-TRANSFORM-1D-TYPE`, and `RECON-RESOLVE-2D-TRANSFORM-PARAMS` cover the
other § 7.15.4 parameter derivations; this change adds the last one.

## Goals / Non-Goals

**Goals:**

- Provide the § 7.15.4 DPCM-direction selection as a total `const fn` producing
  the `Option<DpcmDirection>` the outer transform consumes.
- Keep `splot-recon` free of frame state and the prediction-mode enum (caller
  resolves the plane-selected `useDpcm` flag and whether the mode is `V_PRED`).

**Non-Goals:**

- Resolving `use_dpcm_y` / `use_dpcm_uv` / `YMode` / `UVMode` from syntax, or any
  wiring into the runtime decode path.

## Decisions

- **Home in `transform_params.rs`.** That module documents itself as the
  "§ 7.15.4 inverse-transform parameter derivations" home and already listed the
  DPCM-direction selection as a future row alongside `transform_shift` and
  `get_transform_1d_type`. A free `const fn` matches those siblings.
- **Caller-resolved `(use_dpcm, mode_is_v_pred)` scalars.** § 7.15.4 selects
  `useDpcm` and `mode` per plane from frame/block state `splot-recon` does not
  hold; passing the two resolved booleans keeps the crate free of the
  prediction-mode enum and the per-plane selection, matching how the sibling
  derivations take caller-resolved log2 dimensions and `PlaneTxType`.
- **Return `Option<DpcmDirection>`, not a bool plus a direction.** `None` encodes
  "no DPCM" exactly as the `dpcm` field of `InverseTransform2dOuter` expects, so
  the result drops straight in.
- **`const fn`, no error path.** Every `(use_dpcm, mode_is_v_pred)` maps to a
  defined result; three cases are pinned at compile time as `const` spec
  contracts.

## Risks / Trade-offs

- The mapping is small (three branches), so it risks reading as trivial. It earns
  its place by completing the § 7.15.4 parameter-derivation surface with one
  spec-cited, tested entry point, and the integration test proves the selected
  direction actually drives the outer transform's cumulative sum (not just the
  enum mapping) — catching a wiring mistake between the selection and the apply
  step.
- It is loaded ahead of its runtime caller, matching the established pattern of
  building the § 7.15.4 parameter derivations before the runtime wiring; the
  matrix and roadmap keep the runtime resolution and decode wiring out of scope.
