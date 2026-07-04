## Why

The decoder reconstruction frontier needs the inverse transform after
dequantization. AV2 § 7.15.2.1 is the kernel-based 1D inverse transform that the
§ 7.15.4 2D transform invokes for every row and column. It is the first consumer
of the shared `splot-tables` § 9.6 kernels (the reason that crate was created),
it is small and pure, and it is the natural next residual-path brick.

## What Changes

- Add Feature ID `RECON-INVERSE-TRANSFORM-1D`.
- Add the `splot-recon -> splot-tables` dependency edge (the first consumer of
  the shared transform-kernel crate).
- Add `crates/splot-recon/src/inverse_transform.rs` with:
  - `InverseTransform1dType` modeling the § 7.15.4.1 Table 7.1 kernel types
    `Dct`/`Adst`/`Fdst`/`Ddtx`/`Fddt` (`IDT` is the separate § 7.15.2.3
    identity transform and is excluded).
  - `inverse_transform_1d`, implementing § 7.15.2.1 exactly: matrix-multiply the
    coefficients by the size-and-type § 9.6 kernel, then § 4.8 `Round2` and the
    § 7.15.2.1 `colTx`-dependent `Clip3`.
- Reproduce the spec dispatch faithfully (length-4 `else` to FDST, length-32 DCT
  for every type, `Fddt` reversing the DDTX kernel column), with `i64`
  accumulation and typed `ReconError` for invalid lengths.
- Preserve the current runtime `splot decode` unsupported behavior.
- Add self-contained spec-exact unit tests.
- Update decoder support, feature tracking, roadmap, generated status docs, and
  OpenSpec artifacts.

Non-goals:

- No § 7.15.2.2 Walsh-Hadamard transform, § 7.15.2.3 identity transform,
  § 7.15.3 secondary transform, or § 7.15.4 2D inverse transform orchestration
  (`Transform_Shift`, `get_transform_1d_type`, row/column passes, DPCM, sample
  duplication).
- No dequantization, residual addition, tile-syntax decode, runtime decode
  output, hashes, Y4M, or reference refresh.
- No scheduler state in `splot-recon`.
- No AVM/dav2d source, dependency, wrapper, script, CI job, or required test.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: records source-backed AV2 § 7.15.2.1 1D inverse transform
  support while broader reconstruction (the § 7.15.4 2D transform, the other 1D
  transforms, dequantization, and residual addition) remains partial.

## Impact

- `crates/splot-recon/src/inverse_transform.rs`
- `crates/splot-recon/src/error.rs`
- `crates/splot-recon/src/lib.rs`
- `crates/splot-recon/Cargo.toml`
- `xtask/src/main.rs` (dependency-direction rule)
- `AGENTS.md`, `docs/ARCHITECTURE.md`
- `Cargo.lock`
- `docs/IMPLEMENTATION-MATRIX.toml`
- `docs/DECODER-SUPPORT-MATRIX.toml`
- generated status/coverage docs
- `docs/DECODER-SUPPORT-MATRIX.toml`
- `openspec/specs/decoder-support/spec.md`
