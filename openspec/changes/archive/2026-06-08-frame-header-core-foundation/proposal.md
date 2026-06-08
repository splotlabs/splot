# Change proposal: frame-header core foundation

## Summary

Extend `splot`'s frame-header validation from an activation-only prefix parser to a state-aware frame-header core foundation.

The current validator parses the activation/reference prefix of `frame_header_info()` only. This change adds typed parser modes, explicit frame parse context, core frame-header fields, and local validator diagnostics for state-supported checks.

## Motivation

Most standalone OBU payload foundations are implemented. The remaining validator gaps now depend on frame-header state:

- frame-side QM and film-grain references;
- tile-group header completion;
- frame size and reference-map checks;
- random-access and long-term-reference availability;
- later AVM parser trace comparison.

A full §5.18 implementation is too large for one change. This change introduces the next safe slice.

## Scope

### In scope

- Preserve existing activation-prefix parsing.
- Add a frame-header core parse mode.
- Add explicit parser input/state types.
- Parse state-supported §5.18.2 control fields and selected §5.18.3/§5.18.4 helpers.
- Add parse-status reporting.
- Add local validator diagnostics for exact, supported checks.
- Add inspector JSON summaries.
- Update implementation matrix and generated feature status.

### Out of scope

- Full §5.18 frame header.
- `frame_header_copy()` bit identity.
- Full frame-level filtering, quantization, segmentation/tiling, transform/coding modes, global motion, frame film-grain structures.
- Tile group payload.
- Entropy decoding.
- Pixel reconstruction.
- Encoder/writer changes.
- AVM differential harness as a required CI gate.

## Success criteria

- Existing activation/HLS tests continue to pass.
- New core parser tests cover direct sequence and MFH reference paths.
- Truncated core frame-header payloads produce typed errors or diagnostics, never panics.
- New diagnostics have stable IDs and spec sections.
- Matrix remains honest: umbrella frame rows stay partial.
- `cargo xtask ci` passes.
