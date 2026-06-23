## Context

AV2 § 5.20.7.27 computes `maxLevel` in the second ordinary non-FSC coefficient
pass after sign syntax selection and before § 5.20.7.28 `read_quant`. The value
depends on:

- `get_lf_limits(row, col, txClass, plane)`;
- the luma/chroma plane branch;
- the hidden-parity final scan entry override.

The existing quant-pass composer intentionally still receives this value from a
caller. This change keeps the same loaded-but-unwired boundary style while
making the next caller fact explicit and testable.

## Decisions

1. **Keep a decode-local transform-class enum.**

   The entropy-side helper must not import `splot-recon` transform types across
   the runtime-handoff dependency boundary. A small local enum names the three
   spec classes needed by `get_lf_limits`.

2. **Return quant-pass inputs through an explicit conversion.**

   The derivation records retain `is_low_frequency` for testing and later caller
   diagnostics, while `quant_pass_input()` / `max_levels_to_quant_pass_inputs`
   provide the shape consumed by `apply_nonzero_coeff_quant_pass`.

3. **Stay loaded-but-unwired.**

   Runtime transform class, hidden parity, scan order, and selector derivation
   remain later work. This change is a deterministic helper over already checked
   scan entries and caller-resolved block facts.
