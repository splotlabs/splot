# Change: sequence and HLS validator coverage

## Summary

Extend the AV2 validator from sequence-header-general and initial temporal-unit checks to a stronger sequence/HLS validation layer. The change completes or bounds the remaining `sequence_header_obu()` child parsers, strengthens §6.4 local semantics, and adds the first HLS availability/state checks required before frame and tile validation.

## Motivation

The current matrix shows the descriptor foundation, OBU header, Annex B, trailing bits, byte alignment, payload dispatch skeleton, sequence-header-general parser, activated sequence limit checks, and initial ordering checks are implemented or partially implemented. The next blocker is that most sequence-header child rows are still todo, and HLS availability is only partial.

Frame headers and tile groups depend on sequence-level state. Implementing them before sequence/HLS state would make later validation fragile and could force rewrites.

## Scope

In scope:

- sequence-header child syntax coverage for §5.4.2 through §5.4.13;
- local §6.4 sequence-header diagnostics;
- sequence-header storage, availability, activation, repeated-identical checks, and layer-limit state;
- temporal delimiter payload/state handling;
- MSDO syntax and local §6.6 checks;
- multi-frame-header syntax fields needed for future frame references;
- inspect JSON updates and synthetic fixtures.

Out of scope:

- full frame-header parser;
- tile group payload parser;
- entropy/range decoding;
- decoder or encoder implementation;
- AVM differential harness implementation beyond planning hooks;
- full LCR/OPS/atlas support unless it stays small and separately proven.

## Impact

- `splot-core` gets more sequence/HLS parser structs.
- `splot-validate` gets stronger stateful checks and diagnostics.
- `splot-cli inspect --json` can expose parsed sequence/HLS fields.
- `docs/IMPLEMENTATION-MATRIX.toml` and generated `docs/FEATURE-STATUS.md` become more precise.

## Risks

- Some sequence child sections call shared helpers or tables not yet implemented (`tile_params`, `seg_info`, user-defined QM scan/transform tables). These must be bounded honestly instead of silently skipped.
- HLS activation rules must not be fabricated before frame/CLK parsing exists. Where activation depends on future syntax, the validator should emit a partial-coverage warning or keep the check pending.
- AV1 names, constants, or syntax assumptions must not leak into public AV2 APIs.
