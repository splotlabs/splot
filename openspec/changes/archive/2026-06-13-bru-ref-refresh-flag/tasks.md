# Tasks

- [x] `reference_state_checks`: add the §6.17.2 :4596 refresh-mask-bit check
      (`frame-header/bru-ref-refresh-flag-unset`), bounds-checked + shift-guarded.
- [x] Parameterize `ref_inter_bru` with `refresh_frame_flags`; update callers (conformant bit
      set). Add firing (refresh==0) + silent (refresh==1) tests; extend the conformant-silent
      test to assert the new rule absent.
- [x] Register the diagnostic in `docs/VALIDATOR-DIAGNOSTICS.md`; refresh the
      `bru-without-immediate-output` residual clause.
- [x] Update matrix `AV2-6.17.2-FRAME-HEADER-INFO-SEMANTICS` notes/diagnostics/proof; add the `BLOCKED:` note for the
      residual inter-reference clauses (§7.7 get_ref_frames / get_disp_order_hint).
- [x] `cargo xtask ci` bare.
