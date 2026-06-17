## Why

The § 7.15.4 outer 2D inverse transform (`RECON-INVERSE-TRANSFORM-2D-OUTER`)
takes its `rowShift` / `colShift` down-shifts as caller-resolved inputs. The spec
derives them from the verbatim § 7.15.4 `Transform_Shift[TX_SIZES_ALL][2]`
constant. Providing that lookup is the first of the small, self-contained
inverse-transform parameter derivations that the 2D-outer row deferred, and it
makes the reconstruction side ready to consume real coefficients without changing
any decode output.

## What Changes

- Add Feature ID `RECON-TRANSFORM-SHIFT-LOOKUP`.
- Add `transform_shift(log2_width, log2_height) -> Result<(rowShift, colShift)>`
  to a new `crates/splot-recon/src/transform_params.rs` module. It returns
  `(Transform_Shift[txSz][0], Transform_Shift[txSz][1])` for the `txSz` whose
  `(Tx_Width_Log2, Tx_Height_Log2)` equals the requested shape.
- Transcribe the verbatim 25-row § 7.15.4 `Transform_Shift` constant as a
  hand-written, spec-cited `splot-recon` constant. It is a § 7.15.4 process-body
  constant absent from the generated `all_tables.h` § 9 attachment, so it is
  deliberately not a `cargo xtask gen-tables` output. The parallel
  `(log2W, log2H)` key table mirrors the § 9.2 `Tx_Width_Log2` / `Tx_Height_Log2`
  values, which `splot-recon` cannot reach through `splot-core` under the one-way
  dependency rule — the same reason the § 7.15 transforms take caller-resolved
  log2 dimensions.
- Add the typed `ReconError::InvalidTransformShiftShape` for a `(log2W, log2H)`
  pair that is not one of the 25 AV2 `TX_SIZES_ALL` shapes.
- Update the implementation matrix, decoder-support matrix, conformance-coverage
  group, roadmap, generated status docs, and OpenSpec artifacts.

Non-goals:

- No `get_transform_1d_type` row/column transform-type derivation, no
  DPCM-direction selection, no `Transform_Shift` wiring into the runtime decode
  path, no § 7.15.3 secondary transform, and no coefficient entropy decode.
- No new fixture and no output change — the lookup is a leaf function observed by
  no decode path, so the minimal flat-intra snapshots stay byte-identical.
- No tile-syntax decode, runtime decode output, hashes, Y4M, or reference
  refresh; no scheduler state in `splot-recon`.
- No AVM/dav2d source, dependency, wrapper, script, CI job, or required test.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: records the source-backed AV2 § 7.15.4 `Transform_Shift`
  row/column down-shift lookup, while broader reconstruction (the
  `get_transform_1d_type` derivation, the coefficient entropy decode, and the
  runtime transform wiring) remains partial.

## Impact

- `crates/splot-recon/src/transform_params.rs` (new)
- `crates/splot-recon/src/error.rs`
- `crates/splot-recon/src/lib.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`
- `xtask/src/decoder_conformance_coverage.rs`
- `docs/DECODER-ROADMAP.md`
- generated status/coverage docs
- `openspec/specs/decoder-support/spec.md`
