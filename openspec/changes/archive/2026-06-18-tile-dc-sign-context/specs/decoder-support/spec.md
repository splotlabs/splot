## ADDED Requirements

### Requirement: dc_sign sign CDF context

The `splot-decode` tile CDF selection subset SHALL derive the AV2 § 8.3.2 `dc_sign` sign CDF context, tracked by `DECODE-TILE-CDF-SELECTION-BOUNDARY`. `dc_sign_ctx` SHALL net the DC-sign votes of the block's above and left neighbours — `AboveDcContext[plane][x4 + k]` for `k` in `0..w4` and `LeftDcContext[plane][y4 + k]` for `k` in `0..h4`, where a sign value of `1` decrements and `2` increments a running `dcSign` — and return `1` when `dcSign < 0`, `2` when `dcSign > 0`, and `0` otherwise (the inner index of `TileDcSignCdf[ptype][isHidden][ctx]`). `above_dc` and `left_dc` SHALL be the caller-supplied `AboveDcContext` / `LeftDcContext` plane slices whose lengths are the spec `MiCols` / `MiRows` bounds, so reads past either slice are skipped; the loop SHALL break once the monotonic index leaves the slice, so a pathological `w4` / `h4` cannot spin and the function is total and panic-free. It SHALL NOT be read by any decode path in this change, so the minimal-fixture decode output SHALL be unchanged. The `idtx_sign` sign context, the DC-context buffers, and the coefficient decode loop remain partial.

#### Scenario: dc_sign nets above and left votes

- **WHEN** `cargo test -p splot-decode tile_payload --locked` runs
- **THEN** `dc_sign_ctx` returns context 1 for a net-negative neighbour sum, 2 for
  net-positive, and 0 for a balanced or empty sum, with tests pinning the position
  offset and the out-of-slice (`MiCols` / `MiRows`) skip
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: dc_sign is total over pathological geometry

- **WHEN** `w4` / `h4` or the position offsets are far larger than the slices
- **THEN** the loop terminates without spinning and `dc_sign_ctx` returns without
  panicking
- **AND** library code does not panic, overflow, or unwrap

#### Scenario: Adding the context does not change decode output

- **WHEN** the minimal flat-intra fixture is decoded to a hash, raw, or Y4M output
- **THEN** the output bytes are identical to before the context was added (the
  derivation is not read by any decode path)

#### Scenario: Coefficient CDF selection remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the tile CDF selection boundary records the `dc_sign` sign context
- **AND** broader § 8.3 coefficient CDF selection (the `idtx_sign` context, the
  DC-context buffers, and the coefficient decode loop) remains partial
