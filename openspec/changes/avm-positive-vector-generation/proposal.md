# Change: avm-positive-vector-generation

## Feature IDs

- `CONF-AVM-VALID-STREAMS`

## Why

The committed conformance corpus (the archived `conformance-corpus-foundation`
change) bootstrapped with two small valid vectors (64x64 intra and key+inter).
This change broadens the **positive** corpus with diverse AVM-generated streams
so the validator's zero-false-positive guarantee is exercised across more codec
feature combinations — a real differential check: AVM (the reference oracle)
produces the streams, and `splot validate` must accept the self-contained ones
clean (or flag a genuine, spec-grounded availability gap on the ones AVM
produces for external-HLS provision).

All vectors are generated locally from **project-owned synthetic YUV input**
(no third-party content) per the maintainer's AVM-as-local-oracle decision; AVM
is never vendored or a build/CI dependency.

## What Changes

Six new committed vectors (all IVF, from synthetic input, `--lag-in-frames=0`
so the IVF frame counts are internally consistent), added to `manifest.toml`:

- **Four standalone-clean positives** (`expect = "clean"`): a larger-resolution
  intra key (128x128), a non-square key+inter (96x64), a 10-bit intra key, and
  an operating-point-set stream (`OBU_OPERATING_POINT_SET`).
- **Two external-HLS-dependent streams** (`expect = { diagnostics = [...] }`):
  AVM's `--enable-lcr` emits a local LCR referencing an absent global LCR, and
  `--enable-qm` emits `using_qmatrix` referencing a QM level with no QM OBU.
  Validated standalone (external HLS disabled), `splot` correctly flags exactly
  `lcr/global-lcr-unavailable` (§ 7.3.8.3) / `frame-header/qm-level-unavailable`
  (§ 7.3.8.9) — confirming those availability checks fire on real AVM output.

The existing committed runner (`crates/splot-cli/tests/conformance.rs`) covers
all new vectors automatically (it iterates the manifest and rejects orphans);
no runner code changes.

## Non-goals

- No AVM vendoring; no AVM in build/CI (the runner validates committed bytes).
- No new validator diagnostics; the two diagnostics entries exercise existing,
  registered availability checks.

## Acceptance criteria

- [ ] Six diverse AVM-generated vectors are committed and in the manifest; the
  four clean ones validate clean, the two external-HLS-dependent ones emit
  exactly their documented availability diagnostic. `CONF-AVM-VALID-STREAMS`
  proof updated. `cargo xtask ci` green.
