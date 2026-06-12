# Proposal: complete the intra frame-header tail

## Feature IDs

- `AV2-5.18.8-TRANSFORM-CODING-MODES` (read_tx_mode § 5.18.8.1,
  skip_mode_params § 5.18.8.2, frame_reference_mode § 5.18.8.3)
- `AV2-5.18.10-FILM-GRAIN-STRUCTURES` (film_grain_config § 5.18.10.1; the
  in-band film_grain_model § 5.18.10.2 lives only in the § 5.14
  `AV2-5.14-FILM-GRAIN` OBU context, since `load_grain_model()` reads no bits)
- `AV2-5.18.9-GLOBAL-MOTION` (the loop-trivial intra arm only)
- `AV2-5.18.2-FRAME-HEADER-INFO` (the intra path completes)

## Why

After `ccso_params()` the § 5.18.2 tail reads `read_tx_mode()` (mirror
`05-syntax-structures.md`:7634, gated on the already-derived CodedLossless),
`frame_reference_mode()` (:7739 — intra: `reference_select = 0`, no bits),
`skip_mode_params()` (:7671), `allow_bawp`/`allow_warpmv_mode` (intra:
inferred 0, no bits), `reduced_tx_set f(2)`, `global_motion_params()`
(:7776 — the intra arm is loop-trivial), and `film_grain_config()` (:8163,
which calls `film_grain_model()` — reuse the § 5.14 FilmGrainModel parser
in `crates/splot-core/src/headers/film_grain.rs`). Landing this consumes a
FULL intra frame header, enabling trailing-bits validation of
frame-carrying OBUs, the complete SEF status (the SEF path stops before
film_grain_config today), and exact NumFrameHeaderBits accounting (the
next backlog change).

## What Changes

1. Parse the remaining intra-tail structures exactly per the mirror,
   transcribing every intra-path gate (no-bit inferences included as
   derivations, not reads).
2. The SEF path advances to its complete status (it stops before
   film_grain_config today — finish it per its grammar).
3. The intra-path terminal status becomes a completed-header status;
   trailing-bits validation of fully-parsed frame-carrying OBUs becomes
   decidable where § 5.2.3/§ 5.18.1 prescribe it (add the locally-decidable
   check if unambiguous; otherwise name the residual for the
   NumFrameHeaderBits change).
4. EOF anywhere in the new tail preserves parsed facts (the established
   truncation pattern); constructed-view arithmetic audited.
5. `inspect` surfaces the new structures; synced OpenSpec main-spec
   stop-point requirements updated in the same change.

## Non-goals

- Inter-path arms of any tail structure (named residuals).
- The § 5.18.9 subexp inter arm (its own backlog change).
- NumFrameHeaderBits copy-bit accounting (next change).

## Acceptance criteria

- [ ] A full intra frame header parses end to end (incl. SEF complete);
  positive/negative/EOF tests per structure, gates both ways; film-grain
  model reuse tested against the § 5.14 parser's fixtures.
- [ ] Stop-status progression tested; matrix proof recorded;
  `cargo xtask ci` green.
