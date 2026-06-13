## Why

The roadmap names a small first decode tier, but it is still prose rather than
a tracked support contract. Before runtime decode work starts, the repository
needs a precise, spec-cited tier boundary so future implementation can reject
everything else structurally instead of drifting into broad AV2 support claims.

## What Changes

- Define `DOC-MINIMAL-DECODE-TIER-CONTRACT` as a docs-only contract for the
  first intended `splot decode` success tier.
- Document a strict `minimal-intra-8bit420-hash-v1` subset: Annex B input,
  single selected layer, 8-bit 4:2:0, closed-loop key frames, fixed sequence
  dimensions, one tile, deterministic frame hashes before Y4M success.
- Keep current `splot decode` behavior unchanged: all inputs still emit the
  existing structured `decode/unsupported-feature` diagnostic until runtime
  decode support lands.
- Update decoder support and feature matrices plus generated status docs.
- Align the deterministic-frame-hash OpenSpec wording with the frame/plane
  contract: zero-based frame order is a `splot` emission index over AV2 output
  processes, not an AV2 § 7.21 syntax element.

## Capabilities

### New Capabilities

### Modified Capabilities

- `decoder-support`: add the minimal decode tier contract requirement and align
  the deterministic frame-hash ordering requirement.

## Impact

- Documentation and OpenSpec only.
- No runtime decode behavior, CLI behavior, source code, Cargo manifest,
  dependency graph, crate scaffolding, diagnostic registry, fixtures, AVM/dav2d
  integration, scripts, `xtask`, or CI changes.
