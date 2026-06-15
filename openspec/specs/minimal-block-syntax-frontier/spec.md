# minimal-block-syntax-frontier Specification

## Purpose
Track the narrow decoder boundary that moves the supported minimal flat-intra block-symbol trace into `splot-decode` tile-payload code without claiming broad `decode_block()`, `decode_tile()`, reconstruction, or complete CDF lifecycle support.

## Requirements
### Requirement: Minimal Flat Intra Block Symbol Trace Frontier
The decoder SHALL provide a crate-private tile-payload trace frontier that consumes only the supported minimal-tier flat intra block symbols after the partition frontier using AV2 v1.0.0 §5.20.4.1, §5.20.5.1, §5.20.5.3, §5.20.5.5, §5.20.5.6, §5.20.6.1, §5.20.6.2, §5.20.7.23, §5.20.7.24, §5.20.7.27, §8.2.4, §8.2.6, §8.3.1, §8.3.2, and generated §9.3 CDF defaults.

#### Scenario: Traced minimal block symbols are accepted
- **WHEN** the minimal runtime reaches the first root `decode_block()` frontier for the committed 64x64 flat intra fixture
- **THEN** the tile-payload frontier consumes the traced `y_mode_set`, `y_mode_index`, luma/U all-zero transform, `uv_mode`, and V all-zero transform symbols and validates `exit_symbol()`

#### Scenario: Block symbol mismatch fails closed
- **WHEN** a tile payload mutation changes one of the traced flat block symbols
- **THEN** the minimal runtime reports `decode/unsupported-feature` with a stable minimal block-symbol reason and does not construct output

#### Scenario: Output identity is preserved
- **WHEN** the committed minimal fixture is decoded through hash or Y4M output
- **THEN** the output hash and Y4M bytes remain byte-identical to the pre-change supported minimal tier

#### Scenario: Broad decode tile remains out of scope
- **WHEN** a conforming stream requires syntax outside the traced flat block-symbol subset
- **THEN** the decoder keeps failing closed with `decode/unsupported-feature` rather than claiming broad `decode_block()`, `decode_tile()`, or reconstruction support
