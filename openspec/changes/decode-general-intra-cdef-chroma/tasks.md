# Tasks

## Fixture

- [x] 1.1 Generate a project-owned 8-bit 4:2:0 CDEF-active intra fixture the encoder
      gave NONZERO chroma strengths (`uv_pri 2`, `uv_sec 4`) while staying
      general-intra-admissible (`CdefStrengths == 1`,
      `cdef_on_skip_txfm_frame_enable == 1`), via avmenc with chroma-textured
      cosine-AC content and the broad-tools-off recipe.
- [x] 1.2 Confirm avmdec == dav2d byte-for-byte (raw md5
      `d783f353078cf156ba23dcfd3b2b50ad`) and that the chroma CDEF genuinely changes
      U/V samples (over a thousand each, isolated by re-decoding the same bitstream
      with the chroma strengths forced to zero — luma stays identical), and that the
      encode is deterministic (byte-identical re-encode).

## Tests

- [x] 2.1 Add `syn-2sb-cdefuv-intra-128x64-q170.ivf` to the positive CDEF
      decode-hash test, pinning the frame hash
      `9b11d0effa3b93e84c63306e9ac865921e33f6e098cc35fbc472cbd6096ee3e6`.
- [x] 2.2 Add deterministic `cdef.rs` unit tests: a nonzero-uv strength set derings
      a synthetic chroma ripple (chroma-only, bounded, luma untouched), a zero-uv
      set is a chroma no-op, and the `Cdef_Uv_Dir` direction selection changes the
      chroma output as a function of the luma `yDir` ONLY when `uv_pri != 0`.

## Route Gate And Docs

- [x] 3.1 Relax the general intra route gate to admit nonzero-uv CDEF (drop the
      `uv_*_strength == 0` clause, keep `CdefStrengths == 1`,
      `cdef_on_skip_txfm_frame_enable == 1`, a present damping / strength set, and
      the 8-bit restriction).
- [x] 3.2 Un-qualify the `cdef.rs` module docstring's "luma-only, chroma-no-op
      subset" wording.

## Tracking

- [x] 4.1 Add matrix, decoder-support, LOCAL-REFERENCE-EVIDENCE (avm + dav2d md5s +
      the digest-equality assertion subtable), and conformance manifest entries for
      the nonzero-uv fixture.
- [x] 4.2 Regenerate the four generated docs and run the required checks
      (`cargo xtask ci`, `cargo xtask conformance`, the dupehound diff ratchet,
      `openspec validate decode-general-intra-cdef-chroma --strict`).
