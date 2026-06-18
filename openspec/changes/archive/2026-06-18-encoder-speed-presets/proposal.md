## Why

`splot encode --speed` is currently accepted by the CLI but ignored before the
encoder reaches the runtime API. Baseline Encoder Profile v1 requires documented
speed presets whose numeric value never changes syntax correctness, so the
encoder needs a typed runtime-policy boundary before any packet-producing path
can depend on preset-specific scheduling or search decisions.

## What Changes

- Add a typed `ENC-SPEED-PRESETS` runtime preset model for the current supported
  numeric preset range.
- Wire the CLI `--speed` value into `EncoderRuntimeConfig` instead of dropping it.
- Keep speed presets separate from `EncoderConfig`, because presets are runtime
  policy and not bitstream-affecting syntax configuration.
- Add focused API/CLI tests proving defaults, accepted values, rejected values,
  and non-emitting lifecycle behavior.
- Update the implementation matrix and generated status docs with proof for the
  preset framework only.
- Do not add public packet output, rate control, mode decision, writer
  integration, or any AV2 syntax emission.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `encoder-tools`: define the initial typed speed-preset framework for
  `ENC-SPEED-PRESETS` while preserving the no-output encoder behavior.

## Impact

- Affected crates: `crates/splot-encode`, `crates/splot-cli`.
- Affected docs/status: `docs/IMPLEMENTATION-MATRIX.toml`,
  generated feature/status coverage docs, and encoder roadmap/gap audit notes if
  needed.
- No new dependencies, no dependency graph change, no CLI success-path output,
  and no validator diagnostic changes.
