## Why

Decoder work cannot safely move from the current unsupported CLI entry point to
byte-consuming planning or pixel allocation until the repository defines the
resource budget contract. The existing `decode-limits-budget` row already marks
this as foundation work; this change turns that placeholder into a documented,
spec-grounded contract without adding decoder crates or runtime APIs.

## What Changes

- Define the `DOC-DECODE-LIMITS-CONTRACT` documentation feature for future
  `DecodeOptions { limits: DecodeLimits }` behavior.
- Document the required limit categories before any bitstream-derived allocation:
  input size, OBU count, decoded frame count, output frame count, frame
  dimensions, luma samples per frame, decoded frame bytes, reference frames,
  tile count, tile payload bytes, and output bytes.
- Document the planned `decode/resource-limit` diagnostic shape and stable fields
  without emitting it yet.
- Update decoder support docs and status matrices to mark `decode-limits-budget`
  as a contract-only partial row.
- Keep `splot decode` behavior unchanged: it still emits only
  `decode/unsupported-feature`, reads no input bytes, writes no output, and does
  not invoke AVM, dav2d, ffmpeg, or other external decoders.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: Adds requirements for the future decode limits/resource
  budget contract and planned resource-limit diagnostics.

## Impact

- Affected docs: `docs/DECODER-ROADMAP.md`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated decoder support status,
  feature tracking docs, and the implementation matrix.
- Affected OpenSpec capability: `decoder-support`.
- Affected automation: existing decoder support and feature status drift checks
  prove the docs/matrix updates; no new command is introduced.
- Dependencies: no new crates, no dependency graph changes, no external tool
  integration, and no AVM/dav2d runtime or CI path.
