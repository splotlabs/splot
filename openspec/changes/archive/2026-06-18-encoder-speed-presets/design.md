## Context

`splot encode` already exposes `--speed`, but the value is ignored before the
encoder runtime config is built. `EncoderConfig` is intentionally limited to
bitstream-affecting settings, while `EncoderRuntimeConfig` currently carries only
thread policy. `ENC-SPEED-PRESETS` is tracked in the implementation matrix but
still has no typed API, range check, CLI handoff, or proof.

No public packet-producing path exists yet. This change therefore defines the
runtime-policy boundary for speed presets only; later mode-decision, scheduling,
and rate-control work can consume it without changing the preset contract.

## Goals / Non-Goals

**Goals:**

- Add a typed `SpeedPreset` with a documented accepted numeric range.
- Store the preset in `EncoderRuntimeConfig`, defaulting to a conservative value.
- Parse `splot encode --speed` through the typed API and reject unsupported
  values before constructing the context.
- Expose the preset through `Context::runtime()` and a focused accessor.
- Prove that speed policy remains separate from bitstream configuration and that
  current encode lifecycle behavior stays non-emitting.
- Update `ENC-SPEED-PRESETS` matrix/status evidence.

**Non-Goals:**

- No AV2 syntax emission, writer integration, packet output, rate control, mode
  decision, RDO, threading scheduler changes, or speed/quality performance
  claims.
- No dependency graph changes.
- No claim that Baseline Encoder Profile v1 is implemented.

## Decisions

### Use a newtype with a closed current range

`SpeedPreset` will be a public newtype around `u8` with constants for the current
inclusive range and a `try_from_u8` constructor. The current range is a project
policy, not an AV2 syntax claim. A newtype avoids treating arbitrary integers as
accepted runtime policy and gives the CLI a single validation surface.

Alternative considered: keep `Option<u8>` in the CLI and pass raw values through
the runtime config. That would preserve the current ambiguity and make later
encoder stages decide what values are valid, so it does not establish the
Baseline v1 contract.

### Runtime policy, not bitstream configuration

The preset will live in `EncoderRuntimeConfig` beside `ThreadCount`, not in
`EncoderConfig`. This preserves the established boundary that `EncoderConfig`
describes what is encoded, while runtime config describes how work is attempted.
Until a real packet path exists, tests will prove the preset is retained but does
not create packets.

Alternative considered: adding speed to `EncoderConfig` because future preset
choices may influence decisions that affect output bytes. That would blur the
current bitstream-affecting boundary too early. Later deterministic mode-decision
work can explicitly document whether preset changes are allowed to choose
different legal syntax, while correctness remains invariant.

### Thin CLI parsing

The CLI will keep accepting `--speed <u8>` for user ergonomics and immediately
convert to `SpeedPreset` before constructing `EncoderRuntimeConfig`. The library
owns validation and error text; the CLI only maps it into `anyhow`.

## Flight manifest

- Change ID: `encoder-speed-presets`
- Feature IDs: `ENC-SPEED-PRESETS`
- Base commit: `00352ae5317be8ffb4c7d3df6bbb9bf12a21fa86`
- Depends on merged changes: `encoder-program-contract`,
  `encoder-recon-dependency`, `encoder-frame-input-views`,
  `encoder-context-state-machine`, `encoder-syntax-ir`,
  `encoder-minimal-header-plan`
- Exact files/directories owned by this PR:
  - `crates/splot-encode/src/runtime.rs`
  - `crates/splot-encode/src/context.rs`
  - `crates/splot-encode/src/lib.rs`
  - `crates/splot-cli/src/commands/encode.rs`
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `openspec/changes/archive/2026-06-18-encoder-speed-presets/**`
- Exact files/directories forbidden to this PR:
  - `Cargo.toml`
  - `Cargo.lock`
  - `crates/splot-core/**`
  - `crates/splot-recon/**`
  - `crates/splot-decode/**`
  - `crates/splot-validate/**`
  - `fuzz/**`
  - `docs/spec/av2/**`
- Public APIs/types owned: `SpeedPreset`, `EncoderRuntimeConfig::with_speed_preset`,
  `EncoderRuntimeConfig::speed_preset`, `Context::speed_preset`
- Matrix rows owned: `ENC-SPEED-PRESETS`
- Generated files owned: `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`
- Open sibling PRs audited: none (`gh pr list --state open` returned `[]`)
- Changed-file intersection with each sibling PR: none
- Semantic overlap with each sibling PR: none
- Can build/test/merge directly onto main without another open PR: yes

## Risks / Trade-offs

- Numeric range may need adjustment once real mode-decision presets land.
  Mitigation: expose range constants and keep this as the framework row, not a
  quality/performance promise.
- CLI errors are user-facing before encode is implemented. Mitigation: use the
  library's typed validation and keep the existing "not yet implemented" output
  for accepted presets.
- Future presets may affect legal syntax choices. Mitigation: this change states
  only that presets must never affect syntax correctness; later output work must
  prove deterministic legal output per preset.
