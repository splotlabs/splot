# Design: IntrABC NEWMV record handoff

## Context

AV2 §5.20.5.4 sets IntrABC defaults, calls `find_mv_stack(0)`, reads
`intrabc_mode`, DRL bits, optional `intrabc_precision`, then calls
`assign_mv(0)`. In `assign_mv`, `use_intrabc` maps `intrabc_mode ? NEARMV :
NEWMV`; NEWMV consumes `read_mv()`, signs nonzero components, and forms
`BlockMvs[0] = mv_clamp_to_integer(PredMvs[0] + diffMvs[0])`.

The current code stops before this `assign_mv(0)` call. The existing inter MV
reader implements the same SHELL syntax, but only for the verified inter
EighthPel precision (`MvPrecision == 6`) and `MvCtx == 0`.

## Approach

1. Add a small `MvPrecision`/`MvContext` configuration to the inter MV reader and
   keep the old inter wrapper for existing callers.
2. Extend tile CDF row storage/selectors to cover the generated joint-shell
   class rows for P=3 and P=5, plus the `MV_CONTEXTS == 2` axis for every MV
   syntax CDF that §8.2 indexes by `MvCtx`.
3. In the IntrABC handoff, derive the bounded §7.12.2 IntrABC reference block
   vectors from the four explicit fallback candidates:
   `0,-Block_Height[SbSize]`, `-Block_Width[SbSize]-INTRABC_DELAY_PIXELS,0`,
   `0,-Block_Height[MiSize]`, and `-Block_Width[MiSize],0`, up to
   `max_bvp_drl_bits_minus_1 + 2`.
4. Apply §5.20.7.13 for single prediction:
   - NEARMV uses the selected candidate directly.
   - NEWMV reads the configured `read_mv()` delta with `MV_INTRABC_CONTEXT`,
     adds it to the selected candidate, and clamps.
5. Stop at a new structured unsupported reason before prediction/current-frame
   block copy.

## Risk Controls

- Unit tests use `SymbolEncoder` over real CDF rows so wrong symbol order
  desynchronizes.
- Tests cover quarter-pel NEWMV, one-pel NEWMV shifting, NEARMV no-delta, and
  invalid DRL index rejection.
- The live ignored `ac0ej3` probe must move from the NEWMV gate to the new
  post-block-vector prediction gate without producing output.
