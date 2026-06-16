# Change: writer-validate-crosstool

## Feature IDs

- `ENC-BITSTREAM-WRITER` (advances the writer surface; umbrella stays `partial`)

## Why

The writer mission's cross-tool-agreement invariant: a stream the **writer** produces must pass the
**validator** with zero error diagnostics. The round-trip harness (`writer-roundtrip-harness`) proved
`parse → write → reparse` model equality; this slice closes the loop the other way — that the writer's
re-emission of a conformant stream is itself **conformant** (`splot validate` clean). It reuses the
three committed conformant fixtures that consist only of *writable* OBU types
(`tests/fixtures/{padding,metadata-short,metadata-group}.av2` — temporal delimiter + padding / metadata
HDR_CLL, all `expect = "clean"` in `MANIFEST.toml`).

## What changes

- **Cross-tool test** (`crates/splot-validate/tests/writer_roundtrip_conformance.rs`, new). It is the
  natural home: `splot-validate` already depends on `splot-core`, so the test can call both
  `splot_core::write::*` (the writer) and `splot_validate::Validator` (the validator), obeying the
  one-way dependency rule. For each conformant writable-type fixture it:
  1. parses the fixture (`splot_core::annexb::parse_annex_b_obus`);
  2. asserts each OBU round-trips (`splot_core::write::roundtrip_obu` → `RoundTripped`);
  3. re-emits the whole Annex B stream via the writer (`write_complete_obu` + the `leb128` size
     prefix per OBU);
  4. asserts the re-emitted bytes are **byte-exact** to the original (these fixtures are canonical and
     carry no opaque non-zero blob), and that the validator reports **zero error diagnostics**
     (`ValidationReport::is_conformant`) on the re-emission — the cross-tool claim.

- **No production code change.** Test-only; no new public API, no model change, no new dependency
  (`splot-core` is already a `splot-validate` dependency).

## Validator impact

None (the validator is exercised, not changed).

## Non-goals

- **No** re-emission of frame-carrying streams (tile groups / SEF / TIP are not writable yet; the
  fixtures used here are header/metadata/padding only).
- **No** new conformant fixtures; this reuses the committed ones.
- **No** public `encode` command.

## Impact

- Crate: `crates/splot-validate` (a new integration test under `tests/`).
- Docs: `docs/IMPLEMENTATION-MATRIX.toml` (a WRITER note + proof on `ENC-BITSTREAM-WRITER`) +
  regenerated `docs/FEATURE-STATUS.md`.
