## Why

The § 7.15.4 outer 2D inverse transform (`RECON-INVERSE-TRANSFORM-2D-OUTER`)
takes its `row_type` / `col_type` 1D transform selections as caller-resolved
inputs. The spec derives them with `get_transform_1d_type(dir, sz)`, which indexes
the verbatim § 7.15.4 `Transform_1d_Type[TX_TYPES][2]` constant by `PlaneTxType`
and applies the `useDdt` `DDTX`/`FDDT` substitution. Providing that derivation is
the second of the small, self-contained inverse-transform parameter derivations
the 2D-outer row deferred (after `RECON-TRANSFORM-SHIFT-LOOKUP`).

## What Changes

- Add Feature ID `RECON-GET-TRANSFORM-1D-TYPE`.
- Add `get_transform_1d_type(plane_tx_type, pass, size, use_ddt) ->
  Result<InverseTransform2dDim>` to `crates/splot-recon/src/transform_params.rs`.
  It returns `Transform_1d_Type[PlaneTxType][dir]`, then — when `use_ddt` (the
  caller-resolved `enable_inter_ddt && !use_intrabc && is_inter`) is set, the base
  type is `ADST` or `FDST`, and `size != 4` — substitutes `DDTX` or `FDDT`.
- Add the `TransformPass` enum (`Row` = spec `dir` 0, `Col` = 1).
- Transcribe the verbatim 16-row § 7.15.4 `Transform_1d_Type` constant as a
  hand-written, spec-cited `splot-recon` constant (it is a § 7.15.4 process-body
  constant absent from the generated `all_tables.h` § 9 attachment, like
  `Transform_Shift`). `IDT` maps to `InverseTransform2dDim::Identity`; the kernel
  types map to `InverseTransform2dDim::Kernel`.
- Add the typed `ReconError::InvalidPlaneTxType` for a `PlaneTxType` outside
  `0..TX_TYPES`.
- Update the implementation matrix, decoder-support matrix, conformance-coverage
  group, roadmap, generated status docs, and OpenSpec artifacts.

Non-goals:

- No DPCM-direction selection, no combined transform-parameter resolve helper, no
  wiring of `get_transform_1d_type` into the runtime decode path, no § 7.15.3
  secondary transform, and no coefficient entropy decode.
- No new fixture and no output change — the derivation is a leaf observed by no
  decode path, so the minimal flat-intra snapshots stay byte-identical.
- No tile-syntax decode, runtime decode output, hashes, Y4M, or reference
  refresh; no scheduler state in `splot-recon`.
- No AVM/dav2d source, dependency, wrapper, script, CI job, or required test.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: records the source-backed AV2 § 7.15.4
  `get_transform_1d_type` row/column transform-type derivation, while broader
  reconstruction (the DPCM-direction selection, the runtime transform wiring, and
  the coefficient entropy decode) remains partial.

## Impact

- `crates/splot-recon/src/transform_params.rs`
- `crates/splot-recon/src/error.rs`
- `crates/splot-recon/src/lib.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`
- `xtask/src/decoder_conformance_coverage.rs`
- `docs/DECODER-ROADMAP.md`
- generated status/coverage docs
- `openspec/specs/decoder-support/spec.md`
