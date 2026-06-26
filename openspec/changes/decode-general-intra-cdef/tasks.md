# Tasks

## CDEF Leaf Math (splot-recon)

- [x] 1.1 Add `crates/splot-recon/src/cdef_filter.rs` with the AV2 § 7.18.2
      `cdef_direction` (direction search over the pre-shifted 8x8 luma block, with
      the exact `partial[][]` index mapping, the inline `Div_Table`, the i64
      cost accumulators, `yDir = argmax`, and `var`), the § 7.18.3
      `cdef_constrain`, and the § 7.18.3 `cdef_filter_sample` (primary/secondary
      tap accumulation with the inline `Cdef_Pri_Taps` / `Cdef_Sec_Taps`, the
      `(8 + sum - (sum < 0)) >> 4` rounding, and the `Clip3(min, max, ...)` clamp).
- [x] 1.2 Expose the inline § 7.18.3 `Cdef_Directions` and § 7.18.1 `Cdef_Uv_Dir`
      constants for the caller's `cdef_get_at` neighbour addressing, and export the
      module from `crates/splot-recon/src/lib.rs`.

## CDEF Orchestration (splot-decode)

- [x] 2.1 Add `crates/splot-decode/src/runtime_minimal/cdef.rs` implementing the
      AV2 § 7.18 / § 7.18.1 64x64-unit → 8x8-block traversal over the `splot-recon`
      primitives: snapshot the deblocked frame (§ 7.18 filters `CurrFrame` into
      `CdefFrame`, so every tap must read a pre-CDEF sample), iterate the 8x8 blocks
      (`cdef_idx == 0` everywhere), derive the § 7.18.1 luma priStr (var-scaled) /
      secStr / dir / damping and the chroma priStr / secStr / dir
      (`Cdef_Uv_Dir[subX][subY][yDir]`) / damping, fetch the six § 7.18.3
      directional taps via `cdef_get_at` with the § 5.20.9.3 `is_inside_filter_region`
      (single tile → `is_inside_frame`) availability check, and write the deringed
      samples back in place.
- [x] 2.2 Run the CDEF pass AFTER § 7.17 deblocking, reading the deblocked
      `CurrFrame`, before `workspace.freeze()` (filter order: deblock → CDEF). A
      CDEF-off frame leaves the params `None`, so the pass is skipped.

## Route Gate

- [x] 3.1 Relax the general intra route gate to admit a CDEF-active frame in the
      verified subset (`CdefStrengths == 1` so § 5.20.10.1 `read_cdef` reads no
      per-block symbol; `cdef_on_skip_txfm_frame_enable == 1`; present damping /
      strength set), keeping GDF/CCSO/loop-restoration rejected.
- [x] 3.2 Reject a multi-strength (`CdefStrengths > 1`) frame and a 10-bit
      CDEF-active frame (no oracle fixture pins the per-block read_cdef symbols or
      the 10-bit pass); a CDEF-off frame is unaffected at any bit depth.

## Tests And Tracking

- [x] 4.1 Add the `syn-2sb-cdef-intra-128x64-q130.ivf`,
      `syn-2sb-cdef-intra-128x64-q120.ivf`, and
      `syn-2sb-cdefdeblock-intra-128x64-q100.ivf` conformance fixtures and a
      positive decode test pinning the deringed frame hash (raw md5
      `192e3935f9892345a14e02cb4baf4ba5` / `2319a8f00af1ebb919a52ba18d90f4a1` /
      `472d95801ce2a112160bcdfee93957d5`).
- [x] 4.2 Confirm a CDEF-off frame stays byte-identical and the existing 8-bit and
      10-bit corpus is unchanged.
- [x] 4.3 Add deterministic `cdef.rs` unit tests (a flat frame is unchanged; a
      small ringing step is deringed within bounds) and `cdef_filter.rs` leaf-math
      tests (direction, constrain, tap filter, table spot-checks).
- [x] 4.4 Add matrix, decoder-support, LOCAL-REFERENCE-EVIDENCE, and conformance
      manifest entries for `DECODE-GENERAL-INTRA-CDEF` and `RECON-CDEF-FILTER`.
- [x] 4.5 Regenerate generated docs and run the required checks
      (`cargo xtask ci`, `conformance`, `check-fixtures`, both dupehound gates).
