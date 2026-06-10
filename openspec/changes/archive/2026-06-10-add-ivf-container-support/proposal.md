## Why

AV2 streams in the wild are commonly wrapped in IVF, while `splot` currently
accepts only raw Annex B byte streams. The validator and inspector need to accept
both entry formats so users can point the tools at real files without pre-stripping
the container.

## What Changes

- Add `AV2-IVF-CONTAINER` as the stable Feature ID for IVF demuxing/muxing support.
- Add an `ivf` module in `splot-core` for panic-free IVF header and frame parsing,
  plus a small writer surface for future encoder/decoder use.
- Add a stream-input abstraction that auto-detects IVF vs raw Annex B and exposes
  Annex B OBU envelopes with byte offsets relative to the original input.
- Update `splot validate` and `splot inspect` so existing commands accept both raw
  Annex B and IVF files.
- Emit stable `ivf/*` diagnostics for malformed IVF input instead of panicking or
  returning a CLI-only error.
- Document the format support and the dependency decision: the external `ivf`
  crate is BSD-2-Clause and legally usable, but the project will implement the
  small required surface locally to avoid panicking APIs, AV1-specific defaults,
  and a new dependency.

## Capabilities

### New Capabilities

- `ivf-container`: IVF header/frame parsing, writing, input detection, validator
  diagnostics, and CLI behavior for AV2 Annex B payloads wrapped in IVF.

### Modified Capabilities

- `bitstream`: raw parser entry points gain an input-format detection layer while
  keeping Annex B OBU envelope parsing unchanged.
- `validator`: validation accepts IVF and reports `ivf/*` diagnostics for malformed
  containers before or alongside OBU diagnostics.
- `encoder-tools`: future writer/encoder output contracts include IVF as a supported
  container alongside raw Annex B.

## Impact

- Code: `crates/splot-core`, `crates/splot-validate`, `crates/splot-cli`.
- Docs: `README.md`, architecture/testing/roadmap/diagnostics docs, OpenSpec specs,
  and `docs/IMPLEMENTATION-MATRIX.toml`.
- Tests: parser unit/property tests, validator reports, CLI inspect/validate coverage,
  and IVF fixtures.
- Dependencies: no new dependency. The crates.io `ivf` crate remains an evaluated
  reference only and is not copied into the repository.
