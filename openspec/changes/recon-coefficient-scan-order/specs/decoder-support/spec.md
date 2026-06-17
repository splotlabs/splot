## ADDED Requirements

### Requirement: Coefficient scan order get_scan

The repository SHALL provide a scheduler-free `splot-recon` primitive for the AV2 § 5.20.7.30 `get_scan` coefficient scan order, tracked by `RECON-COEFFICIENT-SCAN-ORDER`. The `coefficient_scan_order` function SHALL write, for a `w * h` transform block and a `TransformClass`, the order in which transform coefficients are scanned — each `out[c]` being the flattened `y * w + x` position of the c-th scanned coefficient — implementing the three spec classes: `TX_CLASS_VERT` as row-major raster order, `TX_CLASS_HORIZ` as column-major (transpose) order, and `TX_CLASS_2D` as the anti-diagonal scan (each anti-diagonal `x + y` traversed from high `y` / low `x` to low `y` / high `x`). The block shape SHALL be caller-resolved (`w` / `h` each 4, 8, 16, or 32), and the function SHALL return a typed `ReconError` for an unsupported shape or a wrong-length output buffer, total and panic-free. The output for every supported shape and class SHALL be a permutation of `0..w*h`. The primitive SHALL NOT implement `get_tx_class`, the coefficient decode loop, the wiring of the scan into a decode path, the § 7.15.3 secondary-transform scan, or runtime decode output.

#### Scenario: get_scan succeeds with self-contained tests

- **WHEN** `cargo test -p splot-recon coefficient_scan --locked` runs
- **THEN** the test suite covers the hand-traced 4x4 `TX_CLASS_2D` order, the
  `TX_CLASS_VERT` identity and `TX_CLASS_HORIZ` transpose orders, and that the
  output is a valid permutation of `0..w*h` for all 4/8/16/32 shapes and all three
  classes
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: Invalid scan shape or length is typed

- **WHEN** callers request a `w` / `h` outside 4/8/16/32, or an output buffer not
  exactly `w * h` long
- **THEN** `coefficient_scan_order` returns a structured `ReconError`
- **AND** library code does not panic, overflow, or unwrap

#### Scenario: Coefficient decode remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the matrix records the `get_scan` coefficient scan order as supported
- **AND** the coefficient decode loop and broader reconstruction remain partial
  until `get_tx_class`, the decode loop, and the runtime wiring are implemented
  and proven
