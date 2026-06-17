## Context

`splot-recon` needs the AV2 § 9.6/§ 9.7 transform-kernel tables for the § 7.15
inverse transform, but the one-way dependency rule forbids `splot-recon` from
depending on `splot-core`, where those generated tables currently live. The
maintainer chose a shared dependency-free crate (over extending the generator to
emit directly into `splot-recon`, or hand-transcribing the kernels).

## Goals / Non-Goals

Goals:

- A dependency-free `splot-tables` crate any crate can use.
- Keep the tables generator the single source of truth (no hand-edited tables,
  drift-checked in CI).
- Zero change to generated table content.

Non-Goals:

- The § 7.15 inverse transform itself and the `splot-recon -> splot-tables`
  edge (next row).
- Moving in-`splot-core` tables that have consumers there.

## Decisions

- **Move only the transform-kernel modules.** `transform_1d` and
  `secondary_transform` have no `splot-core` consumer (verified: zero references
  outside `src/tables/`, save one relocatable spot test), so moving them is
  zero-churn for `splot-core`. `cdf`/`conversion`/`quantizer`/`warp_filter`/
  `loop_restoration` stay because they are consumed by `splot-core` and
  `splot-decode`.
- **Per-module output dir in the generator, not move-everything.** `gen-tables`
  gains `output_dir_for(module)`, routing the two transform modules to
  `crates/splot-tables/src/tables/` and the rest to
  `crates/splot-core/src/tables/`. The write path, stray-file removal, and
  `--check` drift scan all iterate every output directory; one `mod.rs` is
  emitted per directory. This keeps each table in exactly one place (single
  source of truth) while avoiding the wide consumer churn of moving the
  `cdf`/`conversion` tables out of `splot-core`.
- **No dependency-direction change is required this brick.** `splot-tables`
  depends on nothing, and no crate depends on it yet (the consumer edge lands
  with the § 7.15 row). It is recorded in `INTERNAL_DEP_RULES` as a
  dependency-free leaf so the rule set is explicit.
- **Crate layout mirrors `splot-core`.** Generated modules live under
  `crates/splot-tables/src/tables/` with a generated `mod.rs`; the hand-written
  `lib.rs` exposes `pub mod tables;`, so consumers use
  `splot_tables::tables::transform_1d::…`.

## Risks / Trade-offs

- The generator is CI-gating (drift + determinism). The change is verified by
  the unchanged 236-table determinism count, byte-identical regeneration, and a
  green `gen-tables --check` across both directories.
- A crate with no workspace dependents this brick is intentional staging; it is
  exercised by its own mirror cross-check spot test and consumed by the next row.

## Migration Plan

Additive plus a pure relocation of two generated modules. No table content
changes, no public API changes in `splot-core` (the moved tables had no
`splot-core` consumers), and the runtime is unaffected.

## Open Questions

None.
