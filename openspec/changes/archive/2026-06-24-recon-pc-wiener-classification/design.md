## Context

The current ac0ej3 LR path stops after resolving §7.20.4 classified-Wiener
source-read and `LrTxSkip` lookup coordinates. `splot-recon` already has
§7.20.2 source-sample selection/value helpers and §7.20.3 Wiener NS luma/chroma
filter primitives, but it has no pixel-classified Wiener classification helper.

The normative `Pc_Wiener_Lut_To_Class` table is generated today under
`splot-core::tables::loop_restoration`; `splot-recon` cannot depend on
`splot-core`. The dependency-free `splot-tables` crate already exists for shared
generated §9 tables, so this change can expose the generated §9.8
loop-restoration table module there without introducing a new dependency edge.

## Goals / Non-Goals

**Goals:**

- Add a scheduler-free, panic-free `splot-recon` primitive for AV2 §7.20.4
  skip-filter pixel classification.
- Use caller-provided source samples and `LrTxSkip` values, because runtime frame
  storage and transform-block state are still outside this brick.
- Use generated normative §9.8 tables from `splot-tables`, not hand-transcribed
  PC-Wiener class tables.
- Keep the existing `splot-core` generated loop-restoration table module intact
  for current parser and writer consumers.

**Non-Goals:**

- Runtime decode wiring, `FilterClass` grid storage, `SubclassLookup` wiring,
  §7.20.3 filter invocation, frame/current-CDEF storage reads, `LrTxSkip` grid
  derivation, 10-bit output, reference refresh, or successful ac0ej3 decode.
- Moving `splot-core` consumers to `splot-tables` in this brick.
- Introducing new dependencies or changing crate dependency direction.

## Decisions

- **Emit §9.8 into both table crates.** `cargo xtask gen-tables` will generate
  `loop_restoration.rs` into both `crates/splot-core/src/tables/` and
  `crates/splot-tables/src/tables/`. This preserves existing `splot-core`
  consumers while allowing `splot-recon` to read normative tables through its
  existing `splot-tables` dependency. The generated attachment remains the
  single source of truth; the duplicate Rust module is generated, drift-checked,
  and not hand-edited.
- **Keep classifier inputs caller-resolved.** The helper receives
  `source_sample(x, y)` and `tx_skip(x, y)` callbacks over the §7.20.4 feature
  window. The caller owns frame-coordinate offsets, §7.20.2 source selection,
  `BlockStartX`/`BlockEndX` clipping, stripe/tile clipping, and `LrTxSkip`
  storage selection.
- **Return intermediate facts.** The result includes accumulated normalized
  features, accumulated `LrTxSkip`, `lut_input`, and class. Tests can therefore
  prove the value math before the final LUT lookup, and runtime diagnostics can
  later expose precise failure context if needed.
- **Fail before publishing values.** The helper validates sample type, bit-depth
  range, and `LrTxSkip` range while computing into local variables only. Since it
  does not mutate caller output, failures are naturally transactional.

## Risks / Trade-offs

- Generated table duplication increases repository size. Mitigation: only
  generated §9.8 tables are duplicated, `gen-tables --check` owns both copies,
  and no hand-maintained constants are introduced.
- The primitive cannot by itself move the ac0ej3 runtime diagnostic because live
  source sample and `LrTxSkip` values are unavailable at the current runtime
  order. Mitigation: the support row and runtime diagnostic stay honest, and the
  primitive removes a real blocker for the later runtime wiring step.
- Caller callbacks can encode wrong clipping or source selection. Mitigation:
  the API docs keep those caller obligations explicit, and runtime integration
  remains separate until fixture/oracle evidence exists.
