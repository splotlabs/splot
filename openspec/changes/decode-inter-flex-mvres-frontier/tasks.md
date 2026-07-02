# Tasks

## 1. Flexible MV resolution (§ 5.20.7.13)
- [x] 1.1 Wire `UseMostProbablePrecision`/`PbMvPrecision` CDF selectors and the
      `JointShell0/1` sub-one-pel rows through the tile CDF layers.
- [x] 1.2 Read the per-block precision between DRL and `assign_mv` on the
      single-reference NEWMV and WARP_NEWMV paths; thread it into `read_mv`.
- [x] 1.3 Implement § 5.20.7.13 `lower_mv_precision` and apply it to
      NEWMV-family predictors below `MV_PRECISION_HALF_PEL` (asymmetric-value
      unit test).
- [x] 1.4 Record `UseMostProbablePrecisions`/`MvPrecisions` grid values for
      every block class (intra/IntrABC = explicit, non-NEWMV inter = most
      probable) and derive the § 8.3.2 contexts from them.
- [x] 1.5 Check frame AND block precision in the MVD sign-derivation gate.

## 2. Neighbour lists (§ 5.20.7.2 / § 8.3.2)
- [x] 2.1 Collect `NPosBuf` and `NPos` in one probe pass; route each context
      per the spec table; count both reference lists in `count_refs`.
- [x] 2.2 Re-point the superblock-top neighbour tests to the spec semantics.

## 3. Motion-mode prefix (§ 5.20.7.14/15)
- [x] 3.1 Read the SIMPLE-path `inter_intra` flag (TileInterIntraCdf,
      Size_Group context) with the `motion_mode_allowed` size gate;
      frontier-reject a set flag.

## 4. Verification
- [x] 4.1 `syn-2frame-inter-64x64-10bit.ivf` decodes byte-identical to avmdec
      (pinned frame hashes).
- [x] 4.2 `syn-grid-inter-128x128-q80.ivf` decodes byte-identical to avmdec
      (pinned frame hashes; gate-pin tests converted to positive decodes).
- [x] 4.3 ac0ej3 `--limit=1` reproduces the AVM frame-0 sentinel through the
      production CLI; the full-stream gate holds at the first fully-tooled
      inter frame (byte 8345).
- [x] 4.4 Add the ignored `ac0ej3_full_stream_avm_compare` harness.
