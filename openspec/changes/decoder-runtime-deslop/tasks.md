## 1. Runtime Cleanup

- [x] 1.1 Extract shared block context, prediction dispatch, residual planning, and inter prediction helpers.
- [x] 1.2 Compress repeated unsupported-feature diagnostics behind capability helpers.
- [x] 1.3 Remove stale private Rustdoc and long comments from touched runtime files.
- [x] 1.4 Collapse cardinal H/V intra prediction onto the shared directional edge model.
- [x] 1.5 Collapse repeated CDF row lifecycle, chroma/recon write-back, inter residual, and general-intra neighbour code.
- [x] 1.6 Collapse repeated coefficient transform-size derivation, FSC pass plumbing, IntrABC probe scans, partition child construction, and runtime diagnostics.
- [x] 1.7 Collapse CDF row access/update plumbing, intra joint neighbour sampling, IntrABC range comments, MV-stack neighbour probes, and recon directional-angle middle-reference walks.

## 2. Budget Gates

- [x] 2.1 Ratchet `tools/comments/budget.toml` to 419.
- [x] 2.2 Ratchet `tools/dupehound/budget.toml` to 6654.
- [x] 2.3 Confirm source-line hard allowances remain empty.
- [x] 2.4 Ratchet `tools/comments/budget.toml` to 353 and `tools/dupehound/budget.toml` to 6645.
- [x] 2.5 Ratchet `tools/comments/budget.toml` to 316 and `tools/dupehound/budget.toml` to 6606.
- [x] 2.6 Keep `tools/comments/budget.toml` at 316 and ratchet `tools/dupehound/budget.toml` to 6569.
- [x] 2.7 Ratchet `tools/comments/budget.toml` to 279 and `tools/dupehound/budget.toml` to 6539.

## 3. Verification

- [x] 3.1 Regenerate feature/status docs.
- [x] 3.2 Run focused decoder tests and `cargo xtask ci`.
  - 2026-06-30: `cargo check --workspace --all-targets --locked`
  - 2026-06-30: `cargo test -p splot-decode --lib runtime_minimal --locked`
  - 2026-06-30: `cargo test -p splot-recon --lib intra_directional_angle --locked`
  - 2026-06-30: `cargo xtask check-comment-density`
  - 2026-06-30: `cargo xtask check-duplication`
  - 2026-06-30: `cargo xtask check-ai-slop`
  - 2026-06-30: `cargo xtask ci`

## 4. Rust File Orchestration

- [x] 4.1 Inventory module buckets: `runtime_minimal`, `runtime_minimal/inter`,
  `runtime_minimal/wienerns_lr`, `runtime_minimal_recon`, `tile_payload`,
  `tile_payload/cdf`, `tile_payload/coeff_loop`, and decoder-facing `splot-recon`.
- [x] 4.2 Per-file clean `runtime_minimal/wienerns_lr{.rs,/recon.rs,/intrabc_ref_mv_stack.rs,/intrabc_records.rs,/tx_records.rs,/diagnostics.rs,/live_storage.rs,/source_read_math.rs,/tx_records/{ccso.rs,max_rect.rs,skip_records.rs}}`.
- [x] 4.3 Run the module-level `wienerns_lr` unification pass after 4.2.
- [x] 4.4 Per-file clean `runtime_minimal/inter{.rs,/block.rs,/compound.rs,/cross_frame.rs,/find_mv_stack.rs,/find_mv_stack/tests.rs,/mc.rs,/mv_scaling.rs,/read_mv.rs,/read_mv/tests.rs,/single_ref.rs,/single_ref/tests.rs,/test_support.rs,/tests.rs,/lr_live_storage_tests.rs,/lr_source_read_tests.rs}`.
- [x] 4.5 Run the module-level `runtime_minimal/inter` unification pass after 4.4.
- [x] 4.6 Ratchet comment and duplication budgets to the measured post-`runtime_minimal/inter` cleanup counts.
- [x] 4.7 Run focused inter tests and `cargo xtask ci`.

## 5. Root Runtime and Reconstruction Orchestration

- [x] 5.1 Inventory remaining root runtime files: `runtime_minimal{.rs,/block_context.rs,/capability.rs,/cdef.rs,/deblock.rs,/general_intra.rs,/general_intra_tests.rs,/general_intra_tests/general_intra_cdef_tests.rs,/intra_prediction.rs,/limits.rs,/reference_buffer.rs,/residual_pipeline.rs}` and `runtime_minimal_recon{.rs,/chroma_directional.rs,/tests.rs}`.
- [x] 5.2 Per-file clean the inventoried root runtime and reconstruction files.
- [x] 5.3 Run the module-level root runtime/reconstruction unification pass after 5.2.
- [x] 5.4 Ratchet comment and duplication budgets to the measured post-root-runtime cleanup counts.
- [x] 5.5 Run focused root runtime/reconstruction tests and `cargo xtask ci`.

## 6. Tile Payload Root and Partition Orchestration

- [x] 6.1 Inventory `tile_payload.rs` and `tile_payload/{block_decoded_state.rs,block_symbol.rs,coeff_state.rs,derived_tests.rs,general_intra_block.rs,general_intra_residual.rs,general_intra_residual/tests.rs,input.rs,intra_joint_modes.rs,mi_size_state.rs,partition.rs,partition_allowed.rs,partition_allowed/spec_table_tests.rs,partition_size.rs,partition_traversal.rs,partition_traversal/partition_children.rs,partition_traversal_tests.rs,runtime_frontier.rs,tests.rs}` while leaving nested `tile_payload/cdf` and `tile_payload/coeff_loop` for later buckets.
- [x] 6.2 Per-file clean the inventoried tile payload root and partition files.
- [x] 6.3 Run the module-level tile payload root/partition unification pass after 6.2.
- [x] 6.4 Ratchet comment and duplication budgets to the measured post-tile-payload-root cleanup counts.
- [x] 6.5 Run focused tile payload tests and `cargo xtask ci`.

## 7. Tile Payload CDF Orchestration

- [x] 7.1 Inventory `tile_payload/cdf{.rs,/block_context.rs,/block_read.rs,/block_rows.rs,/block_rows/mv.rs,/coeff_context.rs,/coeff_rows.rs,/context.rs,/lifecycle.rs,/partition_read.rs,/tests.rs,/util.rs}`.
- [x] 7.2 Per-file clean the inventoried tile payload CDF files.
- [x] 7.3 Run the module-level tile payload CDF unification pass after 7.2.
- [x] 7.4 Ratchet comment and duplication budgets to the measured post-tile-payload-CDF cleanup counts.
- [x] 7.5 Run focused tile payload CDF tests and `cargo xtask ci`.

## 8. Tile Payload Coefficient Loop Orchestration

- [x] 8.1 Inventory `tile_payload/coeff_loop{.rs,/base_level_pass.rs,/base_level_pass_tests.rs,/base_symbol.rs,/base_symbol_tests.rs,/branch.rs,/eob_symbol_tests.rs,/fsc_level_pass.rs,/fsc_level_pass_tests.rs,/fsc_quant_pass.rs,/fsc_quant_pass_tests.rs,/fsc_sign_pass.rs,/fsc_sign_pass_tests.rs,/level_state.rs,/level_state_tests.rs,/max_level.rs,/ordinary_branch_coeffs_geometry_tests.rs,/ordinary_branch_lossless_tests.rs,/ordinary_branch_mode_to_txfm_tests.rs,/ordinary_branch_tx_set_tests.rs,/ordinary_pass.rs,/ordinary_pass/geometry.rs,/ordinary_pass_tests.rs,/ordinary_state_context_tests.rs,/quant_pass.rs,/quant_state.rs,/read_quant.rs,/scan_walk.rs,/sign_symbol.rs,/test_support.rs,/use_fsc_branch.rs,/use_fsc_branch_tests.rs,/use_fsc_frame_facts_tests.rs}`.
- [x] 8.2 Per-file clean the inventoried tile payload coefficient-loop files.
- [x] 8.3 Run the module-level tile payload coefficient-loop unification pass after 8.2.
- [x] 8.4 Ratchet comment and duplication budgets to the measured post-coefficient-loop cleanup counts.
- [x] 8.5 Run focused coefficient-loop tests and `cargo xtask ci`.

## 9. Reconstruction Intra Prediction and Workspace Orchestration

- [x] 9.1 Inventory `splot-recon/src/{intra.rs,intra_basic.rs,intra_dc_math.rs,intra_dc_subsampled.rs,intra_directional.rs,intra_directional_angle.rs,intra_directional_angle_tests.rs,intra_ibp_angular.rs,intra_ibp_angular/tests.rs,intra_ibp_dc.rs,intra_smooth.rs,workspace.rs,workspace_edges.rs,workspace_intra_dc.rs,workspace_intra_directional_angle.rs,workspace_intra_directional_angle_tests.rs,workspace_tests.rs}`.
- [x] 9.2 Per-file clean the inventoried reconstruction intra/workspace files.
- [x] 9.3 Run the module-level reconstruction intra/workspace unification pass after 9.2.
- [x] 9.4 Ratchet comment and duplication budgets to the measured post-reconstruction-intra cleanup counts.
- [x] 9.5 Run focused reconstruction intra/workspace tests and `cargo xtask ci`.

## 10. Encoder Transform, Quantization, and Core Test Writer Orchestration

- [x] 10.1 Inventory `splot-encode/src/{forward_transform.rs,forward_transform_16x16.rs,quantization.rs,quantization_16x16.rs,quantization_shared.rs}` and the duplicated core writer `Bits` test helpers reported by `dupehound`.
- [x] 10.2 Collapse 4x4/16x16 forward DCT row/column pass arithmetic onto one shared const-generic helper.
- [x] 10.3 Collapse 4x4/16x16 quantization validation, coefficient quantization, and dequantization plumbing onto shared helpers and a shared fixed-parameter type.
- [x] 10.4 Replace copied core writer bit builders with `test_bits::Bits`.
- [x] 10.5 Ratchet `tools/dupehound/budget.toml` to 5778 and run focused core/encode tests.
