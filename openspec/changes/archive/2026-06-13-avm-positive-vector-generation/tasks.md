# Tasks: avm-positive-vector-generation

## 1. Vectors

- [x] 1.1 Generate diverse AVM streams from project-owned synthetic YUV input
  (`--lag-in-frames=0`, IVF frame-count consistent): 128x128 intra, 96x64
  key+inter, 64x64 10-bit intra, 64x64 OPS, plus `--enable-lcr` and
  `--enable-qm` streams.
- [x] 1.2 Verify each: the four standalone vectors validate clean; the LCR/QM
  streams emit exactly their availability diagnostic; all are IVF-consistent.

## 2. Manifest + docs + matrix

- [x] 2.1 Add the six vectors to `tests/conformance/manifest.toml` (four
  `clean`, two `{ diagnostics = [...] }`).
- [x] 2.2 `CONF-AVM-VALID-STREAMS`: expand proof.fixtures; note the diverse
  feature coverage and the LCR/QM external-HLS-dependency differential finding;
  set `openspec_change`. Update `docs/CONFORMANCE.md` if warranted.

## 3. Verification

- [x] 3.1 The committed runner validates all new vectors against the manifest
  (no orphans). `cargo xtask ci` (bare, exit checked) passes.
