# AV2 v1.0.0 — § 5. Syntax structures

<!-- Verbatim mirror of the AOM AV2 v1.0.0 specification (© Alliance for Open Media). The PDF is normative; this is a faithful `pdftotext -layout` copy. See [./README.md](./README.md) and [./index.md](./index.md). Do not hand-edit: regenerate via scripts/spec/regenerate-av2-spec.sh. -->

<a id="s-5"></a>

## § 5 Syntax structures

```text
§   5. Syntax structures
```

<a id="s-5-1"></a>

### § 5.1 General

```text
§   5.1. General
    This section presents the syntax structures in a tabular form. The meaning of each of the syntax elements
    is presented in § 6 Syntax structures semantics.

```

<a id="s-5-2"></a>

### § 5.2 OBU syntax

```text
§   5.2. OBU syntax
```

<a id="s-5-2-1"></a>

#### § 5.2.1 General OBU syntax

```text
§   5.2.1. General OBU syntax

     open_bitstream_unit( sz ) {                                                                Descriptor

       obu_header()

       obuPayloadSize = sz - 1 - obu_header_extension_flag

       startPosition = get_position( )

       load_xlayer_context( obu_xlayer_id )

       if ( obu_type == OBU_SEQUENCE_HEADER ) {

         sequence_header_obu( )

       } else if ( obu_type == OBU_TEMPORAL_DELIMITER ) {

         FirstPictureInTU = 1

         temporal_delimiter_obu( )

       } else if ( obu_type == OBU_MSDO ) {

         multistream_decoder_operation_obu()

       } else if ( obu_type == OBU_MULTI_FRAME_HEADER ) {

         multi_frame_header_obu( )

       } else if ( is_sef() || is_tip_frame() || obu_type == OBU_BRIDGE_FRAME ) {

         frame_header( 1 )

       } else if ( obu_type == OBU_METADATA_SHORT ) {

         metadata_short_obu( obuPayloadSize )

       } else if ( obu_type == OBU_METADATA_GROUP ) {

         metadata_group_obu( )

       } else if ( is_tile_group() ) {

         tile_group_obu( obuPayloadSize )

       } else if ( obu_type == OBU_LAYER_CONFIGURATION_RECORD ) {

         layer_config_record_obu( )

       } else if ( obu_type == OBU_ATLAS_SEGMENT ) {

         atlas_segment_info_obu( )

       } else if ( obu_type == OBU_OPERATING_POINT_SET ) {

         operating_point_set_obu( )

       } else if ( obu_type == OBU_BUFFER_REMOVAL_TIMING ) {

         buffer_removal_timing_obu( )

       } else if ( obu_type == OBU_QUANTIZATION_MATRIX ) {

         quantizer_matrix_obu( )



    AV2 Specification                                                                           Page 52 of 1169
     } else if ( obu_type == OBU_FILM_GRAIN ) {

         film_grain_obu( )

     } else if ( obu_type == OBU_CONTENT_INTERPRETATION ) {

         content_interpretation_obu( )

     } else if ( obu_type == OBU_PADDING ) {

         padding_obu( )

     } else {

         reserved_obu( )

     }

     usedArith = is_tile_group()

     currentPosition = get_position( )

     parsedPayloadBits = currentPosition - startPosition

     remainingPayloadBits = obuPayloadSize * 8 - parsedPayloadBits

     if ( obuPayloadSize > 0 && !usedArith ) {

         if ( is_extensible_obu() ) {

             // OBUs with extensible payloads

             obu_extension_flag                                                             f(1)

             if ( obu_extension_flag ) {

                 obu_extension_data( remainingPayloadBits - 1 )

             } else {

                 trailing_bits( remainingPayloadBits - 1 )

             }

         } else {

             trailing_bits( remainingPayloadBits )

         }

     }

     save_xlayer_context( obu_xlayer_id )

 }


where some helper functions used to identify collections of OBU types are specified as:

 is_tip_frame() {
     return obu_type == OBU_LEADING_TIP || obu_type == OBU_REGULAR_TIP
 }


 is_sef() {
     return obu_type == OBU_LEADING_SEF || obu_type == OBU_REGULAR_SEF
 }


 is_tile_group() {
     return obu_type == OBU_LEADING_TILE_GROUP ||
            obu_type == OBU_REGULAR_TILE_GROUP ||
            obu_type == OBU_CLOSED_LOOP_KEY ||
            obu_type == OBU_OPEN_LOOP_KEY ||




AV2 Specification                                                                         Page 53 of 1169
                        obu_type == OBU_SWITCH ||
                        obu_type == OBU_RAS_FRAME
     }


     is_extensible_obu() {
         return obu_type == OBU_SEQUENCE_HEADER ||
                obu_type == OBU_MULTI_FRAME_HEADER ||
                obu_type == OBU_LAYER_CONFIGURATION_RECORD ||
                obu_type == OBU_CONTENT_INTERPRETATION ||
                obu_type == OBU_OPERATING_POINT_SET ||
                obu_type == OBU_ATLAS_SEGMENT
     }


     obu_extension_data( sz ) {                                                            Descriptor

         for ( i = 0; i < sz; i++ ) {

             obu_extension_data_bit                                                           f(1)

         }

     }


```

<a id="s-5-2-2"></a>

#### § 5.2.2 OBU header syntax

```text
§   5.2.2. OBU header syntax

     obu_header() {                                                                        Descriptor

         obu_header_extension_flag                                                            f(1)

         obu_type                                                                             f(5)

         obu_tlayer_id                                                                        f(2)

         if ( obu_header_extension_flag == 1 ) {

             obu_mlayer_id                                                                    f(3)

             obu_xlayer_id                                                                    f(5)

         } else {

             obu_mlayer_id = 0

        obu_xlayer_id = ( obu_type == OBU_MSDO || obu_type == OBU_TEMPORAL_DELIMITER ) ?
     GLOBAL_XLAYER_ID : 0

         }

     }


```

<a id="s-5-2-3"></a>

#### § 5.2.3 Trailing bits syntax

```text
§   5.2.3. Trailing bits syntax

     trailing_bits( nbBits ) {                                                             Descriptor

         trailing_one_bit                                                                     f(1)

         nbBits--

         while ( nbBits > 0 ) {

             trailing_zero_bit                                                                f(1)

             nbBits--

         }

     }




    AV2 Specification                                                                      Page 54 of 1169
```

<a id="s-5-2-4"></a>

#### § 5.2.4 Byte alignment syntax

```text
§   5.2.4. Byte alignment syntax

     byte_alignment( ) {                                                                            Descriptor

         while ( get_position( ) & 7 ) {

             zero_bit                                                                                  f(1)

         }

     }


```

<a id="s-5-3"></a>

### § 5.3 Reserved OBU syntax

```text
§   5.3. Reserved OBU syntax
     reserved_obu( ) {                                                                              Descriptor

     }



         NOTE: Reserved OBUs do not have a defined syntax. The obu_type reserved values are reserved for
         future use by AOMedia. Decoders should ignore the entire OBU if they do not understand the
         obu_type. The last byte of the valid content of the payload data for this OBU type is considered to be
         the last byte that is not equal to zero. This rule is to prevent the dropping of valid bytes by systems
         that interpret trailing zero bytes as a continuation of the trailing bits in an OBU. This implies that
         when any payload data is present for this OBU type, at least one byte of the payload data (including
         the trailing bit) shall not be equal to 0.


```

<a id="s-5-4"></a>

### § 5.4 Sequence header OBU syntax

```text
§   5.4. Sequence header OBU syntax
```

<a id="s-5-4-1"></a>

#### § 5.4.1 General sequence header OBU syntax

```text
§   5.4.1. General sequence header OBU syntax

     sequence_header_obu( ) {                                                                       Descriptor

         seq_header_id                                                                                uvlc()

         seq_profile_idc                                                                               f(5)

         single_picture_header_flag                                                                    f(1)

         seq_level_idx                                                                                 f(5)

         if ( seq_level_idx > 3 && !single_picture_header_flag ) {

             seq_tier                                                                                  f(1)

         } else {

             seq_tier = 0

         }

         chroma_format_idc                                                                            uvlc()

         bit_depth_idc                                                                                uvlc()

         set_chroma_format_and_bit_depth( )

         if ( single_picture_header_flag ) {

             seq_lcr_id = 0

             still_picture = 1

             max_tlayer_id = 0

             max_mlayer_id = 0

             SeqMaxMlayerCnt = 1




    AV2 Specification                                                                                Page 55 of 1169
       monotonic_output_order_flag = 1

   } else {

       seq_lcr_id                                               f(3)

       still_picture                                            f(1)

       max_tlayer_id                                            f(2)

       max_mlayer_id                                            f(3)

       if    ( max_mlayer_id > 0 ) {

           n = CeilLog2(max_mlayer_id + 1)

           seq_max_mlayer_cnt_minus_1                           f(n)

           SeqMaxMlayerCnt = seq_max_mlayer_cnt_minus_1 + 1

       } else {

           SeqMaxMlayerCnt = 1

       }

       monotonic_output_order_flag                              f(1)

   }

   frame_width_bits_minus_1                                     f(4)

   frame_height_bits_minus_1                                    f(4)

   n = frame_width_bits_minus_1 + 1

   max_frame_width_minus_1                                      f(n)

   n = frame_height_bits_minus_1 + 1

   max_frame_height_minus_1                                     f(n)

   seq_cropping_window_present_flag                             f(1)

   if ( seq_cropping_window_present_flag ) {

       seq_cropping_win_left_offset                            uvlc()

       seq_cropping_win_right_offset                           uvlc()

       seq_cropping_win_top_offset                             uvlc()

       seq_cropping_win_bottom_offset                          uvlc()

   } else {

       seq_cropping_win_left_offset = 0

       seq_cropping_win_right_offset = 0

       seq_cropping_win_top_offset = 0

       seq_cropping_win_bottom_offset = 0

   }

   if ( single_picture_header_flag ) {

       decoder_model_info_present_flag = 0

   } else {

       seq_initial_display_delay_present_flag                   f(1)

       if ( seq_initial_display_delay_present_flag ) {

           seq_initial_display_delay_minus_1                    f(4)

       }

       decoder_model_info_present_flag                          f(1)




AV2 Specification                                             Page 56 of 1169
       if ( decoder_model_info_present_flag ) {

           num_units_in_decoding_tick                                                                    f(32)

           seq_decoder_model_info_present_flag                                                           f(1)

           if ( seq_decoder_model_info_present_flag ) {

               seq_decoder_model_info( )

           }

       }

   }

   for ( mLayer = 0; mLayer < MAX_NUM_MLAYERS; mLayer++ ) {

       for ( currTLayer = 0; currTLayer < MAX_NUM_TLAYERS; currTLayer++ ) {

           for ( refTLayer = 0; refTLayer < MAX_NUM_TLAYERS; refTLayer++ ) {

               TLayerDependencyMap[ mLayer ][ currTLayer ][ refTLayer ] =

                   refTLayer <= currTLayer && currTLayer <= max_tlayer_id && mLayer <= max_mlayer_id

           }

       }

   }

   for ( currLayer = 0; currLayer < MAX_NUM_MLAYERS; currLayer++ ) {

       for ( refLayer = 0; refLayer < MAX_NUM_MLAYERS; refLayer++ ) {

           MLayerDependencyMap[ currLayer ][ refLayer ] =

               refLayer <= currLayer && currLayer <= max_mlayer_id

       }

   }

   if ( max_mlayer_id > 0 ) {

       mlayer_dependency_present_flag                                                                    f(1)

       if ( mlayer_dependency_present_flag ) {

           for ( currLayer = 1; currLayer <= max_mlayer_id; currLayer++ ) {

               for ( refLayer = currLayer; refLayer >= 0; refLayer-- ) {

                   mlayer_dependency_map                                                                 f(1)

                   MLayerDependencyMap[ currLayer ][ refLayer ] =

                    mlayer_dependency_map

               }

           }

       }

   }

   if ( max_tlayer_id > 0 ) {

       tlayer_dependency_present_flag                                                                    f(1)

       if ( tlayer_dependency_present_flag ) {

           if ( max_mlayer_id > 0 )

               multi_tlayer_dependency_map_present_flag                                                  f(1)

           else

               multi_tlayer_dependency_map_present_flag = 0




AV2 Specification                                                                                      Page 57 of 1169
             for ( mLayer = 0; mLayer <= max_mlayer_id; mLayer++ ) {

                 for ( currTLayer = 1; currTLayer <= max_tlayer_id; currTLayer++ ) {

                     for ( refTLayer = currTLayer; refTLayer >= 0; refTLayer-- ) {

                         if (multi_tlayer_dependency_map_present_flag > 0 ||

                              mLayer == 0) {

                             tlayer_dependency_map                                          f(1)

                             TLayerDependencyMap[ mLayer ][ currTLayer ][ refTLayer ] =

                              tlayer_dependency_map

                         } else {

                             TLayerDependencyMap[ mLayer ][ currTLayer ][ refTLayer ] =

                              TLayerDependencyMap[ 0 ][ currTLayer ][ refTLayer ]

                         }

                     }

                 }

             }

         }

     }

     for (mlayerId = 0; mlayerId < MAX_NUM_MLAYERS; mlayerId++) {

         for (refMlayer = 0; refMlayer < MAX_NUM_MLAYERS; refMlayer++) {

             MLayerPresenceMap[mlayerId][refMlayer] = 0

             if ( mlayerId == refMlayer ||

                 MLayerDependencyMap[mlayerId][refMlayer]) {

                 MLayerPresenceMap[mlayerId][refMlayer] = 1

                 for (depMLayerId = 0; depMLayerId < refMlayer; depMLayerId++) {

                     MLayerPresenceMap[mlayerId][depMLayerId] |=

                         MLayerPresenceMap[refMlayer][depMLayerId]

                 }

             }

         }

     }

     sequence_partition_config( )

     sequence_segment_config( )

     sequence_intra_config( )

     sequence_inter_config( )

     sequence_scc_config( )

     sequence_transform_quant_entropy_config( )

     sequence_filter_config( )

     sequence_tile_config( )

     film_grain_params_present                                                              f(1)

     save_sequence_header( )

 }




AV2 Specification                                                                         Page 58 of 1169
```

<a id="s-5-4-2"></a>

#### § 5.4.2 Sequence tile config syntax

```text
§   5.4.2. Sequence tile config syntax

     sequence_tile_config( ) {                                                Descriptor

         seq_tile_info_present_flag                                              f(1)

         if ( seq_tile_info_present_flag ) {

             allow_tile_info_change                                              f(1)

             seqSbSize = get_seq_sb_size()

             ( SeqSbRowStarts, SeqSbRows, SeqTileRows, SeqTileRowsLog2,

             SeqSbColStarts, SeqSbCols, SeqTileCols, SeqTileColsLog2,

             SeqUniformTileSpacingFlag, sbShift) = tile_params(

               max_frame_width_minus_1 + 1, max_frame_height_minus_1 + 1,

               seqSbSize, seqSbSize, 0 )

         }

     }


```

<a id="s-5-4-3"></a>

#### § 5.4.3 Sequence partition config syntax

```text
§   5.4.3. Sequence partition config syntax

     sequence_partition_config( ) {                                           Descriptor

         use_256x256_superblock                                                  f(1)

         if ( !use_256x256_superblock ) {

             use_128x128_superblock                                              f(1)

         }

         if ( Monochrome ) {

             enable_sdp = 0

         } else {

             enable_sdp                                                          f(1)

         }

         if ( enable_sdp && !single_picture_header_flag ) {

             enable_extended_sdp                                                 f(1)

         } else {

             enable_extended_sdp = 0

         }

         enable_ext_partitions                                                   f(1)

         if ( enable_ext_partitions ) {

             enable_uneven_4way_partitions                                       f(1)

         } else {

             enable_uneven_4way_partitions = 0

         }

         reduce_pb_aspect_ratio                                                  f(1)

         if ( reduce_pb_aspect_ratio ) {

             max_pb_aspect_ratio_log2_minus_1                                    f(1)

             MaxPbAspectRatio = 1 << (max_pb_aspect_ratio_log2_minus_1 + 1)

         } else {




    AV2 Specification                                                         Page 59 of 1169
             MaxPbAspectRatio = 8

         }

     }


```

<a id="s-5-4-4"></a>

#### § 5.4.4 Sequence segment config syntax

```text
§   5.4.4. Sequence segment config syntax

     sequence_segment_config( ) {                                              Descriptor

         enable_ext_seg                                                           f(1)

         MaxSegments = enable_ext_seg ? 16 : 8

         seq_seg_info_present_flag                                                f(1)

         if ( seq_seg_info_present_flag ) {

             seq_allow_seg_info_change                                            f(1)

             ( SeqFeatureEnabled, SeqFeatureData ) = seg_info( MaxSegments )

         }

     }


```

<a id="s-5-4-5"></a>

#### § 5.4.5 Sequence intra config syntax

```text
§   5.4.5. Sequence intra config syntax

     sequence_intra_config( ) {                                                Descriptor

         enable_dip                                                               f(1)

         enable_intra_edge_filter                                                 f(1)

         enable_mrls                                                              f(1)

         enable_cfl_intra                                                         f(1)

         if ( Monochrome ) {

             cfl_ds_filter_index = 0

         } else {

             cfl_ds_filter_index                                                  f(2)

         }

         enable_mhccp                                                             f(1)

         enable_ibp                                                               f(1)

     }


```

<a id="s-5-4-6"></a>

#### § 5.4.6 Sequence inter config syntax

```text
§   5.4.6. Sequence inter config syntax

     sequence_inter_config( ) {                                                Descriptor

         if ( single_picture_header_flag ) {

             for ( i = 0; i < MOTION_MODES; i++ ) {

                 seq_enabled_motion_modes[ i ] = 0

             }

             enable_six_param_warp_delta = 0

             enable_masked_compound = 0

             enable_ref_frame_mvs = 0

             reduced_ref_frame_mvs_mode = 0

             OrderHintBits = 0




    AV2 Specification                                                          Page 60 of 1169
     enable_opfl_refine = REFINE_NONE

     enable_refmvbank                                              f(1)

     disable_drl_reorder                                           f(1)

     if ( disable_drl_reorder ) {

         DrlReorder = DRL_REORDER_DISABLED

     } else {

         constrain_drl_reorder                                     f(1)

         DrlReorder = constrain_drl_reorder ?

                DRL_REORDER_CONSTRAINT : DRL_REORDER_ALWAYS

     }

     n = MAX_REF_BV_STACK_SIZE - 1

     seq_max_bvp_drl_bits_minus_1                                  ns(n)

     allow_frame_max_bvp_drl_bits                                  f(1)

     enable_bawp                                                   f(1)

     enable_mv_traj = 0

     enable_imp_msk_bld = 0

     NumRefFrames = 2

     long_term_frame_id_bits = 0

   } else {

     motionModeEnabled = 0

     for ( mode = INTERINTRA; mode < MOTION_MODES; mode++ ) {

         seq_enabled_motion_modes[ mode ]                          f(1)

         motionModeEnabled |= seq_enabled_motion_modes[ mode ]

     }

     if ( motionModeEnabled ) {

         seq_frame_motion_modes_present_flag                       f(1)

     } else {

         seq_frame_motion_modes_present_flag = 0

     }

     if ( seq_enabled_motion_modes[ DELTAWARP ] ) {

         enable_six_param_warp_delta                               f(1)

     } else {

         enable_six_param_warp_delta = 0

     }

     enable_masked_compound                                        f(1)

     enable_ref_frame_mvs                                          f(1)

     if ( enable_ref_frame_mvs ) {

         reduced_ref_frame_mvs_mode                                f(1)

     } else {

         reduced_ref_frame_mvs_mode = 0

     }

     order_hint_bits_minus_1



AV2 Specification                                                Page 61 of 1169
                                                                           f(4)

     OrderHintBits = order_hint_bits_minus_1 + 1

     enable_refmvbank                                                      f(1)

     disable_drl_reorder                                                   f(1)

     if ( disable_drl_reorder ) {

         DrlReorder = DRL_REORDER_DISABLED

     } else {

         constrain_drl_reorder                                             f(1)

         DrlReorder = constrain_drl_reorder ? DRL_REORDER_CONSTRAINT :

                        DRL_REORDER_ALWAYS

     }

     explicit_ref_frame_map                                                f(1)

     explicit_num_ref_frames                                               f(1)

     if ( explicit_num_ref_frames ) {

         num_ref_frames_minus_1                                            f(4)

         NumRefFrames = num_ref_frames_minus_1 + 1

     } else {

         NumRefFrames = 8

     }

     ActiveNumRefFrames = Min( REFS_PER_FRAME, NumRefFrames )

     long_term_frame_id_bits                                               f(3)

     n = MAX_REF_MV_STACK_SIZE - 1

     seq_max_drl_bits_minus_1                                              ns(n)

     allow_frame_max_drl_bits                                              f(1)

     n = MAX_REF_BV_STACK_SIZE - 1

     seq_max_bvp_drl_bits_minus_1                                          ns(n)

     allow_frame_max_bvp_drl_bits                                          f(1)

     num_same_ref_compound                                                 f(2)

     enable_tip                                                            f(1)

     if ( enable_tip ) {

         disable_tip_output                                                f(1)

         EnableTipOutput = !disable_tip_output

         enable_tip_hole_fill                                              f(1)

     } else {

         enable_tip_hole_fill = 0

         EnableTipOutput = 0

     }

     enable_mv_traj                                                        f(1)

     enable_bawp                                                           f(1)

     enable_cwp                                                            f(1)

     enable_imp_msk_bld                                                    f(1)

     enable_df_sub_pu                                                      f(1)



AV2 Specification                                                        Page 62 of 1169
             if ( EnableTipOutput && enable_df_sub_pu ) {

                 enable_tip_explicit_qp                                                 f(1)

             } else {

                 enable_tip_explicit_qp = 0

             }

             enable_opfl_refine                                                         f(2)

             enable_refinemv                                                            f(1)

             if ( enable_tip && ( enable_opfl_refine != 0 || enable_refinemv ) ) {

                 enable_tip_refinemv                                                    f(1)

             } else {

                 enable_tip_refinemv = 0

             }

             enable_bru                                                                 f(1)

             enable_adaptive_mvd                                                        f(1)

             enable_mvd_sign_derive                                                     f(1)

             enable_flex_mvres                                                          f(1)

             if ( single_picture_header_flag ) {

                 enable_global_motion = 0

             } else {

                 enable_global_motion                                                   f(1)

             }

             enable_short_refresh_frame_flags                                           f(1)

         }

     }


```

<a id="s-5-4-7"></a>

#### § 5.4.7 Sequence screen content config syntax

```text
§   5.4.7. Sequence screen content config syntax

     sequence_scc_config( ) {                                                        Descriptor

         if ( single_picture_header_flag ) {

             seq_force_screen_content_tools = SELECT_SCREEN_CONTENT_TOOLS

             seq_force_integer_mv = SELECT_INTEGER_MV

         } else {

             seq_choose_screen_content_tools                                            f(1)

             if ( seq_choose_screen_content_tools ) {

                 seq_force_screen_content_tools = SELECT_SCREEN_CONTENT_TOOLS

             } else {

                 seq_force_screen_content_tools                                         f(1)

             }

             if ( seq_force_screen_content_tools > 0 ) {

                 seq_choose_integer_mv                                                  f(1)

                 if ( seq_choose_integer_mv ) {

                  seq_force_integer_mv = SELECT_INTEGER_MV




    AV2 Specification                                                                Page 63 of 1169
                 } else {

                     seq_force_integer_mv                        f(1)

                 }

             } else {

                 seq_force_integer_mv = SELECT_INTEGER_MV

             }

         }

     }


```

<a id="s-5-4-8"></a>

#### § 5.4.8 Sequence transform quant entropy config syntax

```text
§   5.4.8. Sequence transform quant entropy config syntax

     sequence_transform_quant_entropy_config( ) {             Descriptor

         enable_fsc                                              f(1)

         if ( enable_fsc ) {

             enable_idtx_intra = 1

         } else {

             enable_idtx_intra                                   f(1)

         }

         enable_intra_ist                                        f(1)

         enable_inter_ist                                        f(1)

         if ( Monochrome ) {

             enable_chroma_dctonly = 0

         } else {

             enable_chroma_dctonly                               f(1)

         }

         if ( !single_picture_header_flag ) {

             enable_inter_ddt                                    f(1)

         }

         reduced_tx_part_set                                     f(1)

         if ( Monochrome ) {

             enable_cctx = 0

         } else {

             enable_cctx                                         f(1)

         }

         enable_tcq                                              f(1)

         if ( enable_tcq && !single_picture_header_flag ) {

             choose_tcq_per_frame                                f(1)

         } else {

             choose_tcq_per_frame = 0

         }

         if ( enable_tcq && !choose_tcq_per_frame ) {

             enable_parity_hiding = 0




    AV2 Specification                                         Page 64 of 1169
   } else {

       enable_parity_hiding                                            f(1)

   }

   if ( single_picture_header_flag ) {

       enable_avg_cdf = 1

       avg_cdf_type = 1

   } else {

       enable_avg_cdf                                                  f(1)

       if ( enable_avg_cdf ) {

           avg_cdf_type                                                f(1)

       }

   }

   if ( Monochrome ) {

       separate_uv_delta_q = 0

   } else {

       separate_uv_delta_q                                             f(1)

   }

   BaseYDcDeltaQ = 0

   BaseUVDcDeltaQ = 0

   BaseUVAcDeltaQ = 0

   y_dc_delta_q_enabled = 0

   uv_dc_delta_q_enabled = 0

   uv_ac_delta_q_enabled = 0

   equal_ac_dc_q                                                       f(1)

   if ( !equal_ac_dc_q ) {

       base_y_dc_delta_q                                               f(5)

       BaseYDcDeltaQ = DELTA_DCQUANT_MIN + base_y_dc_delta_q

       y_dc_delta_q_enabled                                            f(1)

   }

   if ( !Monochrome ) {

       if ( !equal_ac_dc_q ) {

           base_uv_dc_delta_q                                          f(5)

           BaseUVDcDeltaQ = DELTA_DCQUANT_MIN + base_uv_dc_delta_q

           uv_dc_delta_q_enabled                                       f(1)

       }

       base_uv_ac_delta_q                                              f(5)

       BaseUVAcDeltaQ = DELTA_DCQUANT_MIN + base_uv_ac_delta_q

       uv_ac_delta_q_enabled                                           f(1)

       if ( equal_ac_dc_q ) {

           BaseUVDcDeltaQ = BaseUVAcDeltaQ

       }




AV2 Specification                                                    Page 65 of 1169
         }

     }


```

<a id="s-5-4-9"></a>

#### § 5.4.9 Segment information syntax

```text
§   5.4.9. Segment information syntax

     seg_info( numSegments ) {                                                  Descriptor

         for ( i = 0; i < numSegments; i++ ) {

             for ( j = 0; j < SEG_LVL_MAX; j++ ) {

                 feature_enabled                                                   f(1)

                 enabled[ i ][ j ] = feature_enabled

                 clippedValue = 0

                 if ( feature_enabled == 1 ) {

                     bitsToRead = Segmentation_Feature_Bits[ j ]

                     limit = Segmentation_Feature_Max[ j ]

                     if ( Segmentation_Feature_Signed[ j ] == 1 ) {

                         n = 1 + bitsToRead

                         feature_value                                             su(n)

                         clippedValue = Clip3( -limit, limit, feature_value)

                     } else {

                         feature_value                                         f(bitsToRead)

                         clippedValue = Clip3( 0, limit, feature_value)

                     }

                 }

                 data[ i ][ j ] = clippedValue

             }

         }

         return (enabled, data)

     }


```

<a id="s-5-4-10"></a>

#### § 5.4.10 Sequence filter config syntax

```text
§   5.4.10. Sequence filter config syntax

     sequence_filter_config( ) {                                                Descriptor

         disable_loopfilters_across_tiles                                          f(1)

         enable_cdef                                                               f(1)

         enable_gdf                                                                f(1)

         if ( enable_gdf && get_seq_sb_size() == BLOCK_64X64 ) {

             gdf_unit_matches_sb_size                                              f(1)

         } else {

             gdf_unit_matches_sb_size = 0

         }

         enable_restoration                                                        f(1)

         if ( enable_restoration ) {

             lr_tools_disable[ 0 ][ RESTORE_PC_WIENER ]                            f(1)




    AV2 Specification                                                           Page 66 of 1169
             lr_tools_disable[ 0 ][ RESTORE_WIENER_NONSEP ]                    f(1)

             lr_tools_disable[ 1 ][ RESTORE_PC_WIENER ] = 1

             lr_tools_uv_present                                               f(1)

             if ( lr_tools_uv_present ) {

                 lr_tools_disable[ 1 ][ RESTORE_WIENER_NONSEP ]                f(1)

             } else {

                 lr_tools_disable[ 1 ][ RESTORE_WIENER_NONSEP ] =

                  lr_tools_disable[ 0 ][ RESTORE_WIENER_NONSEP ]

             }

         }

         enable_ccso                                                           f(1)

         if ( enable_ccso ) {

             ccso_unit_matches_sb_size                                         f(1)

         } else {

             ccso_unit_matches_sb_size = 0

         }

         if ( single_picture_header_flag ) {

             CdefOnSkipTxfm = CDEF_ON_SKIP_TXFM_ADAPTIVE

         } else {

             cdef_on_skip_txfm_always_on                                       f(1)

             if (cdef_on_skip_txfm_always_on) {

                 CdefOnSkipTxfm = CDEF_ON_SKIP_TXFM_ALWAYS_ON

             } else {

                 cdef_on_skip_txfm_disabled                                    f(1)

                 CdefOnSkipTxfm = cdef_on_skip_txfm_disabled ?

                  CDEF_ON_SKIP_TXFM_DISABLED : CDEF_ON_SKIP_TXFM_ADAPTIVE

             }

         }

         df_par_bits_minus_2                                                   f(2)

     }


```

<a id="s-5-4-11"></a>

#### § 5.4.11 User defined QM syntax

```text
§   5.4.11. User defined QM syntax

     user_defined_qm( level, t, plane ) {                                   Descriptor

         txSz = Fundamental_Tx_Size[ t ]

         w = Tx_Width[ txSz ]

         h = Tx_Height[ txSz ]

         if ( plane > 0 ) {

             qm_copy_from_previous_plane                                       f(1)

             if ( qm_copy_from_previous_plane ) {

                 for ( i = 0; i < h; i++ ) {

                  for ( j = 0; j < w; j++ ) {




    AV2 Specification                                                       Page 67 of 1169
                   UserQm[ level ][ t ][ plane ][ i ][ j ] =

                    UserQm[ level ][ t ][ plane - 1 ][ i ][ j ]

               }

           }

           return

       }

   }

   if ( t == 0 ) {

       qm_8x8_is_symmetric                                          f(1)

   } else if ( t == 2 ) {

       qm_4x8_is_transpose_of_8x4                                   f(1)

       if ( qm_4x8_is_transpose_of_8x4 ) {

           for ( i = 0; i < h; i++ ) {

               for ( j = 0; j < w; j++ ) {

                   UserQm[ level ][ t ][ plane ][ i ][ j ] =

                    UserQm[ level ][ 1 ][ plane ][ j ][ i ]

               }

           }

           return

       }

   }

   scan = get_scan( txSz, TX_CLASS_2D )

   quant = 32

   coefRepeat = 0

   for ( c = 0; c < w * h; c++ ) {

       pos = scan[ c ]

       (row, col) = get_tx_row_col(pos, txSz)

       if ( t == 0 && qm_8x8_is_symmetric && col > row ) {

           quant = UserQm[ level ][ t ][ plane ][ col ][ row ]

           UserQm[ level ][ t ][ plane ][ row ][ col ] = quant

       } else if ( coefRepeat ) {

           UserQm[ level ][ t ][ plane ][ row ][ col ] = quant

       } else {

           quant_delta                                             svlc()

           quant2 = (quant + quant_delta) & 255

           if ( quant2 == 0 ) {

               coefRepeat = 1

           } else {

               quant = quant2

           }

           UserQm[ level ][ t ][ plane ][ row ][ col ] = quant




AV2 Specification                                                 Page 68 of 1169
             }

         }

     }


    where Fundamental_Tx_Size (which gives the order of quantization matrices) is specified as:

     Fundamental_Tx_Size[ 3 ] = { TX_8X8, TX_8X4, TX_4X8 }


```

<a id="s-5-4-12"></a>

#### § 5.4.12 Timing info syntax

```text
§   5.4.12. Timing info syntax

     timing_info( ) {                                                                             Descriptor

         num_units_in_display_tick                                                                  f(32)

         time_scale                                                                                 f(32)

         equal_picture_interval                                                                      f(1)

         if ( equal_picture_interval ) {

             num_ticks_per_picture_minus_1                                                          uvlc()

         }

     }


```

<a id="s-5-4-13"></a>

#### § 5.4.13 Sequence decoder model info syntax

```text
§   5.4.13. Sequence decoder model info syntax

     seq_decoder_model_info( ) {                                                                  Descriptor

         decoder_buffer_delay                                                                       uvlc()

         encoder_buffer_delay                                                                       uvlc()

         low_delay_mode_flag                                                                         f(1)

     }


```

<a id="s-5-5"></a>

### § 5.5 Temporal delimiter OBU syntax

```text
§   5.5. Temporal delimiter OBU syntax
     temporal_delimiter_obu( ) {                                                                  Descriptor

         SeenFrameHeader = 0

         for ( level = 0; level < 15; level++ ) {

             QmProtected[ level ] = 0

         }

     }



         NOTE:      The temporal delimiter has an empty payload.


```

<a id="s-5-6"></a>

### § 5.6 Multi Stream Decoder Operation OBU syntax

```text
§   5.6. Multi Stream Decoder Operation OBU syntax
     multistream_decoder_operation_obu( ) {                                                       Descriptor

         num_streams_minus_2                                                                         f(3)

         multistream_profile_idc                                                                     f(5)

         multistream_level_idx                                                                       f(5)




    AV2 Specification                                                                             Page 69 of 1169
         multistream_tier                                       f(1)

         multistream_even_allocation_flag                       f(1)

         if ( !multistream_even_allocation_flag ) {

             multistream_large_picture_idc                      f(3)

         }

         for ( i = 0; i < num_streams_minus_2 + 2; i++ ) {

             sub_xlayer_id[ i ]                                 f(5)

             sub_stream_max_profile[ i ]                        f(5)

             sub_stream_max_level[ i ]                          f(5)

             sub_stream_max_tier[ i ]                           f(1)

         }

         multistream_doh_constraint_flag                        f(1)

     }


```

<a id="s-5-7"></a>

### § 5.7 Multi frame header OBU syntax

```text
§   5.7. Multi frame header OBU syntax
     multi_frame_header_obu( ) {                             Descriptor

         mfh_seq_header_id                                     uvlc()

         mfh_id_minus_1                                        uvlc()

         mfhId = mfh_id_minus_1 + 1

         MfhSeqHeaderId[ mfhId ] = mfh_seq_header_id

         MfhTLayerId[ mfhId ] = obu_tlayer_id

         MfhMLayerId[ mfhId ] = obu_mlayer_id

         mfh_frame_size_present_flag[ mfhId ]                   f(1)

         if ( mfh_frame_size_present_flag[ mfhId ] ) {

             mfh_frame_width_bits_minus_1                       f(4)

             mfh_frame_height_bits_minus_1                      f(4)

             n = mfh_frame_width_bits_minus_1 + 1

             mfh_frame_width_minus_1[ mfhId ]                   f(n)

             n = mfh_frame_height_bits_minus_1 + 1

             mfh_frame_height_minus_1[ mfhId ]                  f(n)

         }

         mfh_deblocking_filter_update[ mfhId ]                  f(1)

         if ( mfh_deblocking_filter_update[ mfhId ] ) {

             for ( i = 0; i < 4; i++ ) {

                 mfh_apply_deblocking_filter[ mfhId ][ i ]      f(1)

             }

         }

         mfh_seg_info_present_flag[ mfhId ]                     f(1)

         if ( mfh_seg_info_present_flag[ mfhId ] ) {

             mfh_ext_seg_flag[ mfhId ]                          f(1)

             mfh_allow_seg_info_change[ mfhId ]                 f(1)




    AV2 Specification                                        Page 70 of 1169
             ( MfhFeatureEnabled[mfhId], MfhFeatureData[mfhId] ) =

                 seg_info( mfh_ext_seg_flag[ mfhId ] ? 16 : 8 )

         }

     }


```

<a id="s-5-8"></a>

### § 5.8 Layer config record OBU syntax

```text
§   5.8. Layer config record OBU syntax
     layer_config_record_obu() {                                     Descriptor

         if ( obu_xlayer_id == GLOBAL_XLAYER_ID ) {

             lcr_global_info( )

         } else {

             lcr_local_info( obu_xlayer_id )

         }

     }


```

<a id="s-5-8-1"></a>

#### § 5.8.1 LCR global info syntax

```text
§   5.8.1. LCR global info syntax

     lcr_global_info( ) {                                            Descriptor

         lcr_global_config_record_id                                    f(3)

         lcr_xlayer_map                                                f(31)

         LcrMaxNumXLayerCount = 0

         for ( i = 0; i < 31; i++ ) {

             if ( lcr_xlayer_map & ( 1     <<   i ) ) {

                 LcrXLayerID[ LcrMaxNumXLayerCount ] = i

                 LcrMaxNumXLayerCount ++

             }

         }

         lcr_aggregate_info_present_flag                                f(1)

         lcr_seq_profile_tier_level_info_present_flag                   f(1)

         lcr_global_payload_present_flag                                f(1)

         lcr_dependent_xlayers_flag                                     f(1)

         lcr_global_atlas_id_present_flag                               f(1)

         lcr_global_purpose_id                                          f(7)

         lcr_doh_constraint_flag                                        f(1)

         lcr_enforce_tile_alignment_flag                                f(1)

         if ( lcr_global_atlas_id_present_flag ) {

             lcr_global_atlas_id                                        f(3)

         } else {

             lcr_global_reserved_zero_3bits                             f(3)

         }

         lcr_global_reserved_zero_5bits                                 f(5)

         if ( lcr_aggregate_info_present_flag ) {

             lcr_aggregate_info( )




    AV2 Specification                                                Page 71 of 1169
         }

         if ( lcr_seq_profile_tier_level_info_present_flag ) {

             for ( i = 0; i < LcrMaxNumXLayerCount; i++ ) {

                 lcr_seq_profile_tier_level_info( LcrXLayerID[ i ] )

             }

         }

         if ( lcr_global_payload_present_flag ) {

             for ( i = 0; i <    LcrMaxNumXLayerCount; i++) {

                 lcr_data_size [ i ]                                            leb128()

                 lcr_global_payload( LcrXLayerID[ i ], lcr_data_size [ i ] )

             }

         }

     }


```

<a id="s-5-8-2"></a>

#### § 5.8.2 LCR local info syntax

```text
§   5.8.2. LCR local info syntax

     lcr_local_info( xlayerId ) {                                              Descriptor

         lcr_global_id[ xlayerId ]                                                f(3)

         lcr_local_id[ xlayerId ]                                                 f(3)

         lcr_profile_tier_level_info_present_flag[ xlayerId ]                     f(1)

         lcr_local_atlas_id_present_flag[ xlayerId ]                              f(1)

         if ( lcr_profile_tier_level_info_present_flag[ xlayerId ] ) {

             lcr_seq_profile_tier_level_info( xlayerId )

         }

         if ( lcr_local_atlas_id_present_flag[ xlayerId ] ) {

             lcr_local_atlas_id[ xlayerId ]                                       f(3)

         } else {

             lcr_local_reserved_zero_3bits[ xlayerId ]                            f(3)

         }

         lcr_local_reserved_zero_5bits[ xlayerId ]                                f(5)

         lcr_xlayer_info( 0, xlayerId )

     }


```

<a id="s-5-8-3"></a>

#### § 5.8.3 LCR aggregate info syntax

```text
§   5.8.3. LCR aggregate info syntax

     lcr_aggregate_info( ) {                                                   Descriptor

         lcr_config_idc                                                           f(6)

         lcr_aggregate_level_idx                                                  f(5)

         lcr_max_tier_flag                                                        f(1)

         lcr_max_interop                                                          f(4)

     }




    AV2 Specification                                                          Page 72 of 1169
```

<a id="s-5-8-4"></a>

#### § 5.8.4 LCR sequence profile tier level information syntax

```text
§   5.8.4. LCR sequence profile tier level information syntax

     lcr_seq_profile_tier_level_info( i ) {                                 Descriptor

         lcr_seq_profile_idc[ i ]                                              f(5)

         lcr_max_level_idx[ i ]                                                f(5)

         lcr_tier_flag[ i ]                                                    f(1)

         lcr_max_mlayer_count[ i ]                                             f(3)

         lsptli_reserved_2bits                                                 f(2)

     }


```

<a id="s-5-8-5"></a>

#### § 5.8.5 LCR global payload syntax

```text
§   5.8.5. LCR global payload syntax

     lcr_global_payload( n, sz ) {                                          Descriptor

         startPosition = get_position( )

         if ( lcr_dependent_xlayers_flag && n > 0 ) {

             lcr_num_dependent_xlayer_map[ n ]                                 f(n)

         }

         lcr_xlayer_info( 1 , n )

         currentPosition = get_position( )

         parsedPayloadBits = currentPosition - startPosition

         RemainingLcrPayloadBits = sz * 8 - parsedPayloadBits

         for ( j = 0; j < RemainingLcrPayloadBits; j++ ) {

             lcr_remaining_payload_bit                                         f(1)

         }

     }


```

<a id="s-5-8-6"></a>

#### § 5.8.6 LCR xlayer info syntax

```text
§   5.8.6. LCR xlayer info syntax

     lcr_xlayer_info( isGlobal, xId ) {                                     Descriptor

         lcr_rep_info_present_flag[ isGlobal ][ xId ]                          f(1)

         lcr_xlayer_purpose_present_flag[ isGlobal ][ xId ]                    f(1)

         lcr_xlayer_color_info_present_flag[ isGlobal ][ xId ]                 f(1)

         lcr_embedded_layer_info_present_flag[ isGlobal ][ xId ]               f(1)

         if ( lcr_rep_info_present_flag[ isGlobal ][ xId ] ) {

             lcr_rep_info( isGlobal, xId )

         }

         if( lcr_xlayer_purpose_present_flag[ isGlobal ][ xId ] ) {

             lcr_xlayer_purpose_id[ isGlobal ][ xId ]                          f(7)

         }

         if( lcr_xlayer_color_info_present_flag[ isGlobal ][ xId ] ) {

             lcr_xlayer_color_info( isGlobal, xId )

         }

         byte_alignment()

         if ( lcr_embedded_layer_info_present_flag[ isGlobal ][ xId ] ) {



    AV2 Specification                                                       Page 73 of 1169
             lcr_embedded_layer_info( isGlobal, xId )

         } else {

             if ( isGlobal && lcr_global_atlas_id_present_flag ) {

                 lcr_xlayer_atlas_segment_id[ xId ]                        f(8)

                 lcr_xlayer_priority_order[ xId ]                          f(8)

                 lcr_xlayer_rendering_method[ xId ]                        f(8)

             }

         }

     }


```

<a id="s-5-8-7"></a>

#### § 5.8.7 LCR rep info syntax

```text
§   5.8.7. LCR rep info syntax

     lcr_rep_info( isGlobal, xId ) {                                    Descriptor

         lcr_max_pic_width[ isGlobal ][ xId ]                             uvlc()

         lcr_max_pic_height[ isGlobal ][ xId ]                            uvlc()

         lcr_format_info_present_flag[ isGlobal ][ xId ]                   f(1)

         lcr_cropping_window_present_flag[ isGlobal ][ xId ]               f(1)

         if ( lcr_format_info_present_flag[ isGlobal ][ xId ] ) {

             lcr_bit_depth_idc[ isGlobal ][ xId ]                         uvlc()

             lcr_chroma_format_idc[ isGlobal ][ xId ]                     uvlc()

         }

         if ( lcr_cropping_window_present_flag[ isGlobal ][ xId ] ) {

             lcr_cropping_win_left_offset [ isGlobal ][ xId ]             uvlc()

             lcr_cropping_win_right_offset[ isGlobal ][ xId ]             uvlc()

             lcr_cropping_win_top_offset [ isGlobal ][ xId ]              uvlc()

             lcr_cropping_win_bottom_offset[ isGlobal ][ xId ]            uvlc()

         }

     }


```

<a id="s-5-8-8"></a>

#### § 5.8.8 LCR embedded layer info syntax

```text
§   5.8.8. LCR embedded layer info syntax

     lcr_embedded_layer_info( isGlobal, xId ) {                         Descriptor

         lcr_mlayer_map[ isGlobal ][ xId ]                                 f(8)

         for ( j = 0; j < 8; j++ ) {

             if ( lcr_mlayer_map[ isGlobal ][ xId ] & (1 << j) ) {

                 n = MAX_NUM_TLAYERS

                 lcr_tlayer_map[ isGlobal ][ xId ][ j ]                    f(n)

                 atlasSegmentPresent = isGlobal ?

                  lcr_global_atlas_id_present_flag :

                  lcr_local_atlas_id_present_flag[ xId ]

                 if ( atlasSegmentPresent ) {

                  lcr_layer_atlas_segment_id[ isGlobal ][ xId ][ j ]       f(8)

                  lcr_priority_order[ isGlobal ][ xId ][ j ]               f(8)




    AV2 Specification                                                   Page 74 of 1169
                     lcr_rendering_method[ isGlobal ][ xId ][ j ]                       f(8)

                 }

                 lcr_layer_type[ isGlobal ][ xId ][ j ]                                 f(8)

                 if ( lcr_layer_type[ isGlobal ][ xId ][ j ] == AUX_LAYER ) {

                     lcr_auxiliary_type[ isGlobal ][ xId ][ j ]                         f(8)

                 }

                 lcr_view_type[ isGlobal ][ xId ][ j ]                                  f(8)

                 if ( lcr_view_type[ isGlobal ][ xId ][ j ] == VIEW_EXPLICIT ) {

                     lcr_view_id[ isGlobal ][ xId ][ j ]                                f(8)

                 }

                 if ( j > 0 ) {

                     lcr_dependent_layer_map[ isGlobal ][ xId ][ j ]                    f(j)

                 }

                 lcr_same_sh_max_resolution_flag[ isGlobal ][ xId ][ j ]                f(1)

                 if ( !lcr_same_sh_max_resolution_flag[ isGlobal ][ xId ][ j ] ) {

                     lcr_max_expected_width[ isGlobal ][ xId ][ j ]                    uvlc()

                     lcr_max_expected_height[ isGlobal ][ xId ][ j ]                   uvlc()

                 }

                 byte_alignment( )

             }

         }

     }


```

<a id="s-5-8-9"></a>

#### § 5.8.9 LCR xlayer color info syntax

```text
§   5.8.9. LCR xlayer color info syntax

     lcr_xlayer_color_info( isGlobal, xId ) {                                        Descriptor

         layer_color_description_idc[ isGlobal ][ xId ]                                rg(2)

         if ( layer_color_description_idc[ isGlobal ][ xId ] == 0 ) {

             layer_color_primaries[ isGlobal ][ xId ]                                   f(8)

             layer_transfer_characteristics[ isGlobal ][ xId ]                          f(8)

             layer_matrix_coefficients[ isGlobal ][ xId ]                               f(8)

         }

         layer_full_range_flag[ isGlobal ][ xId ]                                       f(1)

     }


```

<a id="s-5-9"></a>

### § 5.9 Atlas segment info OBU syntax

```text
§   5.9. Atlas segment info OBU syntax
     atlas_segment_info_obu( ) {                                                     Descriptor

         atlas_segment_id[ obu_xlayer_id ]                                              f(3)

         xAId = atlas_segment_id[ obu_xlayer_id ]

         ats_atlas_segment_mode_idc[ xAId ]                                            uvlc()

         if ( ats_atlas_segment_mode_idc[ xAId ] == ENHANCED_ATLAS ) {

             numSegments = ats_enhanced_atlas_info( xAId )




    AV2 Specification                                                                Page 75 of 1169
         } else if ( ats_atlas_segment_mode_idc[ xAId ] == BASIC_ATLAS ) {

             numSegments = ats_basic_info( xAId )

         } else if ( ats_atlas_segment_mode_idc[ xAId ] == SINGLE_ATLAS ) {

             numSegments = 1

             ats_nominal_width_minus_1[ xAId ]                                        uvlc()

             ats_nominal_height_minus_1[ xAId ]                                       uvlc()

         } else if ( ats_atlas_segment_mode_idc[ xAId ] == MULTISTREAM_ATLAS ) {

             numSegments = ats_multistream_info( obu_xlayer_id, xAId )

         } else if ( ats_atlas_segment_mode_idc[ xAId ] ==

                  MULTISTREAM_ALPHA_ATLAS ) {

             numSegments = ats_multistream_with_alpha_info( obu_xlayer_id, xAId )

         }

         ats_label_segment_info( obu_xlayer_id, xAId, numSegments )

     }


```

<a id="s-5-9-1"></a>

#### § 5.9.1 Atlas label segment info syntax

```text
§   5.9.1. Atlas label segment info syntax

     ats_label_segment_info( xlayerId, xAId, numSegments ) {                        Descriptor

         ats_signaled_atlas_segment_ids_flag[ xlayerId ][ xAId ]                       f(1)

         if ( ats_signaled_atlas_segment_ids_flag[ xlayerId ][ xAId ] ) {

             for ( i = 0;i < numSegments; i++ ) {

                 ats_atlas_segment_id[ xlayerId ][ xAId ][ i ]                         f(8)

                 AtlasSegmentIDToIndex[ xlayerId ][ xAId ]

                  [ ats_atlas_segment_id[ xlayerId ][ xAId ][ i ] ] = i

                 AtlasSegmentIndexToID[ xlayerId ][ xAId ][ i ] =

                  ats_atlas_segment_id[ xlayerId ][ xAId ][ i ]

             }

         } else {

             for ( i = 0;i < numSegments; i++ ) {

                 ats_atlas_segment_id[ xlayerId ][ xAId ][ i ] = i

                 AtlasSegmentIDToIndex[ xlayerId ][ xAId ][ i ] = i

                 AtlasSegmentIndexToID[ xlayerId ][ xAId ][ i ] = i

             }

         }

     }


```

<a id="s-5-9-2"></a>

#### § 5.9.2 Atlas enhanced atlas info syntax

```text
§   5.9.2. Atlas enhanced atlas info syntax

     ats_enhanced_atlas_info( xAId ) {                                              Descriptor

         ats_region_info( xAId )

         numSegments = ats_region_to_segment_mapping( xAId )

         return numSegments

     }




    AV2 Specification                                                               Page 76 of 1169
```

<a id="s-5-9-2-1"></a>

##### § 5.9.2.1 Atlas region info syntax

```text
§   5.9.2.1. Atlas region info syntax

     ats_region_info( xAId ) {                                                   Descriptor

         ats_num_region_columns_minus_1[ xAId ]                                    uvlc()

         ats_num_region_rows_minus_1[ xAId ]                                       uvlc()

         ats_uniform_spacing_flag[ xAId ]                                           f(1)

         AtlasWidth = 0

         AtlasHeight = 0

         if ( !ats_uniform_spacing_flag[ xAId ] ) {

             for ( i = 0; i < ats_num_region_columns_minus_1[ xAId ] + 1;

                 i++ ) {

                 ats_column_width_minus_1[ xAId ][ i ]                             uvlc()

                 AtlasWidth += (ats_column_width_minus_1[ xAId ][ i ] + 1)

             }

             for ( i = 0;i < ats_num_region_rows_minus_1[xAId] + 1; i++ ) {

                 ats_row_height_minus_1[ xAId ][ i ]                               uvlc()

                 AtlasHeight += (ats_row_height_minus_1[ xAId ][ i ] + 1)

             }

         } else {

             ats_region_width_minus_1[ xAId ]                                      uvlc()

             ats_region_height_minus_1[ xAId ]                                     uvlc()

             AtlasWidth =

                 ( ats_region_width_minus_1[ xAId ] + 1 ) *

                 ( ats_num_region_columns_minus_1[ xAId ] + 1 )

             AtlasHeight =

                 ( ats_region_height_minus_1[ xAId ] + 1 ) *

                 ( ats_num_region_rows_minus_1[ xAId ] + 1 )

         }

         NumRegionsInAtlas[ xAId ] =

             ( ats_num_region_columns_minus_1[ xAId ] + 1) *

             ( ats_num_region_rows_minus_1[ xAId ] + 1 )

     }


```

<a id="s-5-9-2-2"></a>

##### § 5.9.2.2 Atlas region to segment mapping syntax

```text
§   5.9.2.2. Atlas region to segment mapping syntax

     ats_region_to_segment_mapping( xAId ) {                                     Descriptor

         ats_single_region_per_atlas_segment_flag[ xAId ]                           f(1)

         if ( !ats_single_region_per_atlas_segment_flag[ xAId ] ) {

             ats_num_atlas_segments_minus_1[ xAId ]                                uvlc()

             for ( i = 0; i <= ats_num_atlas_segments_minus_1[ xAId ]; i++ ) {

                 ats_top_left_region_column[ xAId ][ i ]                           uvlc()

                 ats_top_left_region_row[ xAId ][ i ]                              uvlc()

                 ats_bottom_right_region_column_off[ xAId ][ i ]                   uvlc()




    AV2 Specification                                                            Page 77 of 1169
                 ats_bottom_right_region_row_off[ xAId ][ i ]                          uvlc()

             }

         } else {

             ats_num_atlas_segments_minus_1[ xAId ] =

                 NumRegionsInAtlas[ xAId ] - 1

         }

         return ats_num_atlas_segments_minus_1[ xAId ] + 1

     }


```

<a id="s-5-9-3"></a>

#### § 5.9.3 Atlas multistream info syntax

```text
§   5.9.3. Atlas multistream info syntax

     ats_multistream_info( xlayerId, xAId ) {                                        Descriptor

         ats_msi_width[ xlayerId ][ xAId ]                                             uvlc()

         ats_msi_height[ xlayerId ][ xAId ]                                            uvlc()

         AtlasWidth = ats_msi_width[ xlayerId ][ xAId ]

         AtlasHeight = ats_msi_height[ xlayerId ][ xAId ]

         ats_msi_num_atlas_segments_minus_1[ xlayerId ][ xAId ]                        uvlc()

         ats_msi_background_info_present_flag[ xlayerId ][ xAId ]                       f(1)

         if ( ats_msi_background_info_present_flag[ xlayerId ][ xAId ] ) {

             ats_msi_background_red_value[ xlayerId ][ xAId ]                           f(8)

             ats_msi_background_green_value[ xlayerId ][ xAId ]                         f(8)

             ats_msi_background_blue_value[ xlayerId ][ xAId ]                          f(8)

         }

         for (i=0;i<=ats_msi_num_atlas_segments_minus_1[ xlayerId ][ xAId ];i++) {

             ats_msi_input_stream_id[ xlayerId ][ xAId ][ i ]                           f(5)

             ats_msi_segment_top_left_pos_x[ xlayerId ][ xAId ][ i ]                   uvlc()

             ats_msi_segment_top_left_pos_y[ xlayerId ][ xAId ][ i ]                   uvlc()

             ats_msi_segment_width[ xlayerId ][ xAId ][ i ]                            uvlc()

             ats_msi_segment_height[ xlayerId ][ xAId ][ i ]                           uvlc()

         }

         return ats_msi_num_atlas_segments_minus_1[ xlayerId ][ xAId ] + 1

     }


```

<a id="s-5-9-4"></a>

#### § 5.9.4 Atlas multistream with alpha info syntax

```text
§   5.9.4. Atlas multistream with alpha info syntax

     ats_multistream_with_alpha_info( xlayerId, xAId ) {                             Descriptor

         ats_msi_width[ xlayerId ][ xAId ]                                             uvlc()

         ats_msi_height[ xlayerId ][ xAId ]                                            uvlc()

         AtlasWidth = ats_msi_width[ xlayerId ][ xAId ]

         AtlasHeight = ats_msi_height[ xlayerId ][ xAId ]

         ats_msi_num_atlas_segments_minus_1[ xlayerId ][ xAId ]                        uvlc()

         ats_msi_alpha_segments_present_flag[ xlayerId ][ xAId ]                        f(1)

         ats_msi_background_info_present_flag[ xlayerId ][ xAId ]                       f(1)




    AV2 Specification                                                                Page 78 of 1169
         if ( ats_msi_background_info_present_flag[ xlayerId ][ xAId ] ) {

             ats_msi_background_red_value[ xlayerId ][ xAId ]                           f(8)

             ats_msi_background_green_value[ xlayerId ][ xAId ]                         f(8)

             ats_msi_background_blue_value[ xlayerId ][ xAId ]                          f(8)

         }

         for (i=0;i<=ats_msi_num_atlas_segments_minus_1[ xlayerId ][ xAId ];i++) {

             ats_msi_input_stream_id[ xlayerId ][ xAId ][ i ]                           f(5)

             ats_msi_segment_top_left_pos_x[ xlayerId ][ xAId ][ i ]                   uvlc()

             ats_msi_segment_top_left_pos_y[ xlayerId ][ xAId ][ i ]                   uvlc()

             ats_msi_segment_width[ xlayerId ][ xAId ][ i ]                            uvlc()

             ats_msi_segment_height[ xlayerId ][ xAId ][ i ]                           uvlc()

             if ( ats_msi_alpha_segments_present_flag[ xlayerId ][ xAId ] &&

                 i != ats_msi_num_atlas_segments_minus_1[ xlayerId ][ xAId ] ) {

                 ats_msi_alpha_segment_flag[ xlayerId ][ xAId ][ i ]                    f(1)

             } else {

                 ats_msi_alpha_segment_flag[ xlayerId ][ xAId ][ i ] = 0

             }

         }

         return ats_msi_num_atlas_segments_minus_1[ xlayerId ][ xAId ] + 1

     }


```

<a id="s-5-9-5"></a>

#### § 5.9.5 Atlas basic info syntax

```text
§   5.9.5. Atlas basic info syntax

     ats_basic_info( xAId ) {                                                        Descriptor

         ats_stream_id_present[ xAId ]                                                  f(1)

         ats_width[ xAId ]                                                             uvlc()

         ats_height[ xAId ]                                                            uvlc()

         ats_num_atlas_segments_minus_1[ xAId ]                                        uvlc()

         AtlasWidth = ats_width[ xAId ]

         AtlasHeight = ats_height[ xAId ]

         for ( i = 0; i <= ats_num_atlas_segments_minus_1[ xAId ]; i++ ) {

             if (ats_stream_id_present[ xAId ]) {

                 ats_input_stream_id[ xAId ][ i ]                                       f(5)

             }

             ats_segment_top_left_pos_x[ xAId ][ i ]                                   uvlc()

             ats_segment_top_left_pos_y[ xAId ][ i ]                                   uvlc()

             ats_segment_width[ xAId ][ i ]                                            uvlc()

             ats_segment_height[ xAId ][ i ]                                           uvlc()

         }

         return ats_num_atlas_segments_minus_1[ xAId ] + 1

     }




    AV2 Specification                                                                Page 79 of 1169
```

<a id="s-5-10"></a>

### § 5.10 Operating point set OBU syntax

```text
§   5.10. Operating point set OBU syntax
     operating_point_set_obu( ) {                                               Descriptor

         ops_reset_flag[ obu_xlayer_id ]                                           f(1)

         ops_id[ obu_xlayer_id ]                                                   f(4)

         opsID = ops_id[ obu_xlayer_id ]

         ops_cnt[ obu_xlayer_id ][ opsID ]                                         f(3)

         if ( ops_cnt[ obu_xlayer_id ][ opsID ] > 0 ) {

             ops_priority[ obu_xlayer_id ][ opsID ]                                f(4)

             ops_intent[ obu_xlayer_id ][ opsID ]                                  f(7)

             ops_intent_present_flag[ obu_xlayer_id ][ opsID ]                     f(1)

             ops_ptl_present_flag[ obu_xlayer_id ][ opsID ]                        f(1)

             ops_color_info_present_flag[ obu_xlayer_id ][ opsID ]                 f(1)

             if ( obu_xlayer_id == GLOBAL_XLAYER_ID ) {

                 ops_mlayer_info_idc[ opsID ]                                      f(2)

             } else {

                 ops_reserved_2bits                                                f(2)

             }

             for( i = 0; i < ops_cnt[ obu_xlayer_id ][ opsID ]; i++ ) {

                 operating_point_payload( obu_xlayer_id, opsID, i )

             }

         }

     }


```

<a id="s-5-11"></a>

### § 5.11 Operating point payload syntax

```text
§   5.11. Operating point payload syntax
     operating_point_payload( xId, opsID, i ) {                                 Descriptor

         ops_data_size[ xId ][ opsID ][ i ]                                      leb128()

         startPos = get_position( )

         if ( ops_intent_present_flag[ xId ][ opsID ] ) {

             ops_op_intent[ xId ][ opsID ][ i ]                                    f(7)

         }

         if ( ops_ptl_present_flag[ xId ][ opsID ] ) {

             if ( xId == GLOBAL_XLAYER_ID ) {

                 ops_aggregate_info( opsID, i )

             } else {

                 ops_seq_profile_tier_level_info( xId, opsID, i, xId )

             }

         }

         if ( ops_color_info_present_flag[ xId ][ opsID ] ) {

             ops_color_info( opsID, i )

         }

         ops_decoder_model_info_for_this_op_present_flag[ xId ][ opsID ][ i ]



    AV2 Specification                                                           Page 80 of 1169
                                                                                       f(1)

     if ( ops_decoder_model_info_for_this_op_present_flag[ xId ][ opsID ][ i ] ) {

         ops_decoder_model_info( opsID, i )

     }

     ops_initial_display_delay_present_flag[ xId ][ opsID ][ i ]                       f(1)

     if ( ops_initial_display_delay_present_flag[ xId ][ opsID ][ i ] ) {

         ops_initial_display_delay_minus_1[ xId ][ opsID ][ i ]                        f(4)

     }

     if ( xId == GLOBAL_XLAYER_ID ) {

         ops_xlayer_map[ opsID ][ i ]                                                  f(31)

         k = 0

         for ( j = 0; j < 31; j++ ) {

             if ( ops_xlayer_map[ opsID ][ i ] & (1 << j) ) {

                 OpsxLayerId[ xId ][ opsID ][ i ][ k ] = j

                 k++

                 if ( ops_ptl_present_flag[ xId ][ opsID ] ) {

                     ops_seq_profile_tier_level_info( xId, opsID, i, j )

                 }

                 idc = ops_mlayer_info_idc[ opsID ]

                 if ( idc == 1 ) {

                     ops_mlayer_info( xId, opsID, i, j )

                 } else if ( idc == 2 ) {

                     ops_mlayer_explicit_info_flag[ opsID ][ i ][ j ]                  f(1)

                     if ( ops_mlayer_explicit_info_flag[ opsID ][ i ][ j ] ) {

                         ops_mlayer_info( xId, opsID, i, j )

                     } else {

                         ops_embedded_ops_id[ opsID ][ i ][ j ]                        f(4)

                         ops_embedded_op_index[ opsID ][ i ][ j ]                      f(3)

                     }

                 }

             }

         }

         XCount[ xId ][ opsID ][ i ] = k

     } else {

         XCount[ xId ][ opsID ][ i ] = 1

         OpsxLayerId[ xId ][ opsID ][ i ][ 0 ] = xId

         ops_mlayer_info( xId, opsID, i, xId )

     }

     byte_alignment()

     opsBytes = (get_position() - startPos) >> 3

 }




AV2 Specification                                                                    Page 81 of 1169
```

<a id="s-5-11-1"></a>

#### § 5.11.1 Operating point set aggregate info syntax

```text
§   5.11.1. Operating point set aggregate info syntax

     ops_aggregate_info( opsID, i ) {                                             Descriptor

         ops_config_idc[ opsID ][ i ]                                                f(6)

         ops_aggregate_level_idx[ opsID ][ i ]                                       f(5)

         ops_max_tier_flag[ opsID ][ i ]                                             f(1)

         ops_max_interop[ opsID ][ i ]                                               f(4)

     }


```

<a id="s-5-11-2"></a>

#### § 5.11.2 Operating point set sequence profile tier level information syntax

```text
§   5.11.2. Operating point set sequence profile tier level information syntax

     ops_seq_profile_tier_level_info( xId, opsID, i, j ) {                        Descriptor

         ops_seq_profile_idc[ xId ][ opsID ][ i ][ j ]                               f(5)

         ops_level_idx[ xId ][ opsID ][ i ][ j ]                                     f(5)

         ops_tier_flag[ xId ][ opsID ][ i ][ j ]                                     f(1)

         ops_mlayer_count[ xId ][ opsID ][ i ][ j ]                                  f(3)

         ops_ptl_reserved_2bits                                                      f(2)

     }


```

<a id="s-5-11-3"></a>

#### § 5.11.3 Operating point set decoder model info syntax

```text
§   5.11.3. Operating point set decoder model info syntax

     ops_decoder_model_info( opsID, i ) {                                         Descriptor

         ops_decoder_buffer_delay[ obu_xlayer_id ][ opsID ][ i ]                    uvlc()

         ops_encoder_buffer_delay[ obu_xlayer_id ][ opsID ][ i ]                    uvlc()

         ops_low_delay_mode_flag[ obu_xlayer_id ][ opsID ][ i ]                      f(1)

     }


```

<a id="s-5-11-4"></a>

#### § 5.11.4 Operating point set color info syntax

```text
§   5.11.4. Operating point set color info syntax

     ops_color_info( opsID, i ) {                                                 Descriptor

         ops_color_description_idc[ obu_xlayer_id ][ opsID ][ i ]                   rg(2)

         if ( ops_color_description_idc[ obu_xlayer_id ][ opsID ][ i ] == 0 ) {

             ops_color_primaries[ obu_xlayer_id ][ opsID ][ i ]                      f(8)

             ops_transfer_characteristics[ obu_xlayer_id ][ opsID ][ i ]             f(8)

             ops_matrix_coefficients[ obu_xlayer_id ][ opsID ][ i ]                  f(8)

         }

         ops_full_range_flag[ obu_xlayer_id ][ opsID ][ i ]                          f(1)

     }


```

<a id="s-5-11-5"></a>

#### § 5.11.5 Operating point set mlayer info syntax

```text
§   5.11.5. Operating point set mlayer info syntax

     ops_mlayer_info( obuXLId, opsID, opIndex, xLId ) {                           Descriptor

         ops_mlayer_map[ obuXLId ][ opsID ][ opIndex ][ xLId ]                       f(8)

         mCount = 0

         for ( j = 0; j < 8; j++ ) {




    AV2 Specification                                                             Page 82 of 1169
             if (ops_mlayer_map[ obuXLId ][ opsID ][ opIndex ][ xLId ] & (1 << j)) {

                 ops_tlayer_map[ obuXLId ][ opsID ][ opIndex ][ xLId ][ j ]               f(4)

                 tCount = 0

                 for ( k = 0; k < 4; k++ ) {

                     if ( ops_tlayer_map[ obuXLId ][ opsID ][ opIndex ][ xLId ][ j ]

                               & (1 << k) ) {

                         tCount++

                     }

                 }

                 mCount++

             }

         }

     }


```

<a id="s-5-12"></a>

### § 5.12 Buffer removal timing OBU syntax

```text
§   5.12. Buffer removal timing OBU syntax
     buffer_removal_timing_obu() {                                                     Descriptor

         br_ops_dependent_flag                                                            f(1)

         if ( br_ops_dependent_flag ) {

             br_ops_id                                                                    f(4)

             br_ops_cnt[ br_ops_id ]                                                      f(3)

             for ( i = 0; i < br_ops_cnt[ br_ops_id ]; i++ ) {

                 br_decoder_model_present_op_flag[ br_ops_id ][ i ]                       f(1)

                 if ( br_decoder_model_present_op_flag[ br_ops_id ][ i ] ) {

                     br_time_op[ br_ops_id ][ i ]                                        rg(4)

                 }

             }

         } else {

             br_time                                                                     rg(4)

         }

     }


```

<a id="s-5-13"></a>

### § 5.13 Quantizer Matrix OBU syntax

```text
§   5.13. Quantizer Matrix OBU syntax
     quantizer_matrix_obu( ) {                                                         Descriptor

         qm_bit_map                                                                      f(15)

         qm_chroma_info_present_flag                                                      f(1)

         numPlanes = qm_chroma_info_present_flag ? 3 : 1

         if ( qm_bit_map == 0 ){

             for ( level = 0; level < NUM_CUSTOM_QMS; level++ ) {

                 QmProtected[ level ] = 1

                 QmNumPlanes[ level ]     = numPlanes

                 QmDataPresent[ level ] = 0



    AV2 Specification                                                                  Page 83 of 1169
                 QmMLayerId[ level ] = -1

                 QmTLayerId[ level ] = -1

             }

         } else {

             for ( level = 0; level < 15; level++ ) {

                 if ( qm_bit_map & (1 << level) ) {

                     QmSeen[ level ] = 1

                     QmProtected[ level ] = 1

                     QmNumPlanes[ level ] = numPlanes

                     QmMLayerId[ level ] = obu_mlayer_id

                     QmTLayerId[ level ] = obu_tlayer_id

                     QmDataPresent[ level ] = 1

                     qm_is_default_flag                                           f(1)

                     if ( qm_is_default_flag ) {

                         QmDataPresent[ level ] = 0

                     } else {

                         for ( t = 0; t < 3; t++ ){

                             for ( plane = 0; plane < numPlanes; plane++ ) {

                                 user_defined_qm( level, t, plane )

                             }

                         }

                     }

                 }

             }

         }

     }


```

<a id="s-5-14"></a>

### § 5.14 Film grain OBU syntax

```text
§   5.14. Film grain OBU syntax
     film_grain_obu( ) {                                                       Descriptor

         fgm_update_flags                                                         f(8)

         fgm_chroma_idc                                                          uvlc()

         if ( fgm_chroma_idc == CHROMA_FORMAT_420 ) {

             subX = 1

             subY = 1

         } else if ( fgm_chroma_idc == CHROMA_FORMAT_444 ) {

             subX = 0

             subY = 0

         } else if ( fgm_chroma_idc == CHROMA_FORMAT_422 ) {

             subX = 1

             subY = 0

         } else if ( fgm_chroma_idc == CHROMA_FORMAT_400 ) {




    AV2 Specification                                                          Page 84 of 1169
             subX = 1

             subY = 1

         }

         monochrome = fgm_chroma_idc == CHROMA_FORMAT_400

         for ( i = 0; i < MAX_FILM_GRAIN; i++ ) {

             if ( fgm_update_flags & (1 << i) ) {

                 FilmGrainPresent[ i ] = 1

                 film_grain_model( monochrome, subX, subY)

                 save_grain_model( i )

                 FgmTLayerId[ i ] = obu_tlayer_id

                 FgmMLayerId[ i ] = obu_mlayer_id

                 FgmChromaIdc[ i ] = fgm_chroma_idc

             }

         }

     }


```

<a id="s-5-15"></a>

### § 5.15 Content interpretation OBU syntax

```text
§   5.15. Content interpretation OBU syntax
     content_interpretation_obu() {                          Descriptor

         ci_scan_type_idc                                       f(2)

         ci_color_description_present_flag                      f(1)

         ci_chroma_sample_position_present_flag                 f(1)

         ci_aspect_ratio_info_present_flag                      f(1)

         ci_timing_info_present_flag                            f(1)

         ci_reserved_2bit                                       f(2)

         ci_color_primaries = CP_UNSPECIFIED

         ci_transfer_characteristics = TC_UNSPECIFIED

         ci_matrix_coefficients = MC_UNSPECIFIED

         ci_full_range_flag = 0

         if ( ci_color_description_present_flag ) {

             ci_color_description_idc                          rg(2)

             if ( ci_color_description_idc == 0 ) {

                 ci_color_primaries                             f(8)

                 ci_transfer_characteristics                    f(8)

                 ci_matrix_coefficients                         f(8)

             }

             ci_full_range_flag                                 f(1)

         }

         if ( ci_chroma_sample_position_present_flag ) {

             ci_chroma_sample_position_top                     uvlc()

             if ( ci_scan_type_idc != 1 ) {

                 ci_chroma_sample_position_bottom              uvlc()




    AV2 Specification                                        Page 85 of 1169
             } else {

                 ci_chroma_sample_position_bottom = ci_chroma_sample_position_top

             }

         } else {

             ci_chroma_sample_position_top = CSP_UNSPECIFIED

             ci_chroma_sample_position_bottom = CSP_UNSPECIFIED

         }

         if ( ci_aspect_ratio_info_present_flag ) {

             ci_aspect_ratio_idc                                                       f(8)

             if ( ci_aspect_ratio_idc == 255 ) {

                 ci_sar_width                                                         uvlc()

                 ci_sar_height                                                        uvlc()

             } else {

                 ci_sar_width = Aspect_Ratio_Width[ ci_aspect_ratio_idc ]

                 ci_sar_height = Aspect_Ratio_Height[ ci_aspect_ratio_idc ]

             }

         }

         if ( ci_timing_info_present_flag ) {

             timing_info()

         }

     }


    where the tables Aspect_Ratio_Width and Aspect_Ratio_Height are specified as:

     Aspect_Ratio_Width[ 17 ] = {         0, 1, 12, 10, 16, 40, 24, 20,
                                         32, 80, 18, 15, 64, 160, 4, 3, 2 }

     Aspect_Ratio_Height[ 17 ] = {         0, 1, 11, 11, 11, 33, 11, 11,
                                          11, 33, 11, 11, 33, 99, 3, 2, 1 }


```

<a id="s-5-16"></a>

### § 5.16 Padding OBU syntax

```text
§   5.16. Padding OBU syntax
     padding_obu( ) {                                                               Descriptor

         for ( i = 0; i < obu_padding_length; i++ ) {

             obu_padding_byte                                                          f(8)

         }

     }




    AV2 Specification                                                               Page 86 of 1169
      NOTE: obu_padding_length is not coded in the bitstream but can be computed based on the OBU
      size minus the number of trailing bytes. In practice, though, since this is padding data meant to be
      skipped, decoders do not need to determine either that length or the number of trailing bytes. They
      can ignore the entire OBU. The last byte of the valid content of the payload data for this OBU type is
      considered to be the last byte that is not equal to zero. This rule is to prevent the dropping of valid
      bytes by systems that interpret trailing zero bytes as a continuation of the trailing bits in an OBU.
      This implies that when any payload data is present for this OBU type, at least one byte of the payload
      data (including the trailing bit) shall not be equal to 0.


      NOTE: A padding OBU with an obuPayloadSize of 0 is legal. This means the OBU has
      obu_padding_length of 0 and will not contain any trailing bits. A padding OBU with an
      obuPayloadSize of 1 is legal. This means the OBU has obu_padding_length of 0 and does contain
      trailing bits. This is allowed so that any OBU can be converted into a padding OBU in-place.


```

<a id="s-5-17"></a>

### § 5.17 Metadata OBU syntax

```text
§   5.17. Metadata OBU syntax
    This specification defines two distinct OBU types for carrying metadata:

      • OBU_METADATA_SHORT: using metadata short OBU syntax, and
      • OBU_METADATA_GROUP: using metadata group OBU syntax.

    Both OBU types use the same metadata_unit() syntax element to carry the actual metadata payload. The
    OBU_METADATA_SHORT type provides a compact header structure, while OBU_METADATA_GROUP
    provides extended capabilities including the ability to carry multiple metadata units within a single OBU
    with additional signaling for application-specific handling, layer targeting, and priority.

```

<a id="s-5-17-1"></a>

#### § 5.17.1 Metadata unit syntax

```text
§   5.17.1. Metadata unit syntax

     metadata_unit( metadataPayloadSize ) {                                                       Descriptor

       startPosition = get_position()

       if ( metadata_type == METADATA_TYPE_ITUT_T35 ) {

         metadata_itut_t35( metadataPayloadSize )

       } else if ( metadata_type == METADATA_TYPE_HDR_CLL ) {

         metadata_hdr_cll( )

       } else if ( metadata_type == METADATA_TYPE_HDR_MDCV ) {

         metadata_hdr_mdcv( )

       } else if ( metadata_type == METADATA_TYPE_TIMECODE ) {

         metadata_timecode( )

       } else if ( metadata_type == METADATA_TYPE_BANDING_HINTS ) {

         metadata_banding_hints( )

       } else if ( metadata_type == METADATA_TYPE_ICC_PROFILE ) {

         metadata_icc_profile( metadataPayloadSize )

       } else if ( metadata_type == METADATA_TYPE_SCAN_TYPE ) {

         metadata_scan_type( )

       } else if ( metadata_type == METADATA_TYPE_TEMPORAL_POINT_INFO ) {




    AV2 Specification                                                                             Page 87 of 1169
             metadata_temporal_point_info( )

         } else if ( metadata_type == METADATA_TYPE_DECODED_FRAME_HASH ) {

             metadata_decoded_frame_hash( )

         } else if ( metadata_type == METADATA_TYPE_USER_DATA_UNREGISTERED ) {

             metadata_user_data_unregistered( metadataPayloadSize )

         }

         currentPosition = get_position( )

         parsedPayloadBits = currentPosition - startPosition

         remainingMuPayloadBits = metadataPayloadSize * 8 - parsedPayloadBits

         for ( j = 0; j < remainingMuPayloadBits; j++ ) {

             metadata_unit_remaining_bit                                                                f(1)

         }

     }



         NOTE: The exact syntax of metadata_unit is not defined in this specification when metadata_type is
         equal to a value reserved for future use or a user private value. Decoders should ignore the
         metadata_unit() if they do not understand the metadata_type. For OBU_METADATA_SHORT, this means ignoring
         the entire OBU. For OBU_METADATA_GROUP, decoders should skip only the unrecognized metadata_unit() and
         continue processing other metadata units within the same OBU.

```

<a id="s-5-17-2"></a>

#### § 5.17.2 Metadata short OBU syntax

```text
§   5.17.2. Metadata short OBU syntax

     metadata_short_obu( obuPayloadSize ) {                                                          Descriptor

         metadata_is_suffix                                                                             f(1)

         metadata_necessity_idc = 0

         metadata_application_id = 0

         muh_layer_idc                                                                                  f(3)

         muh_cancel_flag                                                                                f(1)

         muh_persistence_idc                                                                            f(3)

         muh_priority = 0

         metadata_type                                                                                leb128()

         if ( muh_cancel_flag ) {

             return

         }

         metadataPayloadSize = obuPayloadSize - 2 - Leb128Bytes

         metadata_unit( metadataPayloadSize )

     }


```

<a id="s-5-17-3"></a>

#### § 5.17.3 Metadata group OBU syntax

```text
§   5.17.3. Metadata group OBU syntax

     metadata_group_obu() {                                                                          Descriptor

         metadata_is_suffix                                                                             f(1)

         metadata_necessity_idc                                                                         f(2)




    AV2 Specification                                                                                 Page 88 of 1169
     metadata_application_id                                      f(5)

     metadata_unit_cnt_minus_1                                  leb128()

     for ( i = 0; i <= metadata_unit_cnt_minus_1; i++ ) {

         metadata_type                                          leb128()

         muh_header_size                                          f(7)

         muh_cancel_flag                                          f(1)

         headerRemainingBytes = muh_header_size

         if ( !muh_cancel_flag ) {

             muh_payload_size                                   leb128()

             headerRemainingBytes -= Leb128Bytes

             muh_layer_idc                                        f(3)

             muh_persistence_idc                                  f(3)

             muh_priority                                         f(8)

             muh_reserved_zero_2bits                              f(2)

             headerRemainingBytes -= 2

             if ( muh_layer_idc == LAYER_VALUES ) {

                 if ( obu_xlayer_id == GLOBAL_XLAYER_ID ) {

                     muh_xlayer_map                               f(32)

                     headerRemainingBytes -= 4

                     for ( n = 0; n < 31; n++ ) {

                         if ( muh_xlayer_map & (0x1 << n) ) {

                             muh_mlayer_map                       f(8)

                             headerRemainingBytes -= 1

                         }

                     }

                 } else {

                     muh_mlayer_map                               f(8)

                     headerRemainingBytes -= 1

                 }

             }

         }

         for ( j = 0; j < headerRemainingBytes; j++ ) {

             muh_header_extension_byte                            f(8)

         }

         if ( !muh_cancel_flag ) {

             metadata_unit( muh_payload_size )

         }

     }

 }




AV2 Specification                                               Page 89 of 1169
```

<a id="s-5-17-4"></a>

#### § 5.17.4 Metadata ITUT T35 syntax

```text
§   5.17.4. Metadata ITUT T35 syntax

     metadata_itut_t35( metadataPayloadSize ) {                                                   Descriptor

         itu_t_t35_country_code                                                                      f(8)

         t35PayloadSize = metadataPayloadSize - 1

         if ( itu_t_t35_country_code == 0xFF ) {

             itu_t_t35_country_code_extension_byte                                                   f(8)

             t35PayloadSize--

         }

         itu_t_t35_payload_bytes                                                                le(t35PayloadSi
                                                                                                      ze)

     }



         NOTE: The exact syntax of itu_t_t35_payload_bytes is not defined in this specification. External
         specifications can define the syntax. Decoders should ignore the entire OBU if they do not understand
         it.

```

<a id="s-5-17-5"></a>

#### § 5.17.5 Metadata high dynamic range content light level syntax

```text
§   5.17.5. Metadata high dynamic range content light level syntax

     metadata_hdr_cll( ) {                                                                        Descriptor

         max_cll                                                                                     f(16)

         max_fall                                                                                    f(16)

     }


```

<a id="s-5-17-6"></a>

#### § 5.17.6 Metadata high dynamic range mastering display color volume syntax

```text
§   5.17.6. Metadata high dynamic range mastering display color volume syntax

     metadata_hdr_mdcv( ) {                                                                       Descriptor

         for ( i = 0; i < 3; i++ ) {

             primary_chromaticity_x[ i ]                                                             f(16)

             primary_chromaticity_y[ i ]                                                             f(16)

         }

         white_point_chromaticity_x                                                                  f(16)

         white_point_chromaticity_y                                                                  f(16)

         luminance_max                                                                               f(32)

         luminance_min                                                                               f(32)

     }


```

<a id="s-5-17-7"></a>

#### § 5.17.7 Metadata timecode syntax

```text
§   5.17.7. Metadata timecode syntax

     metadata_timecode( ) {                                                                       Descriptor

         counting_type                                                                               f(5)

         full_timestamp_flag                                                                         f(1)

         discontinuity_flag                                                                          f(1)

         cnt_dropped_flag                                                                            f(1)

         n_frames                                                                                    f(9)




    AV2 Specification                                                                              Page 90 of 1169
         if ( full_timestamp_flag ) {

             seconds_value                                                  f(6)

             minutes_value                                                  f(6)

             hours_value                                                    f(5)

         } else {

             seconds_flag                                                   f(1)

             if ( seconds_flag ) {

                 seconds_value                                              f(6)

                 minutes_flag                                               f(1)

                 if ( minutes_flag ) {

                     minutes_value                                          f(6)

                     hours_flag                                             f(1)

                     if ( hours_flag ) {

                         hours_value                                        f(5)

                     }

                 }

             }

         }

         time_offset_length                                                 f(5)

         if ( time_offset_length > 0 ) {

             time_offset_value                                         f(time_offset_l
                                                                            ength)

         }

     }


```

<a id="s-5-17-8"></a>

#### § 5.17.8 Metadata banding hints syntax

```text
§   5.17.8. Metadata banding hints syntax

     metadata_banding_hints( ) {                                         Descriptor

         coding_banding_present_flag                                        f(1)

         source_banding_present_flag                                        f(1)

         if ( coding_banding_present_flag ) {

             banding_hints_flag                                             f(1)

             if ( banding_hints_flag ) {

                 three_color_components_flag                                f(1)

                 numComponents = three_color_components_flag ? 3 : 1

                 for ( plane = 0; plane < numComponents; plane++ ) {

                     banding_in_component_present_flag                      f(1)

                     if ( banding_in_component_present_flag ) {

                         max_band_width_minus_4                             f(6)

                         max_band_step_minus_1                              f(4)

                     }

                 }

                 band_units_information_present_flag                        f(1)



    AV2 Specification                                                    Page 91 of 1169
                 if ( band_units_information_present_flag ) {

                     num_band_units_rows_minus_1                                       f(5)

                     num_band_units_cols_minus_1                                       f(5)

                     varying_size_band_units_flag                                      f(1)

                     if ( varying_size_band_units_flag ) {

                         band_block_in_luma_samples                                    f(3)

                         for ( r = 0; r <= num_band_units_rows_minus_1; r++ ) {

                             vert_size_in_band_blocks_minus_1                          f(5)

                         }

                         for ( c = 0; c <= num_band_units_cols_minus_1; c++ ) {

                             horz_size_in_band_blocks_minus_1                          f(5)

                         }

                     }

                     for ( r = 0; r <= num_band_units_rows_minus_1; r++ ) {

                         for ( c = 0; c <= num_band_units_cols_minus_1; c++ ) {

                             banding_in_band_unit_present_flag                         f(1)

                         }

                     }

                 }

             }

         }

     }


```

<a id="s-5-17-9"></a>

#### § 5.17.9 Metadata ICC profile syntax

```text
§   5.17.9. Metadata ICC profile syntax

     metadata_icc_profile( metadataPayloadSize ) {                                  Descriptor

         icc_profile_data_payload_bytes                                           le(metadataPayl
                                                                                      oadSize)

     }


```

<a id="s-5-17-10"></a>

#### § 5.17.10 Metadata scan type syntax

```text
§   5.17.10. Metadata scan type syntax

     metadata_scan_type( ) {                                                        Descriptor

         mps_pic_struct_type                                                           f(5)

         mps_source_scan_type_idc                                                      f(2)

         mps_duplicate_flag                                                            f(1)

     }


```

<a id="s-5-17-11"></a>

#### § 5.17.11 Metadata temporal point info syntax

```text
§   5.17.11. Metadata temporal point info syntax

     metadata_temporal_point_info( ) {                                              Descriptor

         frame_presentation_time                                                     leb128()

     }




    AV2 Specification                                                               Page 92 of 1169
```

<a id="s-5-17-12"></a>

#### § 5.17.12 Metadata decoded frame hash syntax

```text
§   5.17.12. Metadata decoded frame hash syntax

     metadata_decoded_frame_hash( ) {                             Descriptor

         hash_type                                                   f(4)

         per_plane                                                   f(1)

         has_grain                                                   f(1)

         is_monochrome                                               f(1)

         reserved                                                    f(1)

         if ( per_plane ) {

             numPlanes = is_monochrome ? 1 : 3

             for ( i = 0; i < numPlanes; i++ ) {

                 plane_hash[ i ]                                    le(16)

             }

         } else {

             frame_hash                                             le(16)

         }

     }


```

<a id="s-5-17-13"></a>

#### § 5.17.13 Metadata user data unregistered syntax

```text
§   5.17.13. Metadata user data unregistered syntax

     metadata_user_data_unregistered( metadataPayloadSize ) {     Descriptor

         uuid_iso_iec_11578                                         f(128)

         for( i = 16; i < metadataPayloadSize; i++ ) {

             user_data_payload_byte                                  f(8)

         }

     }


```

<a id="s-5-18"></a>

### § 5.18 Frame header syntax

```text
§   5.18. Frame header syntax
```

<a id="s-5-18-1"></a>

#### § 5.18.1 General frame header syntax

```text
§   5.18.1. General frame header syntax

     frame_header( isFirst ) {                                    Descriptor

         if ( isFirst ) {

             SeenFrameHeader = 1

             CountFrameHeaderForLevelConstraint = 1

             FrameSymbolCount = 0

             startBitPos = get_position( )

             frame_header_info( )

             NumFrameHeaderBits = get_position( ) - startBitPos

             FirstPictureInTU = 0

             if ( IsBridge ) {

                 NumTiles = TileCols * TileRows

                 tg_start = 0

                 tg_end = NumTiles - 1




    AV2 Specification                                             Page 93 of 1169
                 tile_group_payload( 0 )

             } else if ( ShowExistingFrame ||

                 TipFrameMode == TIP_FRAME_AS_OUTPUT ||

                 bru_inactive ) {

                 decode_frame_wrapup( )

                 SeenFrameHeader = 0

                 CountFrameHeaderForLevelConstraint = 0

             } else {

                 TileNum = 0

             }

         } else {

             CountFrameHeaderForLevelConstraint = 0

             frame_header_copy()

         }

     }


    where the syntax structure frame_header_copy is defined as:

     frame_header_copy() {                                                             Descriptor

         for ( i = 0; i < NumFrameHeaderBits; i++ ) {

             header_bit[ i ]                                                              f(1)

         }

     }


```

<a id="s-5-18-2"></a>

#### § 5.18.2 Frame header info syntax

```text
§   5.18.2. Frame header info syntax

     frame_header_info( ) {                                                            Descriptor

         keyFrame = obu_type == OBU_CLOSED_LOOP_KEY || obu_type == OBU_OPEN_LOOP_KEY

         IsRegular = ( obu_type == OBU_OPEN_LOOP_KEY ||

                    obu_type == OBU_REGULAR_TILE_GROUP ||

                    obu_type == OBU_REGULAR_TIP ||

                    obu_type == OBU_REGULAR_SEF ||

                    obu_type == OBU_SWITCH ||

                    obu_type == OBU_RAS_FRAME ||

                    obu_type == OBU_BRIDGE_FRAME )

         for ( i = 0; i < NUM_CUSTOM_QMS; i++ ) {

             QmSeen[ i ] = 0

         }

         startCVS = obu_type == OBU_CLOSED_LOOP_KEY && FirstPictureInTU

         if ( startCVS ) {

             OlkEncountered = 0

             for( i = 0; i < MAX_NUM_MLAYERS; i++ ) {

                 OlkRefresh[ i ] = 0




    AV2 Specification                                                                  Page 94 of 1169
       }

       flush_implicit_output_frames( 0 )

   }

   if ( OlkEncountered && IsRegular && FirstPictureInTU ) {

       flush_implicit_output_frames( 1 )

       OlkEncountered = 0

       allowedFrames = 0

       for ( i = 0; i < MAX_NUM_MLAYERS; i++ ) {

           allowedFrames |= OlkRefresh[ i ]

           OlkRefresh[ i ] = 0

       }

       for ( i = 0; i < NUM_REF_FRAMES; i++ ) {

           if ( ( allowedFrames & (1 << i) ) == 0 && RefLongTermId[ i ] == -1 )

            RefValid[ i ] = 0

       }

   }

   IsBridge = obu_type == OBU_BRIDGE_FRAME

   if ( IsBridge ) {

       cur_mfh_id = 0

   } else {

       cur_mfh_id                                                                  uvlc()

   }

   if ( cur_mfh_id == 0 ) {

       seq_header_id_in_frame_header                                               uvlc()

       load_sequence_header( seq_header_id_in_frame_header )

       mfh_deblocking_filter_update[ cur_mfh_id ] = 0

   } else {

       load_sequence_header( MfhSeqHeaderId[ cur_mfh_id ] )

   }

   if ( keyFrame ) {

       if ( seq_lcr_id != 0 ) {

           activate_layer_configuration_record( seq_lcr_id )

       }

   }

   if ( cur_mfh_id == 0 || !mfh_frame_size_present_flag[ cur_mfh_id ] ) {

       mfh_frame_width_minus_1[ cur_mfh_id ] = max_frame_width_minus_1

       mfh_frame_height_minus_1[ cur_mfh_id ] = max_frame_height_minus_1

   }

   if ( keyFrame && FirstPictureInTU ) {

       reset_qm( )

   }




AV2 Specification                                                                 Page 95 of 1169
   if ( IsBridge ) {

       n = CeilLog2(NumRefFrames)

       bridge_frame_ref_idx                                                 f(n)

   }

   allFrames = (1 << NumRefFrames) - 1

   use_bru = 0

   bru_inactive = 0

   if ( single_picture_header_flag ) {

       ShowExistingFrame = 0

       FrameType = KEY_FRAME

       FrameIsIntra = 1

       immediate_output_frame = 1

       implicit_output_frame = 0

   } else {

       ShowExistingFrame = is_sef()

       if ( ShowExistingFrame == 1 ) {

           n = CeilLog2(NumRefFrames)

           frame_to_show_map_idx                                            f(n)

           derive_sef_order_hint                                            f(1)

           if ( derive_sef_order_hint == 0 ) {

               sef_order_hint                                          f(OrderHintBits
                                                                              )

               OrderHintLsbs = sef_order_hint

               OrderHint = get_disp_order_hint()

           } else {

               OrderHint = RefOrderHint[ frame_to_show_map_idx ]

           }

           if ( IsRegular && OlkEncountered && !FirstPictureInTU ) {

               OlkTUOrderHint = derive_sef_order_hint ?

                       RefOrderHint[ frame_to_show_map_idx ] :

                       OrderHint

           }

           refresh_frame_flags = 0

           FrameType = RefFrameType[ frame_to_show_map_idx ]

           immediate_output_frame = 1

           film_grain_config()

           if ( derive_sef_order_hint ) {

               save_grain_params( frame_to_show_map_idx )

           }

           TipFrameMode = TIP_FRAME_DISABLED

           return

       }



AV2 Specification                                                        Page 96 of 1169
     if ( IsBridge ) {

         FrameType = INTER_FRAME

     } else if ( obu_type == OBU_SWITCH || obu_type == OBU_RAS_FRAME ) {

         restricted_prediction_switch                                            f(1)

         FrameType = SWITCH_FRAME

     } else if ( is_tip_frame() ) {

         FrameType = INTER_FRAME

     } else if ( obu_type == OBU_CLOSED_LOOP_KEY ||

                 obu_type == OBU_OPEN_LOOP_KEY ) {

         FrameType = KEY_FRAME

     } else {

         frame_is_inter                                                          f(1)

         FrameType = frame_is_inter ? INTER_FRAME : INTRA_ONLY_FRAME

     }

     LongTermId = -1

     if ( FrameType == KEY_FRAME ) {

         long_term_id_plus_1                                                f(long_term_fra
                                                                              me_id_bits)

         LongTermId = long_term_id_plus_1 - 1

     }

     num_key_ref_frames = 0

     if ( (obu_type == OBU_RAS_FRAME || obu_type == OBU_OPEN_LOOP_KEY) &&

         long_term_frame_id_bits != 0) {

         num_key_ref_frames                                                      f(3)

         for ( i = 0; i < num_key_ref_frames; i++ ) {

             ref_long_term_id[ i ]                                          f(long_term_fra
                                                                              me_id_bits)

         }

     }

     if ( FrameType == SWITCH_FRAME && restricted_prediction_switch ) {

         for (i = 0; i < NUM_REF_FRAMES; i++) {

             if ( MLayerPresenceMap[RefMLayerId[i]][obu_mlayer_id] ) {

                 if ( is_frame_eligible_for_output( i ) ) {

                     output_frame_buffers( i )

                 }

                 RefOrderHint[ i ] = RESTRICTED_OH

             }

         }

     }

     if ( obu_type == OBU_RAS_FRAME ||

         (obu_type == OBU_SWITCH && restricted_prediction_switch) ) {

         reset_qm()




AV2 Specification                                                             Page 97 of 1169
       }

       FrameIsIntra = (FrameType == INTRA_ONLY_FRAME ||

                FrameType == KEY_FRAME)

       if ( IsBridge || obu_type == OBU_OPEN_LOOP_KEY ) {

           immediate_output_frame = 0

       } else {

           immediate_output_frame                                                        f(1)

       }

       if ( IsBridge || immediate_output_frame || monotonic_output_order_flag ) {

           implicit_output_frame = 0

       } else {

           implicit_output_frame                                                         f(1)

       }

   }

   if ( use_256x256_superblock ) {

       SbSize = FrameIsIntra ? BLOCK_128X128 : BLOCK_256X256

   } else if ( use_128x128_superblock ) {

       SbSize = BLOCK_128X128

   } else {

       SbSize = BLOCK_64X64

   }

   if ( FrameType == KEY_FRAME && immediate_output_frame ) {

       for ( i = 0; i < REFS_PER_FRAME; i++ ) {

           OrderHints[ i ] = 0

       }

   }

   disable_cross_frame_cdf_init = 0

   if ( IsBridge ) {

       primary_ref_frame = PRIMARY_REF_NONE

       OrderHintLsbs = RefOrderHintLsbs[ bridge_frame_ref_idx ]

       OrderHint = RefOrderHint[ bridge_frame_ref_idx ]

   } else {

       if ( FrameType == SWITCH_FRAME ) {

           frame_size_override_flag = 1

       } else if ( single_picture_header_flag ) {

           frame_size_override_flag = 0

       } else {

           frame_size_override_flag                                                      f(1)

       }

       order_hint                                                                   f(OrderHintBits
                                                                                           )

       OrderHintLsbs = order_hint



AV2 Specification                                                                     Page 98 of 1169
       OrderHint = get_disp_order_hint()

       if ( FrameIsIntra || FrameType == SWITCH_FRAME ) {

           primary_ref_frame = PRIMARY_REF_NONE

       } else {

           signal_primary_ref_frame                                          f(1)

           if ( !is_tip_frame( ) ) {

               disable_cross_frame_cdf_init                                  f(1)

           }

           if ( signal_primary_ref_frame ) {

               primary_ref_frame                                             f(3)

           } else {

               primary_ref_frame = PRIMARY_REF_CHOOSE

           }

       }

   }

   FrameMvPrecision = MV_PRECISION_ONE_PEL

   MvPrecision = FrameMvPrecision

   allow_high_precision_mv = 0

   use_ref_frame_mvs = 0

   allow_intrabc = 0

   allow_global_intrabc = 0

   allow_local_intrabc = 0

   allow_high_precision_mv = 0

   allow_df_sub_pu = 0

   if ( IsBridge ) {

       bridge_frame_overwrite_flag                                           f(1)

   }

   if ( FrameType == KEY_FRAME ) {

       if ( obu_type == OBU_CLOSED_LOOP_KEY && max_mlayer_id == 0 ) {

           refresh_frame_flags = allFrames

       } else if ( enable_short_refresh_frame_flags ) {

           n = CeilLog2(NumRefFrames)

           frame_to_refresh                                                  f(n)

           refresh_frame_flags = 1 << frame_to_refresh

       } else {

           refresh_frame_flags                                          f(NumRefFrames)

       }

       if ( obu_type == OBU_CLOSED_LOOP_KEY && FirstPictureInTU ) {

           for ( i = 0; i < NumRefFrames; i++ ) {

               RefValid[i] = 0

           }




AV2 Specification                                                         Page 99 of 1169
       }

       if ( obu_type == OBU_CLOSED_LOOP_KEY ) {

           OlkEncountered = 0

           for( i = 0; i < MAX_NUM_MLAYERS; i++ ) {

               OlkRefresh[ i ] = 0

           }

       }

       if ( obu_type == OBU_OPEN_LOOP_KEY ) {

           OlkEncountered = 1

           OlkRefresh[ obu_mlayer_id ] = refresh_frame_flags

           if ( implicit_output_frame ) {

               OlkTUOrderHint = OrderHint

           }

       }

   } else if ( IsBridge && !bridge_frame_overwrite_flag ) {

       refresh_frame_flags = 1 << bridge_frame_ref_idx

   } else if ( obu_type == OBU_RAS_FRAME && max_mlayer_id == 0 ) {

       refresh_frame_flags = 0

       for ( i = 0; i < NumRefFrames; i++ ) {

           if ( !RefValid[i] || !long_term_id_in_use( RefLongTermId[i] ) ) {

               refresh_frame_flags |= (1 << i)

           }

       }

   } else if ( FrameType == SWITCH_FRAME ) {

       refresh_frame_flags                                                     f(NumRefFrames)

   } else if ( enable_short_refresh_frame_flags &&

               FrameType != SWITCH_FRAME &&

               FrameType != KEY_FRAME ) {

       has_refresh_frame_flags                                                      f(1)

       if ( has_refresh_frame_flags ) {

           n = CeilLog2(NumRefFrames)

           frame_to_refresh                                                         f(n)

           refresh_frame_flags = 1 << frame_to_refresh

       } else {

           refresh_frame_flags = 0

       }

   } else {

       refresh_frame_flags                                                     f(NumRefFrames)

   }

   AllowedFrames = -1

   if ( IsRegular && OlkEncountered && !FirstPictureInTU ) {




AV2 Specification                                                               Page 100 of 1169
       AllowedFrames = 0

       for ( i = 0; i < MAX_NUM_MLAYERS; i++ ) {

           AllowedFrames |= OlkRefresh[ i ]

       }

       OlkRefresh[ obu_mlayer_id ] |= refresh_frame_flags

       if ( immediate_output_frame || implicit_output_frame ) {

           OlkTUOrderHint = OrderHint

       }

   }

   if ( FrameIsIntra ) {

       frame_size( )

       screen_content_params( )

       intrabc_params( )

       NumTotalRefs = 0

       TipFrameMode = TIP_FRAME_DISABLED

   } else {

       if ( FrameType == SWITCH_FRAME || IsBridge ) {

           explicitRefFrameMap = 1

       } else if ( explicit_ref_frame_map ) {

           frame_explicit_ref_frame_map                              f(1)

           explicitRefFrameMap = frame_explicit_ref_frame_map

       } else {

           explicitRefFrameMap = 0

       }

       if ( IsBridge ) {

           NumTotalRefs = 1

       } else if ( explicitRefFrameMap ) {

           num_total_refs                                            f(3)

           NumTotalRefs = num_total_refs

       } else {

           get_ref_frames( 0 )

       }

       for ( i = 0; i < NumTotalRefs; i++ ) {

           if ( IsBridge ) {

               ref_frame_idx[ i ] = bridge_frame_ref_idx

           } else if ( explicitRefFrameMap ) {

               n = CeilLog2(NumRefFrames)

               ref_frame_idx[ i ]                                    f(n)

           }

       }

       if ( IsBridge ) {




AV2 Specification                                                 Page 101 of 1169
         frame_size_with_bridge( )

     } else if ( frame_size_override_flag && FrameType != SWITCH_FRAME ) {

         frame_size_with_refs( )

     } else {

         frame_size( )

     }

     if ( !explicitRefFrameMap ) {

         get_ref_frames( 1 )

     }

     NumSameRefCompound = Min(num_same_ref_compound, NumTotalRefs)

     if ( enable_bru && FrameType == INTER_FRAME && !is_tip_frame( ) &&

         !IsBridge ) {

         use_bru                                                                  f(1)

         if ( use_bru ) {

             n = CeilLog2(NumTotalRefs)

             bru_ref                                                              f(n)

             bru_inactive                                                         f(1)

         }

     }

     if ( explicitRefFrameMap ) {

         for ( i = 0; i < NumTotalRefs; i++ ) {

             ScoresDistance[ i ] = get_relative_dist( OrderHint,

                         RefOrderHint[ ref_frame_idx[ i ] ] )

         }

     }

     get_past_future_cur_ref_lists()

     if ( FrameType == SWITCH_FRAME || !enable_ref_frame_mvs ||

         IsBridge || bru_inactive ) {

         use_ref_frame_mvs = 0

     } else {

         use_ref_frame_mvs                                                        f(1)

     }

     if ( use_ref_frame_mvs && NumTotalRefs > 1 && SbSize != BLOCK_64X64 ) {

         tmvp_sample_step_minus_1                                                 f(1)

         ProjStep = tmvp_sample_step_minus_1 + 1

     } else {

         ProjStep = 1

     }

     for ( i = 0; i < NumTotalRefs; i++ ) {

         FrameDistance[ i ] = get_relative_dist( OrderHint,

                       RefOrderHint[ ref_frame_idx[ i ] ] )




AV2 Specification                                                              Page 102 of 1169
         if ( RefOrderHint[ ref_frame_idx[ i ] ] == RESTRICTED_OH ) {

             FrameDistance[ i ] = -FrameDistance[ i ]

         }

     }

     for ( i = 0; i < NumTotalRefs; i++ ) {

         refFrame = i

         hint = RefOrderHint[ ref_frame_idx[ i ] ]

         OrderHints[ refFrame ] = hint

     }

     if ( enable_tip &&

         (use_ref_frame_mvs && NumTotalRefs >= 2) &&

         !bru_inactive ) {

         TipInterpFilter = EIGHTTAP_SHARP

         TipGlobalMv[ 0 ] = 0

         TipGlobalMv[ 1 ] = 0

         if ( EnableTipOutput && is_tip_frame( ) ) {

             TipFrameMode = TIP_FRAME_AS_OUTPUT

         } else {

             tip_frame_mode                                                  f(1)

             TipFrameMode = tip_frame_mode

         }

         frame_opfl_refine_type()

         if ( TipFrameMode != TIP_FRAME_DISABLED &&

             enable_tip_hole_fill ) {

             allow_tip_hole_fill                                             f(1)

         } else {

             allow_tip_hole_fill = 0

         }

         usesEqualWeight = enable_tip_refinemv &&

             NumFutureRefs > 0 && NumPastRefs > 0 &&

             ( opfl_refine_type != REFINE_NONE || enable_refinemv )

         if ( TipFrameMode == TIP_FRAME_DISABLED || usesEqualWeight ) {

             tip_global_wtd_index = 0

         } else {

             tip_global_wtd_index                                            f(3)

         }

         if ( TipFrameMode == TIP_FRAME_AS_OUTPUT ) {

             tip_mv_zero                                                     f(1)

             if ( !tip_mv_zero ) {

              tip_mv_row                                                     f(4)

              tip_mv_col                                                     f(4)




AV2 Specification                                                         Page 103 of 1169
                 if ( tip_mv_row != 0 ) {

                     tip_mv_row_sign                                            f(1)

                     TipGlobalMv[ 0 ] = tip_mv_row_sign ?

                      -tip_mv_row : tip_mv_row

                 }

                 if ( tip_mv_col != 0 ) {

                     tip_mv_col_sign                                            f(1)

                     TipGlobalMv[ 1 ] = tip_mv_col_sign ?

                      -tip_mv_col : tip_mv_col

                 }

             }

             tip_sharp                                                          f(1)

             if ( tip_sharp ) {

                 TipInterpFilter = EIGHTTAP_SHARP

             } else {

                 tip_regular                                                    f(1)

                 TipInterpFilter = tip_regular ? EIGHTTAP: EIGHTTAP_SMOOTH

             }

         }

     } else {

         TipFrameMode = TIP_FRAME_DISABLED

         if ( !bru_inactive && !IsBridge ) {

             frame_opfl_refine_type()

         }

     }

     if ( TipFrameMode != TIP_FRAME_AS_OUTPUT && !bru_inactive &&

         !IsBridge ) {

         screen_content_params( )

         intrabc_params( )

         max_drl_bits_minus_1 = seq_max_drl_bits_minus_1

         if ( allow_frame_max_drl_bits ) {

             change_drl                                                         f(1)

             if ( change_drl ) {

                 n = MAX_REF_MV_STACK_SIZE - 2

                 max_drl_bits_minus_1                                           ns(n)

                 if ( max_drl_bits_minus_1 >= seq_max_drl_bits_minus_1 ) {

                     max_drl_bits_minus_1 += 1

                 }

             }

         }

         if ( force_integer_mv ) {




AV2 Specification                                                            Page 104 of 1169
               FrameMvPrecision = MV_PRECISION_ONE_PEL

               UsePerBlockMvPrecision = 0

           } else {

               use_qtr_precision_mv                                              f(1)

               if ( use_qtr_precision_mv ) {

                   FrameMvPrecision = MV_PRECISION_QUARTER_PEL

               } else {

                   allow_high_precision_mv                                       f(1)

                   FrameMvPrecision = allow_high_precision_mv ?

                    MV_PRECISION_EIGHTH_PEL : MV_PRECISION_HALF_PEL

               }

               UsePerBlockMvPrecision = enable_flex_mvres

           }

           MvPrecision = FrameMvPrecision

           read_interpolation_filter( )

           for ( mode = INTERINTRA; mode < MOTION_MODES; mode++ ) {

               if ( !seq_frame_motion_modes_present_flag ) {

                   frame_enabled_motion_modes[ mode ] =

                    seq_enabled_motion_modes[ mode ]

               } else if ( seq_enabled_motion_modes[ mode ] ) {

                   frame_enabled_motion_modes[ mode ]                            f(1)

               } else {

                   frame_enabled_motion_modes[ mode ] = 0

               }

           }

       }

   }

   if ( TipFrameMode == TIP_FRAME_AS_OUTPUT ) {

       if ( enable_tip_explicit_qp ) {

           quantization_params( )

       }

       if ( enable_df_sub_pu ) {

           allow_df_sub_pu                                                       f(1)

       }

       if ( allow_df_sub_pu ) {

           apply_deblocking_filter_tip                                           f(1)

       } else {

           apply_deblocking_filter_tip = 0

       }

   }

   if ( TipFrameMode == TIP_FRAME_AS_OUTPUT || bru_inactive || IsBridge ) {




AV2 Specification                                                             Page 105 of 1169
       for ( i = 0 ; i < 3; i++ ) {

           frame_filters_on[ i ] = 0

       }

       if ( bru_inactive || IsBridge ) {

           if ( IsBridge ) {

               tile_info( )

               refIdx = bridge_frame_ref_idx

           } else {

               refIdx = ref_frame_idx[ bru_ref ]

           }

           base_q_idx = RefBaseQIdx[ refIdx ]

           DeltaQUAc = RefDeltaQUAc[ refIdx ]

           DeltaQVAc = RefDeltaQVAc[ refIdx ]

           set_primary_ref_frame_and_ctx( 0 )

       } else if ( apply_deblocking_filter_tip ) {

           tile_info( )

       }

       film_grain_config( )

       if ( bru_inactive || IsBridge ) {

           set_primary_ref_frame_and_ctx( 1 )

       }

       for (row = 0; row < MiRows; row++) {

           for (col = 0; col < MiCols; col++) {

               SegmentIds[ row ][ col ] = 0

           }

       }

       for ( ref = 0; ref < REFS_PER_FRAME; ref++ ) {

           for ( i = 0; i < 6; i++ ) {

               gm_params[ ref ][ i ] = Default_Warp_Params[ i ]

           }

       }

   } else {

       disable_cdf_update                                            f(1)

   }

   if ( bru_inactive || IsBridge ) {

       apply_deblocking_filter[ 0 ] = 0

       apply_deblocking_filter[ 1 ] = 0

       cdef_frame_enable = 0

       for ( plane = 0; plane < NumPlanes; plane++ ) {

           ccso_planes[ plane ] = 0

       }




AV2 Specification                                                 Page 106 of 1169
       FrameRestorationType[ 0 ] = RESTORE_NONE

       FrameRestorationType[ 1 ] = RESTORE_NONE

       FrameRestorationType[ 2 ] = RESTORE_NONE

       gdf_frame_enable = 0

       segmentation_enabled = 0

       for ( i = 0; i < MAX_SEGMENTS; i++ ) {

           for ( j = 0; j < SEG_LVL_MAX; j++ ) {

               FeatureEnabled[ i ][ j ] = 0

               FeatureData[ i ][ j ] = 0

           }

       }

       if ( primary_ref_frame == PRIMARY_REF_NONE ||

           disable_cross_frame_cdf_init) {

           init_coeff_cdfs( )

       }

       return

   }

   if ( use_ref_frame_mvs == 1 ) {

       HasBothRefs = ClosestFuture != NONE && ClosestPast != NONE

       motion_field_estimation( )

       if ( TipFrameMode == TIP_FRAME_AS_OUTPUT ) {

           if ( !enable_tip_explicit_qp ) {

               slot0 = ref_frame_idx[ ClosestPast ]

               slot1 = ref_frame_idx[ ClosestFuture ]

               base_q_idx = Round2(RefBaseQIdx[slot0] + RefBaseQIdx[slot1], 1)

               DeltaQUAc = Round2(RefDeltaQUAc[slot0] + RefDeltaQUAc[slot1], 1)

               DeltaQVAc = Round2(RefDeltaQVAc[slot0] + RefDeltaQVAc[slot1], 1)

           }

           set_primary_ref_frame_and_ctx( 1 )

           for (i = 0; i < MAX_SEGMENTS; i++) {

               for ( j = 0; j < SEG_LVL_MAX; j++ ) {

                   FeatureData[ i ][ j ] = 0

                   FeatureEnabled[ i ][ j ] = 0

               }

           }

           for (row = 0; row < MiRows; row++) {

               for (col = 0; col < MiCols; col++) {

                   PrevSegmentIds[ row ][ col ] = 0

               }

           }

           for ( plane = 0; plane < 3; plane++ ) {




AV2 Specification                                                                 Page 107 of 1169
               ccso_planes[ plane ] = 0

           }

           if ( primary_ref_frame == PRIMARY_REF_NONE ||

               disable_cross_frame_cdf_init ) {

               init_coeff_cdfs( )

           }

       }

       if ( TipFrameMode == TIP_FRAME_DISABLED ) {

           fill_tpl_mvs_sample_gap( )

       }

   }

   if ( TipFrameMode != TIP_FRAME_DISABLED ) {

       setup_tip_motion_field( )

   }

   if ( TipFrameMode == TIP_FRAME_AS_OUTPUT ) {

       return

   }

   tile_info( )

   quantization_params( )

   set_primary_ref_frame_and_ctx( 1 )

   segmentation_params( )

   setup_qm_params( )

   delta_q_params( )

   if ( primary_ref_frame == PRIMARY_REF_NONE ||

       disable_cross_frame_cdf_init ) {

       init_coeff_cdfs( )

   }

   if ( DerivedPrimaryRefFrame != PRIMARY_REF_NONE ) {

       load_previous_segment_ids( )

   }

   CodedLossless = 1

   HasLosslessSegment = 0

   for ( segmentId = 0; segmentId < MaxSegments; segmentId++ ) {

       qindex = get_qindex( 1, segmentId )

       LosslessArray[ segmentId ] = qindex == 0 && delta_q_present == 0 &&

                       DeltaQYDc + BaseYDcDeltaQ <= 0 &&

                       DeltaQUDc + BaseUVDcDeltaQ <= 0 &&

                       DeltaQVDc + BaseUVDcDeltaQ <= 0 &&

                       DeltaQUAc + BaseUVAcDeltaQ <= 0 &&

                       DeltaQVAc + BaseUVAcDeltaQ <= 0

       if ( LosslessArray[ segmentId ] ) {




AV2 Specification                                                            Page 108 of 1169
           HasLosslessSegment = 1

       } else {

           CodedLossless = 0

       }

       if ( using_qmatrix ) {

           if ( LosslessArray[ segmentId ] ) {

               SegQMLevel[ 0 ][ segmentId ] = 15

               SegQMLevel[ 1 ][ segmentId ] = 15

               SegQMLevel[ 2 ][ segmentId ] = 15

           } else {

               qmNum = pic_qm_num_minus_1 + 1

               qmIndexBits = CeilLog2( qmNum )

               qm_index                                           f(qmIndexBits)

               SegQMLevel[ 0 ][ segmentId ] = qm_y[ qm_index ]

               SegQMLevel[ 1 ][ segmentId ] = qm_u[ qm_index ]

               SegQMLevel[ 2 ][ segmentId ] = qm_v[ qm_index ]

           }

       }

   }

   if ( CodedLossless ) {

       allow_tcq = 0

   } else if ( choose_tcq_per_frame ) {

       allow_tcq                                                       f(1)

   } else {

       allow_tcq = enable_tcq

   }

   if ( CodedLossless || !enable_parity_hiding || allow_tcq ) {

       allow_parity_hiding = 0

   } else {

       allow_parity_hiding                                             f(1)

   }

   deblocking_filter_params( )

   gdf_params( )

   cdef_params( )

   lr_params( )

   ccso_params( )

   read_tx_mode( )

   frame_reference_mode( )

   skip_mode_params( )

   if (!FrameIsIntra && enable_bawp) {

       allow_bawp                                                      f(1)




AV2 Specification                                                  Page 109 of 1169
     } else {

         allow_bawp = 0

     }

     if ( !FrameIsIntra && frame_enabled_motion_modes[ DELTAWARP ] ) {

         allow_warpmv_mode                                                                 f(1)

     } else {

         allow_warpmv_mode = 0

     }

     reduced_tx_set                                                                        f(2)

     global_motion_params( )

     film_grain_config( )

 }


where the function reset_qm is defined as:

 reset_qm() {
     for ( level = 0; level < 15; level++ ) {
         if ( obu_type == OBU_SWITCH || obu_type == OBU_RAS_FRAME ) {
              needsReset = QmMLayerId[ level ] == -1 ||
                           MLayerPresenceMap[QmMLayerId[ level ]][obu_mlayer_id]
         } else {
              needsReset = 1
         }
         if ( !QmProtected[ level ] && needsReset ) {
              QmDataPresent[ level ] = 0
              QmNumPlanes[ level ] = NumPlanes
              QmMLayerId[ level ] = -1
              QmTLayerId[ level ] = -1
         }
     }
 }


where the function get_disp_order_hint is defined as:

 get_disp_order_hint( ) {
     if ( obu_type == OBU_CLOSED_LOOP_KEY ||
          ( !is_sef() && FrameType == SWITCH_FRAME &&
            restricted_prediction_switch ) ) {
         return OrderHintLsbs
     }
     maxDisp = get_max_disp_order_hint( 1 )
     dispOrderHint = OrderHintLsbs
     offset = maxDisp - ((1 << OrderHintBits) >> 1) - OrderHintLsbs
     if ( offset >= 0 ) {
         dispOrderHint += ((offset >> OrderHintBits) + 1) << OrderHintBits
     }
     return dispOrderHint
 }


where get_max_disp_order_hint (which returns the maximum order hint from certain frames) is defined
as:

 get_max_disp_order_hint( onlyShowable ) {
     maxDisp = 0
     for ( i = 0; i < NumRefFrames; i++ ) {
         if ( RefValid[i] &&



AV2 Specification                                                                       Page 110 of 1169
                    TLayerDependencyMap[obu_mlayer_id][obu_tlayer_id][RefTLayerId[i]] &&
                    MLayerDependencyMap[obu_mlayer_id][RefMLayerId[i]] &&
                    ( !onlyShowable || RefImplicitOutputFrame[ i ] ||
                      RefImmediateOutputFrame[ i ] ) ) {
                maxDisp = Max( maxDisp, RefOrderHint[i])
          }
      }
      return maxDisp
 }


It is a requirement of bitstream conformance that the value returned from get_disp_order_hint is less
than (1 << (DISPLAY_ORDER_HINT_BITS - 1)).

The function set_primary_ref_frame_and_ctx is defined as:

 set_primary_ref_frame_and_ctx( loadCdfs ) {
     (DerivedPrimaryRefFrame,derivedSecondaryRefFrame) =
         choose_primary_secondary_ref_frame()
     if ( primary_ref_frame == PRIMARY_REF_CHOOSE ) {
         primary_ref_frame = DerivedPrimaryRefFrame
     }
     if ( DerivedPrimaryRefFrame == PRIMARY_REF_NONE ||
           primary_ref_frame == PRIMARY_REF_NONE ) {
         primary_ref_frame = PRIMARY_REF_NONE
         DerivedPrimaryRefFrame = PRIMARY_REF_NONE
         disable_cross_frame_cdf_init = 1
     }
     if ( !loadCdfs ) {
         return
     }
     if ( primary_ref_frame == PRIMARY_REF_NONE ||
           disable_cross_frame_cdf_init ) {
         init_non_coeff_cdfs( )
     } else {
         load_cdfs( ref_frame_idx[ primary_ref_frame ] )
         if ( TipFrameMode != TIP_FRAME_AS_OUTPUT ) {
              blendFrame = (primary_ref_frame == DerivedPrimaryRefFrame) ?
                  derivedSecondaryRefFrame : DerivedPrimaryRefFrame
              if ( enable_avg_cdf && !avg_cdf_type &&
                   blendFrame != PRIMARY_REF_NONE &&
                   !bru_inactive ) {
                  blend_cdfs( ref_frame_idx[ blendFrame ] )
              }
         }
     }
     if ( DerivedPrimaryRefFrame == PRIMARY_REF_NONE ) {
         setup_past_independence( )
     } else {
         load_previous( )
     }
 }


The functions choose_primary_secondary_ref_frame and is_ref_better are defined as:

 choose_primary_secondary_ref_frame() {
     if ( FrameIsIntra || FrameType == SWITCH_FRAME ) {
         return (PRIMARY_REF_NONE, PRIMARY_REF_NONE)
     }
     primary = PRIMARY_REF_NONE
     primaryQpDiff = 512
     secondary = PRIMARY_REF_NONE
     secondaryQpDiff = 512
     primaryD = 0
     secondaryD = 0
     primaryRatio = 0




AV2 Specification                                                                           Page 111 of 1169
      secondaryRatio = 0
      for ( i = 0; i < NumTotalRefs; i++ ) {
          idx = ref_frame_idx[ i ]
          if ( RefFrameType[ idx ] == INTER_FRAME && first_slot_with_ref(idx) &&
                RefOrderHint[idx] != RESTRICTED_OH ) {
              q = RefBaseQIdx[ idx ]
              d = RefOrderHint[ idx ]
              dRatio = FloorLog2( RefFrameWidth[ idx ] * RefFrameHeight[ idx ] )
              qpDiff = Abs(q - base_q_idx)
              if ( (qpDiff < primaryQpDiff) ||
                    (qpDiff == primaryQpDiff &&
                        is_ref_better(d,primaryD,dRatio,primaryRatio)) ) {
                   secondary = primary
                   secondaryQpDiff = primaryQpDiff
                   secondaryD = primaryD
                   secondaryRatio = primaryRatio
                   primary = i
                   primaryQpDiff = qpDiff
                   primaryD = d
                   primaryRatio = dRatio
              } else if ((qpDiff < secondaryQpDiff) ||
                      (qpDiff == secondaryQpDiff &&
                          is_ref_better(d,secondaryD,dRatio,secondaryRatio))) {
                   secondary = i
                   secondaryQpDiff = qpDiff
                   secondaryD = d
                   secondaryRatio = dRatio
              }
          }
      }
      if ( signal_primary_ref_frame ) {
          if ( primary_ref_frame == PRIMARY_REF_NONE ) {
              primary = PRIMARY_REF_NONE
              secondary = PRIMARY_REF_NONE
          } else if ( primary_ref_frame != primary ) {
              if ( secondary == PRIMARY_REF_NONE ||
                    secondary == primary_ref_frame ) {
                   secondary = primary
              }
              primary = primary_ref_frame
          }
      }
      return (primary,secondary)
 }

 is_ref_better(refDisp, bestDisp, refRatio, bestRatio) {
     d0 = Abs(get_relative_dist(OrderHint,refDisp)) - (refRatio << 1)
     d1 = Abs(get_relative_dist(OrderHint,bestDisp)) - (bestRatio << 1)
     if (d0 < d1) {
         return 1
     }
     if (d0 == d1 && get_relative_dist(refDisp,bestDisp) > 0) {
         return 1
     }
     return 0
 }


The function long_term_id_in_use (which determines if longTermId is present in the ref_long_term_id
array) is defined as:


 long_term_id_in_use( longTermId ) {
     for ( j = 0; j < num_key_ref_frames; j++ ) {
         if ( longTermId == ref_long_term_id[ j ] ) {
             return 1
         }




AV2 Specification                                                                         Page 112 of 1169
                 }
                 return 0
     }


```

<a id="s-5-18-3"></a>

#### § 5.18.3 Frame configuration structures

```text
§   5.18.3. Frame configuration structures

```

<a id="s-5-18-3-1"></a>

##### § 5.18.3.1 Get relative distance function

```text
§   5.18.3.1. Get relative distance function

    This function computes the distance between two order hints by sign extending the result of subtracting
    the values.

     get_relative_dist( a, b ) {
         if ( a == RESTRICTED_OH && b == RESTRICTED_OH ) {
             return 0
         } else if ( a == RESTRICTED_OH ) {
             return 127
         } else if ( b == RESTRICTED_OH ) {
             return -127
         } else {
             return Clip3( -127, 127, a - b )
         }
     }


```

<a id="s-5-18-3-2"></a>

##### § 5.18.3.2 Frame optical flow refine type syntax

```text
§   5.18.3.2. Frame optical flow refine type syntax

     frame_opfl_refine_type( ) {                                                               Descriptor

         if ( TipFrameMode == TIP_FRAME_AS_OUTPUT ) {

             opfl_refine_type = ( !enable_tip_refinemv ||

                        enable_opfl_refine == REFINE_NONE ) ?

                            REFINE_NONE : REFINE_ALL

         } else if ( enable_opfl_refine == REFINE_AUTO ) {

             opfl_refine_type                                                                     f(1)

             if ( opfl_refine_type != REFINE_SWITCHABLE ) {

                 opfl_refine_all                                                                  f(1)

                 opfl_refine_type = opfl_refine_all ? REFINE_ALL : REFINE_NONE

             }

         } else {

             opfl_refine_type = enable_opfl_refine

         }

     }


```

<a id="s-5-18-3-3"></a>

##### § 5.18.3.3 Screen content params syntax

```text
§   5.18.3.3. Screen content params syntax

     screen_content_params( ) {                                                                Descriptor

         if ( seq_force_screen_content_tools == SELECT_SCREEN_CONTENT_TOOLS ) {

             allow_screen_content_tools                                                           f(1)

         } else {

             allow_screen_content_tools = seq_force_screen_content_tools

         }




    AV2 Specification                                                                          Page 113 of 1169
         if ( allow_screen_content_tools ) {

             if ( seq_force_integer_mv == SELECT_INTEGER_MV ) {

                 force_integer_mv                                         f(1)

             } else {

                 force_integer_mv = seq_force_integer_mv

             }

         } else {

             force_integer_mv = 0

         }

     }


```

<a id="s-5-18-3-4"></a>

##### § 5.18.3.4 Intra block copy params syntax

```text
§   5.18.3.4. Intra block copy params syntax

     intrabc_params( ) {                                               Descriptor

         allow_intrabc                                                    f(1)

         if ( allow_intrabc ) {

             if ( FrameIsIntra ) {

                 allow_global_intrabc                                     f(1)

                 if ( allow_global_intrabc ) {

                     allow_local_intrabc                                  f(1)

                 } else {

                     allow_local_intrabc = 1

                 }

             } else {

                 allow_global_intrabc = 0

                 allow_local_intrabc = 1

             }

             max_bvp_drl_bits_minus_1 = seq_max_bvp_drl_bits_minus_1

             if ( allow_frame_max_bvp_drl_bits ) {

                 change_bvp_drl                                           f(1)

                 if ( change_bvp_drl ) {

                     max_bvp_drl_bits_minus_1                             ns(2)

                     if ( max_bvp_drl_bits_minus_1 >=

                          seq_max_bvp_drl_bits_minus_1 ) {

                         max_bvp_drl_bits_minus_1 += 1

                     }

                 }

             }

         }

     }




    AV2 Specification                                                  Page 114 of 1169
```

<a id="s-5-18-4"></a>

#### § 5.18.4 Frame size structures

```text
§   5.18.4. Frame size structures

```

<a id="s-5-18-4-1"></a>

##### § 5.18.4.1 Frame size syntax

```text
§   5.18.4.1. Frame size syntax

     frame_size( ) {                                                    Descriptor

         if ( frame_size_override_flag ) {

             n = frame_width_bits_minus_1 + 1

             frame_width_minus_1                                           f(n)

             n = frame_height_bits_minus_1 + 1

             frame_height_minus_1                                          f(n)

             FrameWidth = frame_width_minus_1 + 1

             FrameHeight = frame_height_minus_1 + 1

         } else {

             FrameWidth = mfh_frame_width_minus_1[ cur_mfh_id ] + 1

             FrameHeight = mfh_frame_height_minus_1[ cur_mfh_id ] + 1

         }

         compute_image_size( )

     }


```

<a id="s-5-18-4-2"></a>

##### § 5.18.4.2 Frame size with bridge syntax

```text
§   5.18.4.2. Frame size with bridge syntax

     frame_size_with_bridge( ) {                                        Descriptor

         n = frame_width_bits_minus_1 + 1

         bridge_frame_width_minus_1                                        f(n)

         n = frame_height_bits_minus_1 + 1

         bridge_frame_height_minus_1                                       f(n)

         FrameWidth = Min( RefFrameWidth[ bridge_frame_ref_idx ],

                     bridge_frame_width_minus_1 + 1 )

         FrameHeight = Min( RefFrameHeight[ bridge_frame_ref_idx ],

                     bridge_frame_height_minus_1 + 1 )

         compute_image_size( )

     }


```

<a id="s-5-18-4-3"></a>

##### § 5.18.4.3 Frame size with refs syntax

```text
§   5.18.4.3. Frame size with refs syntax

     frame_size_with_refs( ) {                                          Descriptor

         for ( i = 0; i < NumTotalRefs; i++ ) {

             found_ref                                                     f(1)

             if ( found_ref == 1 ) {

                 FrameWidth = RefFrameWidth[ ref_frame_idx[ i ] ]

                 FrameHeight = RefFrameHeight[ ref_frame_idx[ i ] ]

                 break

             }

         }




    AV2 Specification                                                   Page 115 of 1169
         if ( NumTotalRefs == 0 || found_ref == 0 ) {

             frame_size( )

         } else {

             compute_image_size( )

         }

     }


```

<a id="s-5-18-4-4"></a>

##### § 5.18.4.4 Compute image size function

```text
§   5.18.4.4. Compute image size function

     compute_image_size( ) {
         MiCols = 2 * ( ( FrameWidth + 7 ) >> 3 )
         MiRows = 2 * ( ( FrameHeight + 7 ) >> 3 )
         maxFrameWidth = max_frame_width_minus_1 + 1
         maxFrameHeight = max_frame_height_minus_1 + 1
         CropLeft = (seq_cropping_win_left_offset * FrameWidth) / maxFrameWidth
         cropRight = FrameWidth - ((seq_cropping_win_right_offset * FrameWidth) /
                                   maxFrameWidth)
         CropTop = (seq_cropping_win_top_offset * FrameHeight) / maxFrameHeight
         cropBottom = FrameHeight - ((seq_cropping_win_bottom_offset * FrameHeight) /
                                     maxFrameHeight)
         CropWidth = cropRight - CropLeft
         CropHeight = cropBottom - CropTop
     }


```

<a id="s-5-18-5"></a>

#### § 5.18.5 Filtering structures

```text
§   5.18.5. Filtering structures

```

<a id="s-5-18-5-1"></a>

##### § 5.18.5.1 Interpolation filter syntax

```text
§   5.18.5.1. Interpolation filter syntax

     read_interpolation_filter( ) {                                                     Descriptor

         is_filter_switchable                                                              f(1)

         if ( is_filter_switchable == 1 ) {

             interpolation_filter = SWITCHABLE

         } else {

             interpolation_filter                                                          f(2)

         }

     }


```

<a id="s-5-18-5-2"></a>

##### § 5.18.5.2 Deblocking filter params syntax

```text
§   5.18.5.2. Deblocking filter params syntax

     deblocking_filter_params( ) {                                                      Descriptor

         if ( CodedLossless ) {

             apply_deblocking_filter[ 0 ] = 0

             apply_deblocking_filter[ 1 ] = 0

             return

         }

         if ( enable_df_sub_pu && FrameType == INTER_FRAME ) {

             allow_df_sub_pu                                                               f(1)

         } else {

             allow_df_sub_pu = 0




    AV2 Specification                                                                   Page 116 of 1169
     }

     if ( mfh_deblocking_filter_update[ cur_mfh_id ] ) {

         apply_deblocking_filter[ 0 ] = mfh_apply_deblocking_filter[ cur_mfh_id ][ 0 ]

         apply_deblocking_filter[ 1 ] = mfh_apply_deblocking_filter[ cur_mfh_id ][ 1 ]

         apply_deblocking_filter[ 2 ] = 0

         apply_deblocking_filter[ 3 ] = 0

         if ( NumPlanes > 1 ) {

             if ( apply_deblocking_filter[0] || apply_deblocking_filter[1] ) {

                 apply_deblocking_filter[2] = mfh_apply_deblocking_filter[cur_mfh_id][2]

                 apply_deblocking_filter[3] = mfh_apply_deblocking_filter[cur_mfh_id][3]

             }

         }

     } else {

         apply_deblocking_filter[ 0 ]                                                          f(1)

         apply_deblocking_filter[ 1 ]                                                          f(1)

         apply_deblocking_filter[ 2 ] = 0

         apply_deblocking_filter[ 3 ] = 0

         if ( NumPlanes > 1 ) {

             if ( apply_deblocking_filter[ 0 ] || apply_deblocking_filter[ 1 ] ) {

                 apply_deblocking_filter[ 2 ]                                                  f(1)

                 apply_deblocking_filter[ 3 ]                                                  f(1)

             }

         }

     }

     for ( i = 0; i < 4; i++ ) {

         if ( apply_deblocking_filter[ i ] ) {

             df_delta_q_present[ i ]                                                           f(1)

             if ( df_delta_q_present[ i ] ) {

                 dfParBits = df_par_bits_minus_2 + 2

                 df_delta_q[ i ]                                                           f(dfParBits)

                 DfDeltaQ[ i ] = df_delta_q[ i ] - ( 1 << (dfParBits - 1) )

             } else {

                 DfDeltaQ[ i ] = (i == 1) ? DfDeltaQ[ 0 ] : 0

             }

         } else {

             DfDeltaQ[ i ] = 0

         }

     }

 }




AV2 Specification                                                                          Page 117 of 1169
```

<a id="s-5-18-6"></a>

#### § 5.18.6 Quantization structures

```text
§   5.18.6. Quantization structures

```

<a id="s-5-18-6-1"></a>

##### § 5.18.6.1 Quantization params syntax

```text
§   5.18.6.1. Quantization params syntax

     quantization_params( ) {                                                      Descriptor

       n = BitDepth == 8 ? 8 : 9

       base_q_idx                                                                     f(n)

       DeltaQYDc = 0

       DeltaQUDc = 0

       DeltaQUAc = 0

       DeltaQVDc = 0

       DeltaQVAc = 0

       if ( TipFrameMode != TIP_FRAME_AS_OUTPUT && y_dc_delta_q_enabled ) {

           DeltaQYDc = read_delta_q( )

       }

       if ( NumPlanes > 1 && (

               uv_ac_delta_q_enabled ||

               (TipFrameMode != TIP_FRAME_AS_OUTPUT && uv_dc_delta_q_enabled)

               ) ) {

           if ( separate_uv_delta_q ) {

               diff_uv_delta                                                          f(1)

           } else {

               diff_uv_delta = 0

           }

           if ( TipFrameMode != TIP_FRAME_AS_OUTPUT && uv_dc_delta_q_enabled ) {

               DeltaQUDc = read_delta_q( )

           }

           if ( uv_ac_delta_q_enabled ) {

               DeltaQUAc = read_delta_q( )

           }

           if ( equal_ac_dc_q ) {

               DeltaQUDc = DeltaQUAc

           }

           if ( diff_uv_delta ) {

               if ( TipFrameMode != TIP_FRAME_AS_OUTPUT &&

                   uv_dc_delta_q_enabled ) {

                   DeltaQVDc = read_delta_q( )

               }

               if ( uv_ac_delta_q_enabled ) {

                   DeltaQVAc = read_delta_q( )

               }

               if ( equal_ac_dc_q ) {




    AV2 Specification                                                              Page 118 of 1169
                     DeltaQVDc = DeltaQVAc

                 }

             } else {

                 DeltaQVDc = DeltaQUDc

                 DeltaQVAc = DeltaQUAc

             }

         }

     }


```

<a id="s-5-18-6-2"></a>

##### § 5.18.6.2 Setup QM params syntax

```text
§   5.18.6.2. Setup QM params syntax

     setup_qm_params( ) {                                Descriptor

         using_qmatrix                                      f(1)

         if ( using_qmatrix ) {

             if ( segmentation_enabled ) {

                 pic_qm_num_minus_1                         f(2)

             } else {

                 pic_qm_num_minus_1 = 0

             }

             qmNum = pic_qm_num_minus_1 + 1

             for ( i = 0; i < qmNum; i++ ) {

                 qm_y[ i ]                                  f(4)

                 if ( NumPlanes > 1 ) {

                     qm_uv_same_as_y                        f(1)

                     if ( qm_uv_same_as_y ) {

                         qm_u[ i ] = qm_y [ i ]

                         qm_v[ i ] = qm_y [ i ]

                     } else {

                         qm_u[ i ]                          f(4)

                         if ( !separate_uv_delta_q ) {

                             qm_v[ i ] = qm_u[ i ]

                         } else {

                             qm_v[ i ]                      f(4)

                         }

                     }

                 }

             }

         }

     }


```

<a id="s-5-18-6-3"></a>

##### § 5.18.6.3 Delta quantizer syntax

```text
§   5.18.6.3. Delta quantizer syntax

     read_delta_q( ) {                                   Descriptor




    AV2 Specification                                    Page 119 of 1169
         delta_coded                                                                       f(1)

         if ( delta_coded ) {

             delta_q                                                                       su(7)

         } else {

             delta_q = 0

         }

         return delta_q

     }


```

<a id="s-5-18-7"></a>

#### § 5.18.7 Segmentation and tiling structures

```text
§   5.18.7. Segmentation and tiling structures

```

<a id="s-5-18-7-1"></a>

##### § 5.18.7.1 Segmentation params syntax

```text
§   5.18.7.1. Segmentation params syntax

     segmentation_params( ) {                                                           Descriptor

         segmentation_enabled                                                              f(1)

         if ( segmentation_enabled == 1 ) {

             if ( cur_mfh_id > 0 && mfh_seg_info_present_flag[ cur_mfh_id ] ) {

                 haveSegParams = mfh_ext_seg_flag[ cur_mfh_id ] == enable_ext_seg

                 allowChange = haveSegParams && mfh_allow_seg_info_change[cur_mfh_id]

                 mfhId = cur_mfh_id

             } else if ( seq_seg_info_present_flag ) {

                 haveSegParams = 1

                 allowChange = seq_allow_seg_info_change

                 mfhId = 0

             } else {

                 haveSegParams = 0

                 allowChange = 0

             }

             if ( allowChange ) {

                 reuse_seg_info                                                            f(1)

             } else {

                 reuse_seg_info = haveSegParams

             }

             if ( reuse_seg_info ) {

                 for ( i = 0; i < MAX_SEGMENTS; i++ ) {

                  for ( j = 0; j < SEG_LVL_MAX; j++ ) {

                    if ( mfhId == 0 ) {

                      FeatureData[ i ][ j ] = SeqFeatureData[ i ][ j ]

                      FeatureEnabled[ i ][ j ] = SeqFeatureEnabled[ i ][ j ]

                    } else {

                      FeatureData[ i ][ j ] =

                        MfhFeatureData[ mfhId ][ i ][ j ]

                      FeatureEnabled[ i ][ j ] =



    AV2 Specification                                                                   Page 120 of 1169
                         MfhFeatureEnabled[ mfhId ][ i ][ j ]

                     }

                 }

             }

         } else {

             (FeatureEnabled, FeatureData) = seg_info( MaxSegments )

         }

         if ( DerivedPrimaryRefFrame == PRIMARY_REF_NONE ) {

             segmentation_update_map = 1

             segmentation_temporal_update = 0

         } else {

             segmentation_update_map                                                 f(1)

             if ( segmentation_update_map == 1 && FrameType != SWITCH_FRAME ) {

                 segmentation_temporal_update                                        f(1)

             } else {

                 segmentation_temporal_update = 0

             }

         }

     } else {

         for ( i = 0; i < MAX_SEGMENTS; i++ ) {

             for ( j = 0; j < SEG_LVL_MAX; j++ ) {

                 FeatureEnabled[ i ][ j ] = 0

                 FeatureData[ i ][ j ] = 0

             }

         }

     }

     SegIdPreSkip = 0

     LastActiveSegId = 0

     for ( i = 0; i < MaxSegments; i++ ) {

         for ( j = 0; j < SEG_LVL_MAX; j++ ) {

             if ( FeatureEnabled[ i ][ j ] ) {

                 LastActiveSegId = i

                 if ( j >= SEG_LVL_SKIP ) {

                     SegIdPreSkip = 1

                 }

             }

         }

     }

 }




AV2 Specification                                                                 Page 121 of 1169
    The constant lookup tables used in this syntax are defined as:

     Segmentation_Feature_Bits[ SEG_LVL_MAX ]   = { 9, 0, 0 }
     Segmentation_Feature_Signed[ SEG_LVL_MAX ] = { 1, 0, 0 }
     Segmentation_Feature_Max[ SEG_LVL_MAX ] = { MAXQ_BITS, 0, 0 }


```

<a id="s-5-18-7-2"></a>

##### § 5.18.7.2 Tile info syntax

```text
§   5.18.7.2. Tile info syntax

     tile_info ( ) {                                                                Descriptor

       sb4x4 = Num_4x4_Blocks_Wide[ SbSize ]

       sbShift = Mi_Width_Log2[ SbSize ]

       sbCols = ( MiCols + sb4x4 - 1 ) >> sbShift

       sbRows = ( MiRows + sb4x4 - 1 ) >> sbShift

       if ( IsBridge ) {

           haveTileParams = 0

       } else {

           haveTileParams = seq_tile_info_present_flag

       }

       if ( haveTileParams &&

           ( SeqUniformTileSpacingFlag ? (

               uniform_eligible( SeqTileRowsLog2, sbRows) &&

               uniform_eligible( SeqTileColsLog2, sbCols) ) :

               ( SeqSbCols == sbCols && SeqSbRows == sbRows ) ) ) {

           if ( allow_tile_info_change ) {

               reuse_tile_info                                                         f(1)

           } else {

               reuse_tile_info = 1

           }

       } else {

           reuse_tile_info = 0

       }

       seqSbSize = get_seq_sb_size()

       if ( reuse_tile_info ) {

           ( sbRowStarts, TileRows, TileRowsLog2, sbColStarts, TileCols,

           TileColsLog2, sbShift2) = reuse_tile_params(SeqUniformTileSpacingFlag,

               SeqSbRowStarts, SeqTileRows, SeqTileRowsLog2,

               SeqSbColStarts, SeqTileCols, SeqTileColsLog2, seqSbSize, SbSize )

       } else {

           ( sbRowStarts, sbRows, TileRows, TileRowsLog2, sbColStarts, sbCols,

           TileCols, TileColsLog2, uniformSpacing, sbShift2) = tile_params(

               FrameWidth, FrameHeight, seqSbSize, SbSize, IsBridge )

       }

       for ( i = 0; i < TileCols; i++ ) {




    AV2 Specification                                                               Page 122 of 1169
             MiColStarts[ i ] = sbColStarts[ i ] << sbShift2

         }

         for ( i = 0; i < TileRows; i++ ) {

             MiRowStarts[ i ] = sbRowStarts[ i ] << sbShift2

         }

         MiColStarts[ TileCols ] = MiCols

         MiRowStarts[ TileRows ] = MiRows

         if ( (TileCols > 1 || TileRows > 1) && !IsBridge &&

             TipFrameMode != TIP_FRAME_AS_OUTPUT ) {

             if ( !enable_avg_cdf || !avg_cdf_type ) {

                 n = TileRowsLog2 + TileColsLog2

                 context_update_tile_id                                                   f(n)

             }

             tile_size_bytes_minus_1                                                      f(2)

             TileSizeBytes = tile_size_bytes_minus_1 + 1

         } else {

             context_update_tile_id = 0

         }

     }


    where uniform_eligible is specified as:

     uniform_eligible( tileLog2, sbNum ) {
         tileNum = 1 << tileLog2
         tileWidth = (sbNum + tileNum - 1) >> tileLog2
         lastTileWidth = sbNum - (tileNum - 1) * tileWidth
         return tileWidth >= 1 && lastTileWidth >= 1
     }


```

<a id="s-5-18-7-3"></a>

##### § 5.18.7.3 Tile params syntax

```text
§   5.18.7.3. Tile params syntax

     tile_params( frameWidth, frameHeight, uniformSbSize, sbSize, isBridge ) {         Descriptor

         miCols = 2 * ( ( frameWidth + 7 ) >> 3 )

         miRows = 2 * ( ( frameHeight + 7 ) >> 3 )

         sb4x4 = Num_4x4_Blocks_Wide[ sbSize ]

         sbShift = Mi_Width_Log2[ sbSize ]

         sbCols = ( miCols + sb4x4 - 1 ) >> sbShift

         sbRows = ( miRows + sb4x4 - 1 ) >> sbShift

         if ( seq_level_idx != 31 ) {

             maxTileWidthSb = ( Tile_Width_Scaling_Factor[seq_tier][seq_level_idx] *

                       MAX_TILE_WIDTH ) >> (sbShift + 4)

             maxTileAreaSb = ( Tile_Area_Scaling_Factor[seq_tier][seq_level_idx] *

                      MAX_TILE_AREA ) >> ( 2 * (sbShift + 2) + 2 )

         } else {




    AV2 Specification                                                                  Page 123 of 1169
       maxTileWidthSb = sbCols

       maxTileAreaSb = sbCols * sbRows

   }

   minLog2TileCols = tile_log2(maxTileWidthSb, sbCols)

   maxLog2TileCols = tile_log2(1, Min(sbCols, MAX_TILE_COLS))

   maxLog2TileRows = tile_log2(1, Min(sbRows, MAX_TILE_ROWS))

   minLog2Tiles = Max( minLog2TileCols,

                    tile_log2(maxTileAreaSb, sbRows * sbCols))

   if ( isBridge ) {

       uniform_tile_spacing_flag = 1

   } else {

       uniform_tile_spacing_flag                                             f(1)

   }

   if ( uniform_tile_spacing_flag ) {

       sbShift = Mi_Width_Log2[ uniformSbSize ]

       tileColsLog2 = minLog2TileCols

       if ( !isBridge ) {

           while ( tileColsLog2 < maxLog2TileCols ) {

               increment_tile_cols_log2                                      f(1)

               if ( increment_tile_cols_log2 == 1 ) {

                   tileColsLog2 += 1

               } else {

                   break

               }

           }

       }

       (sbColStarts, tileCols) = uniform_spacing( tileColsLog2, miCols,

                               uniformSbSize )

       tileColsLog2 = tile_log2(1, tileCols)

       minLog2TileRows = Max( minLog2Tiles - tileColsLog2, 0)

       tileRowsLog2 = minLog2TileRows

       if ( !isBridge ) {

           while ( tileRowsLog2 < maxLog2TileRows ) {

               increment_tile_rows_log2                                      f(1)

               if ( increment_tile_rows_log2 == 1 ) {

                   tileRowsLog2++

               } else {

                   break

               }

           }

       }




AV2 Specification                                                         Page 124 of 1169
             (sbRowStarts, tileRows) = uniform_spacing( tileRowsLog2, miRows,

                                 uniformSbSize )

         } else {

             widestTileSb = 1

             startSb = 0

             for ( i = 0; startSb < sbCols; i++ ) {

                 sbColStarts[ i ] = startSb

                 n = Min(sbCols - startSb, maxTileWidthSb)

                 width_in_sbs_minus_1                                                                     ns(n)

                 sizeSb = width_in_sbs_minus_1 + 1

                 widestTileSb = Max( sizeSb, widestTileSb )

                 startSb += sizeSb

             }

             tileCols = i

             tileColsLog2 = tile_log2(1, tileCols)

             if (minLog2Tiles > 0) {

                 maxTileAreaSb = (sbRows * sbCols) >> (minLog2Tiles + 1)

             } else {

                 maxTileAreaSb = sbRows * sbCols

             }

             maxTileHeightSb = Max( maxTileAreaSb / widestTileSb, 1 )

             startSb = 0

             for ( i = 0; startSb < sbRows; i++ ) {

                 sbRowStarts[ i ] = startSb

                 maxHeight = Min(sbRows - startSb, maxTileHeightSb)

                 height_in_sbs_minus_1                                                                ns(maxHeight)

                 sizeSb = height_in_sbs_minus_1 + 1

                 startSb = startSb + sizeSb

             }

             tileRows = i

         }

         tileRowsLog2 = tile_log2(1, tileRows)

         return ( sbRowStarts, sbRows, tileRows, tileRowsLog2, sbColStarts, sbCols,

                 tileCols, tileColsLog2, uniform_tile_spacing_flag, sbShift)

     }


```

<a id="s-5-18-7-4"></a>

##### § 5.18.7.4 Reuse tile params function

```text
§   5.18.7.4. Reuse tile params function

     reuse_tile_params( uniformSpacing, sbRowStarts, tileRows, tileRowsLog2, sbColStarts, tileCols,
     tileColsLog2, seqSbSize, sbSize ) {
         if ( uniformSpacing ) {
             sbShift = Mi_Width_Log2[ seqSbSize ]
             (unifSbColStarts, tileCols) = uniform_spacing( tileColsLog2, MiCols,
                                                            seqSbSize )
             (unifSbRowStarts, tileRows) = uniform_spacing( tileRowsLog2, MiRows,



    AV2 Specification                                                                                 Page 125 of 1169
                                                             seqSbSize )
              tileColsLog2 = tile_log2(1, tileCols)
              tileRowsLog2 = tile_log2(1, tileRows)
              return ( unifSbRowStarts, tileRows, tileRowsLog2, unifSbColStarts,
                       tileCols, tileColsLog2, sbShift)
          } else {
              sbShift = Mi_Width_Log2[ sbSize ]
              tileColsLog2 = tile_log2(1, tileCols)
              tileRowsLog2 = tile_log2(1, tileRows)
              return ( sbRowStarts, tileRows, tileRowsLog2, sbColStarts, tileCols,
                       tileColsLog2, sbShift)
          }
     }


```

<a id="s-5-18-7-5"></a>

##### § 5.18.7.5 Uniform spacing function

```text
§   5.18.7.5. Uniform spacing function

     uniform_spacing( tileLog2, mis, sbSize ) {
         sb4x4 = Num_4x4_Blocks_Wide[ sbSize ]
         sbShift = Mi_Width_Log2[ sbSize ]
         sbs = ( mis + sb4x4 - 1 ) >> sbShift
         fullSbs = mis >> sbShift
         tileSb = fullSbs >> tileLog2
         if ( tileSb == 0 ) {
             extraSbs = sbs
         } else {
             extraSbs = fullSbs - (tileSb << tileLog2)
         }
         startSb = 0
         for (i = 0; i < (1 << tileLog2) && startSb < sbs; i++) {
             sbStarts[ i ] = startSb
             startSb += tileSb
             if (i < extraSbs) {
                  startSb += 1
             }
         }
         return (sbStarts, i)
     }


```

<a id="s-5-18-7-6"></a>

##### § 5.18.7.6 Get sequence superblock size function

```text
§   5.18.7.6. Get sequence superblock size function

     get_seq_sb_size() {
         if ( use_256x256_superblock ) {
             return BLOCK_256X256
         } else if ( use_128x128_superblock ) {
             return BLOCK_128X128
         } else {
             return BLOCK_64X64
         }
     }


```

<a id="s-5-18-7-7"></a>

##### § 5.18.7.7 Tile size calculation function

```text
§   5.18.7.7. Tile size calculation function

    tile_log2 returns the smallest value for k such that blkSize << k is greater than or equal to target.

     tile_log2( blkSize, target ) {
         for ( k = 0; (blkSize << k) < target; k++ ) {
         }
         return k
     }




    AV2 Specification                                                                               Page 126 of 1169
```

<a id="s-5-18-7-8"></a>

##### § 5.18.7.8 Quantizer index delta parameters syntax

```text
§   5.18.7.8. Quantizer index delta parameters syntax

     delta_q_params( ) {                                            Descriptor

         delta_q_res = 0

         delta_q_present = 0

         if ( base_q_idx > 0 ) {

             delta_q_present                                           f(1)

         }

         if ( delta_q_present ) {

             delta_q_res                                               f(2)

         }

     }


```

<a id="s-5-18-7-9"></a>

##### § 5.18.7.9 GDF params syntax

```text
§   5.18.7.9. GDF params syntax

     gdf_params( ) {                                                Descriptor

         if ( CodedLossless || !enable_gdf ) {

             gdf_frame_enable = 0

         } else {

             if ( single_picture_header_flag ) {

                 gdf_frame_enable = 1

             } else {

                 gdf_frame_enable                                      f(1)

             }

             if ( !gdf_frame_enable ) {

                 return

             }

             gdfBlkSize = Max(Block_Width[ SbSize ],GDF_MIN_SIZE)

             if ( gdf_unit_matches_sb_size ) {

                 gdfBlkSize = Block_Width[ SbSize ]

             } else if ( SbSize == BLOCK_64X64 ) {

                 a = 0

                 for ( i = 0; i < TileCols; i++ ) {

                     a = a | MiColStarts[ i ]

                 }

                 for ( i = 0; i < TileRows; i++ ) {

                     a = a | MiRowStarts[ i ]

                 }

                 if ( a & 16 ) {

                     gdfBlkSize = 64

                 }

             }

             GdfBlkSize = gdfBlkSize




    AV2 Specification                                               Page 127 of 1169
             if ( MiCols * MI_SIZE > gdfBlkSize ||

                 MiRows * MI_SIZE > gdfBlkSize ||

                 ( disable_loopfilters_across_tiles &&

                  (TileRows > 1 || TileCols > 1) ) ) {

                 gdf_per_block                                              f(1)

             } else {

                 gdf_per_block = 0

             }

             gdf_pic_qc_idx                                                 f(2)

             gdf_pic_scale_idx                                              f(2)

             GdfPixScale = 1 + gdf_pic_scale_idx

         }

     }


```

<a id="s-5-18-7-10"></a>

##### § 5.18.7.10 CDEF params syntax

```text
§   5.18.7.10. CDEF params syntax

     cdef_params( ) {                                                    Descriptor

         if ( CodedLossless ||

             !enable_cdef ) {

             cdef_frame_enable = 0

             return

         }

         if ( single_picture_header_flag ) {

             cdef_frame_enable = 1

         } else {

             cdef_frame_enable                                              f(1)

         }

         if ( !cdef_frame_enable ) {

             return

         }

         cdef_damping_minus_3                                               f(2)

         CdefDamping = cdef_damping_minus_3 + 3

         cdef_strengths_minus_1                                             f(3)

         CdefStrengths = cdef_strengths_minus_1 + 1

         if ( CdefOnSkipTxfm == CDEF_ON_SKIP_TXFM_ADAPTIVE ) {

             cdef_on_skip_txfm_frame_enable                                 f(1)

         } else if ( CdefOnSkipTxfm == CDEF_ON_SKIP_TXFM_ALWAYS_ON ) {

             cdef_on_skip_txfm_frame_enable = 1

         } else {

             cdef_on_skip_txfm_frame_enable = 0

         }

         for ( i = 0; i < CdefStrengths; i++ ) {




    AV2 Specification                                                    Page 128 of 1169
             cdef_y_pri_zero                                  f(1)

             if ( cdef_y_pri_zero ) {

                 cdef_y_pri_strength[ i ] = 0

             } else {

                 cdef_y_pri_strength[ i ]                     f(4)

             }

             cdef_y_sec_strength[ i ]                         f(2)

             if ( cdef_y_sec_strength[ i ] == 3 ) {

                 cdef_y_sec_strength[ i ] += 1

             }

             if ( NumPlanes > 1 ) {

                 cdef_uv_pri_zero                             f(1)

                 if ( cdef_uv_pri_zero ) {

                     cdef_uv_pri_strength[ i ] = 0

                 } else {

                     cdef_uv_pri_strength[ i ]                f(4)

                 }

                 cdef_uv_sec_strength[ i ]                    f(2)

                 if ( cdef_uv_sec_strength[ i ] == 3 ) {

                     cdef_uv_sec_strength[ i ] += 1

                 }

             }

         }

     }


```

<a id="s-5-18-7-11"></a>

##### § 5.18.7.11 Loop restoration params syntax

```text
§   5.18.7.11. Loop restoration params syntax

     lr_params( ) {                                        Descriptor

         if ( CodedLossless || !enable_restoration ) {

             FrameRestorationType[ 0 ] = RESTORE_NONE

             FrameRestorationType[ 1 ] = RESTORE_NONE

             FrameRestorationType[ 2 ] = RESTORE_NONE

             UsesLr = 0

             for ( i = 0; i < 3; i++ ) {

                 frame_filters_on[ i ] = 0

             }

             return

         }

         usesLumaLr = 0

         usesChromaLr = 0

         for ( plane = 0; plane < NumPlanes; plane++ ) {

             toolsCount = 1




    AV2 Specification                                      Page 129 of 1169
     indexToTool[ 0 ] = RESTORE_NONE

     for ( i = 1; i < RESTORE_SWITCHABLE_TYPES; i++ ) {

         if ( !lr_tools_disable[ plane > 0 ][ i ] ) {

             indexToTool[ toolsCount ] = i

             toolsCount += 1

         }

     }

     indexToTool[ toolsCount ] = RESTORE_SWITCHABLE

     allowSwitchable = (toolsCount > 2)

     n = toolsCount + allowSwitchable

     tool_index                                                                ns(n)

     FrameRestorationType[ plane ] = indexToTool[ tool_index ]

     if ( FrameRestorationType[ plane ] != RESTORE_NONE ) {

         if ( plane == 0 ) {

             usesLumaLr = 1

         } else {

             usesChromaLr = 1

         }

     }

     r = FrameRestorationType[ plane ]

     if ( plane == 0 ) {

         NumFilterClasses = 1

     }

     frame_filters_on[ plane ] = 0

     temporal_pred_flag[ plane ] = 0

     if ( r == RESTORE_WIENER_NONSEP || r == RESTORE_SWITCHABLE ) {

         frame_filters_on[ plane ]                                             f(1)

         if ( frame_filters_on[ plane ] ) {

             numRefFrames = (FrameIsIntra || FrameType == SWITCH_FRAME) ?

                 0 : NumTotalRefs

             if ( numRefFrames > 0 ) {

                 temporal_pred_flag[ plane ]                                   f(1)

             }

             if ( temporal_pred_flag[ plane ] && numRefFrames > 1 ) {

                 n = CeilLog2(numRefFrames)

                 rst_ref_pic_idx                                               f(n)

             } else {

                 rst_ref_pic_idx = 0

             }

             if ( temporal_pred_flag[ plane ] ) {

                 refIdx = ref_frame_idx[ rst_ref_pic_idx ]




AV2 Specification                                                           Page 130 of 1169
                   refPlane = plane

                   if ( plane > 0 && !RefFrameFiltersOn[ refIdx ][ plane ] ) {

                       refPlane = plane == 1 ? 2 : 1

                   }

                   if ( plane == 0 ) {

                       NumFilterClasses = RefNumFilterClasses[ refIdx ]

                   }

                   for ( c = 0; c < WIENER_NS_CLASSES; c++ ) {

                       for ( i = 0; i < WIENER_NS_CHROMA_COEFFS; i++ ) {

                           FrameLrWienerNs[plane][c][i] =

                            RefFrameLrWienerNs[refIdx][refPlane][c][i]

                       }

                   }

               }

           }

           if ( plane == 0 && frame_filters_on[ 0 ] ) {

               if ( temporal_pred_flag[ plane ] ) {

                   num_filter_classes_idx =

                       Encode_Num_Filter_Classes[ NumFilterClasses ]

               } else {

                   num_filter_classes_idx                                           f(3)

                   NumFilterClasses =

                       Decode_Num_Filter_Classes[ num_filter_classes_idx ]

               }

               qindex = base_q_idx

               index = get_filter_set_index(qindex)

               SubclassLookup =

                   Pc_Wiener_Sub_Classify2[ index ][ num_filter_classes_idx ]

           }

       }

   }

   UsesLr = usesLumaLr || usesChromaLr

   LoopRestorationSize[ 0 ] = RESTORATION_TILESIZE_MAX >> 3

   LoopRestorationSize[ 1 ] = RESTORATION_TILESIZE_MAX >>

                             ( 3 + Max(SubsamplingX, SubsamplingY) )

   if ( usesLumaLr ) {

       lr_luma_use_half_size                                                        f(1)

       if ( lr_luma_use_half_size ) {

           shift = 1

       } else if ( SbSize == BLOCK_256X256 ) {

           shift = 0




AV2 Specification                                                                Page 131 of 1169
         } else {

             lr_luma_use_max_size                                                f(1)

             if ( lr_luma_use_max_size ) {

                 shift = 0

             } else if ( SbSize == BLOCK_128X128 ) {

                 shift = 2

             } else {

                 lr_luma_use_quarter_size                                        f(1)

                 shift = lr_luma_use_quarter_size ? 2 : 3

             }

         }

         LoopRestorationSize[ 0 ] = RESTORATION_TILESIZE_MAX >> shift

     }

     if ( usesChromaLr ) {

         LoopRestorationSize[ 1 ] = RESTORATION_TILESIZE_MAX >>

                             Max(SubsamplingX, SubsamplingY)

         lr_chroma_use_half_size                                                 f(1)

         if ( lr_chroma_use_half_size ) {

             shift = 1

         } else if ( SbSize == BLOCK_256X256 ) {

             shift = 0

         } else {

             lr_chroma_use_max_size                                              f(1)

             if ( lr_chroma_use_max_size ) {

                 shift = 0

             } else if ( SbSize == BLOCK_128X128 ) {

                 shift = 2

             } else {

                 lr_chroma_use_quarter_size                                      f(1)

                 shift = lr_chroma_use_quarter_size ? 2 : 3

             }

         }

         LoopRestorationSize[ 1 ] = LoopRestorationSize[ 1 ] >> shift

     }

     LoopRestorationSize[ 2 ] = LoopRestorationSize[ 1 ]

     for ( plane = 0; plane < NumPlanes; plane++ ) {

         if ( frame_filters_on[ plane ] && !temporal_pred_flag[ plane ] ) {

             read_wienerns_filter(plane, 0, 0, 1)

         }

     }

 }




AV2 Specification                                                             Page 132 of 1169
    where the function get_filter_set_index is defined as:

     get_filter_set_index( base_qindex ) {
         if (base_qindex < 130) {
             return 0
         } else if (base_qindex < 190) {
             return 1
         } else if (base_qindex < 220) {
             return 2
         } else {
             return 3
         }
     }


    and the constant tables Decode_Num_Filter_Classes and Encode_Num_Filter_Classes are defined as:

     Encode_Num_Filter_Classes[ 17 ] = {
         0, 0, 1, 2, 3, 0, 4, 0, 5, 0, 0, 0, 6, 0, 0, 0, 7
     }

     Decode_Num_Filter_Classes[ 8 ] = {
         1, 2, 3, 4, 6, 8, 12, 16
     }


```

<a id="s-5-18-7-12"></a>

##### § 5.18.7.12 CCSO params syntax

```text
§   5.18.7.12. CCSO params syntax

     ccso_params( ) {                                                                      Descriptor

       for ( plane = 0; plane < NumPlanes; plane++ ) {

           ccso_planes[ plane ] = 0

       }

       if ( CodedLossless || !enable_ccso ) {

           return

       }

       a = 0

       for ( i = 0; i < TileCols; i++ ) {

           a = a | MiColStarts[ i ]

       }

       for ( i = 0; i < TileRows; i++ ) {

           a = a | MiRowStarts[ i ]

       }

       if ( ccso_unit_matches_sb_size ) {

           CcsoLumaSizeLog2 = Mi_Width_Log2[ SbSize ] + MI_SIZE_LOG2

       } else if ( (a & 63) == 0 ) {

           CcsoLumaSizeLog2 = 8

       } else if ( (a & 31) == 0 ) {

           CcsoLumaSizeLog2 = 7

       } else {

           CcsoLumaSizeLog2 = 6

       }




    AV2 Specification                                                                     Page 133 of 1169
   if ( single_picture_header_flag ) {

       ccso_frame_flag = 1

   } else {

       ccso_frame_flag                                                   f(1)

   }

   if ( !ccso_frame_flag ) {

       return

   }

   for ( plane = 0; plane < NumPlanes; plane++ ) {

       ccso_planes[ plane ]                                              f(1)

       if ( ccso_planes[ plane ] ) {

           if ( FrameIsIntra || FrameType == SWITCH_FRAME ) {

               reuse_ccso[ plane ] = 0

               sb_reuse_ccso[ plane ] = 0

           } else {

               reuse_ccso[ plane ]                                       f(1)

               sb_reuse_ccso[ plane ]                                    f(1)

           }

           if ( reuse_ccso[ plane ] || sb_reuse_ccso[ plane ] ) {

               n = CeilLog2(NumTotalRefs)

               ccso_ref_idx[ plane ]                                     f(n)

               idx = ref_frame_idx[ ccso_ref_idx[ plane ] ]

               tmpCcsoLumaSizeLog2 = CcsoLumaSizeLog2

               load_ccso_params(idx, plane)

               CcsoLumaSizeLog2 = tmpCcsoLumaSizeLog2

           }

       }

       if ( ccso_planes[ plane ] && !reuse_ccso[ plane ] ) {

           ccso_bo_only[ plane ]                                         f(1)

           ccso_scale_idx[ plane ]                                       f(2)

           if ( ccso_bo_only[ plane ] ) {

               ccso_quant_idx[ plane ] = 0

               ccso_ext_filter[ plane ] = 0

               ccso_edge_clf[ plane ] = 0

           } else {

               ccso_quant_idx[ plane ]                                   f(2)

               ccso_ext_filter[ plane ]                                  f(3)

               quantStep = CCSO_Quant_Sz[ ccso_scale_idx[ plane ] ]

                         [ ccso_quant_idx[ plane ] ]

               if ( quantStep == 0 ) {

                ccso_edge_clf[ plane ] = 0




AV2 Specification                                                     Page 134 of 1169
                     } else {

                         ccso_edge_clf[ plane ]                                          f(1)

                     }

                 }

                 n = 2 + ccso_bo_only[ plane ]

                 ccso_max_band_log2[ plane ]                                             f(n)

                 maxEdgeInterval = CCSO_INPUT_INTERVAL - ccso_edge_clf[ plane ]

                 if ( ccso_bo_only[ plane ] ) {

                     maxEdgeInterval = 1

                 }

                 maxBand = 1 << ccso_max_band_log2[ plane ]

                 for ( d0 = 0; d0 < maxEdgeInterval; d0++ ) {

                     for ( d1 = 0; d1 < maxEdgeInterval; d1++ ) {

                         for ( band = 0; band < maxBand; band++ ) {

                             ccso_offset_idx                                             tu(7)

                             offset = Ccso_Offset[ ccso_offset_idx ] *

                                  (ccso_scale_idx[ plane ] + 1)

                             CcsoFilterOffset[ plane ][ band ][ d0 ][ d1 ] = offset

                         }

                     }

                 }

             }

         }

     }


    where Ccso_Offset is defined as:

     Ccso_Offset[ 8 ] = {
         0, 1, -1, 3, -3, 7, -7, -10
     }


```

<a id="s-5-18-8"></a>

#### § 5.18.8 Transform and coding mode structures

```text
§   5.18.8. Transform and coding mode structures

```

<a id="s-5-18-8-1"></a>

##### § 5.18.8.1 TX mode syntax

```text
§   5.18.8.1. TX mode syntax

     read_tx_mode( ) {                                                                Descriptor

         if ( CodedLossless == 1 ) {

             TxMode = ONLY_4X4

         } else {

             tx_mode_select                                                              f(1)

             if ( tx_mode_select ) {

                 TxMode = TX_MODE_SELECT

             } else {

                 TxMode = TX_MODE_LARGEST



    AV2 Specification                                                                 Page 135 of 1169
             }

         }

     }


```

<a id="s-5-18-8-2"></a>

##### § 5.18.8.2 Skip mode params syntax

```text
§   5.18.8.2. Skip mode params syntax

     skip_mode_params( ) {                                        Descriptor

         if ( FrameIsIntra || FrameType == SWITCH_FRAME ) {

             skipModeAllowed = 0

         } else {

             skipModeAllowed = 1

             SkipModeFrame[ 0 ] = 0

             SkipModeFrame[ 1 ] = NumTotalRefs > 1 ? 1 : 0

             if ( NumTotalRefs > 1 ) {

                 curToRef0 = Abs(get_relative_dist(OrderHint,

                           RefOrderHint[ ref_frame_idx[ 0 ] ]))

                 curToRef1 = Abs(get_relative_dist(OrderHint,

                           RefOrderHint[ ref_frame_idx[ 1 ] ]))

                 if ( OrderHints[ 0 ] == RESTRICTED_OH ) {

                     curToRef0 = 0

                 }

                 if ( OrderHints[ 1 ] == RESTRICTED_OH ) {

                     curToRef1 = 0

                 }

                 if ( Abs(curToRef0 - curToRef1) > 1 ) {

                     SkipModeFrame[ 1 ] = 0

                 }

             }

         }

         if ( skipModeAllowed ) {

             skip_mode_present                                       f(1)

         } else {

             skip_mode_present = 0

         }

     }


```

<a id="s-5-18-8-3"></a>

##### § 5.18.8.3 Frame reference mode syntax

```text
§   5.18.8.3. Frame reference mode syntax

     frame_reference_mode( ) {                                    Descriptor

         if ( FrameIsIntra ) {

             reference_select = 0

         } else {

             reference_select                                        f(1)




    AV2 Specification                                             Page 136 of 1169
         }

     }


```

<a id="s-5-18-9"></a>

#### § 5.18.9 Global motion structures

```text
§   5.18.9. Global motion structures

```

<a id="s-5-18-9-1"></a>

##### § 5.18.9.1 Global motion params syntax

```text
§   5.18.9.1. Global motion params syntax

     global_motion_params( ) {                                                     Descriptor

         for ( ref = 0; ref < REFS_PER_FRAME; ref++ ) {

             GmType[ ref ] = IDENTITY

             for ( i = 0; i < 6; i++ ) {

                 gm_params[ ref ][ i ] = ( ( i % 3 == 2 ) ?

                       1 << WARPEDMODEL_PREC_BITS : 0 )

             }

         }

         if ( FrameIsIntra || !enable_global_motion) {

             return

         }

         use_global_motion                                                            f(1)

         if ( !use_global_motion ) {

             return

         }

         for ( i = 0; i < 6; i++ ) {

             baseParams[ i ] = Default_Warp_Params[ i ]

         }

         baseDistance = 1

         if ( FrameType == SWITCH_FRAME ) {

             our_ref = NumTotalRefs

         } else {

             n = NumTotalRefs + 1

             our_ref                                                                  ns(n)

         }

         if ( our_ref != NumTotalRefs ) {

             refIdx = ref_frame_idx[ our_ref ]

             if ( RefNumTotalRefs[ refIdx ] > 0 ) {

                 n = RefNumTotalRefs[ refIdx ]

                 their_ref                                                            ns(n)

                 for ( i = 0; i < 6; i++ ) {

                     baseParams[ i ] = SavedGmParams[ refIdx ][ their_ref ][ i ]

                 }

                 baseDistance = get_relative_dist(OrderHints[ our_ref ],

                           SavedOrderHints[ refIdx ][ their_ref ])

             }



    AV2 Specification                                                              Page 137 of 1169
     }

     for ( ref = 0; ref < NumTotalRefs; ref++ ) {

         dist = get_relative_dist(OrderHint,OrderHints[ ref ])

         if ( dist == 0 || OrderHints[ ref ] == RESTRICTED_OH ) {

             for ( i = 0; i < 6; i++ ) {

                 gm_params[ ref ][ i ] = Default_Warp_Params[ i ]

             }

             GmType[ ref ] = IDENTITY

         } else {

             for ( i = 0; i < 6; i++ ) {

                 params = scale_warp_model(baseParams, baseDistance, dist)

                 PrevGmParams[ ref ][ i ] = params[ i ]

             }

             is_global                                                          f(1)

             if ( is_global ) {

                 is_rot_zoom                                                    f(1)

                 if ( is_rot_zoom ) {

                     type = ROTZOOM

                 } else {

                     type = AFFINE

                 }

             } else {

                 type = IDENTITY

             }

             GmType[ ref ] = type

             if ( type >= ROTZOOM ) {

                 read_global_param(ref,2)

                 read_global_param(ref,3)

                 if ( type == AFFINE ) {

                     read_global_param(ref,4)

                     read_global_param(ref,5)

                 } else {

                     gm_params[ ref ][ 4 ] = -gm_params[ ref ][ 3 ]

                     gm_params[ ref ][ 5 ] = gm_params[ ref ][ 2 ]

                 }

                 read_global_param(ref,0)

                 read_global_param(ref,1)

             }

         }

     }

 }




AV2 Specification                                                            Page 138 of 1169
    where Param_Shift, Param_Min, Param_Max, and scale_warp_model are defined as:

     Param_Shift[ 6 ] = {
         GM_TRANS_PREC_DIFF,           GM_TRANS_PREC_DIFF,   GM_ALPHA_PREC_DIFF,
         GM_ALPHA_PREC_DIFF,           GM_ALPHA_PREC_DIFF,   GM_ALPHA_PREC_DIFF
     }

     Param_Min[ 6 ] = {
         GM_TRANS_MIN,           GM_TRANS_MIN,
         GM_ALPHA_MIN,           GM_ALPHA_MIN,
         GM_ALPHA_MIN,           GM_ALPHA_MIN
     }

     Param_Max[ 6 ] = {
         GM_TRANS_MAX,           GM_TRANS_MAX,
         GM_ALPHA_MAX,           GM_ALPHA_MAX,
         GM_ALPHA_MAX,           GM_ALPHA_MAX
     }


     scale_warp_model(baseParams, baseDistance, dist) {
         if ( baseDistance == 0 ) {
             return Default_Warp_Params
         }
         if ( baseDistance < 0 ) {
             baseDistance = -baseDistance
             dist = -dist
         }
         for ( i = 0; i < 6; i++ ) {
             center = Default_Warp_Params[ i ]
             limit = (1 << 22) - 1
             input = Clip3( -limit, limit, baseParams[ i ] - center )
             (divShift, divFactor) = resolve_divisor( baseDistance )
             scaled = Round2Signed( input * divFactor, divShift )
             output = Round2Signed( scaled * dist, Param_Shift[ i ] )
             output = Clip3( Param_Min[i], Param_Max[i], output ) << Param_Shift[i]
             params[ i ] = center + output
         }
         return params
     }


```

<a id="s-5-18-9-2"></a>

##### § 5.18.9.2 Global param syntax

```text
§   5.18.9.2. Global param syntax

     read_global_param( ref, idx ) {                                                    Descriptor

         precBits = GM_ALPHA_PREC_BITS

         mx = GM_ALPHA_MAX

         if ( idx < 2 ) {

             precBits = GM_TRANS_PREC_BITS

             mx = GM_TRANS_MAX

         }

         precDiff = WARPEDMODEL_PREC_BITS - precBits

         round = (idx % 3) == 2 ? (1 << WARPEDMODEL_PREC_BITS) : 0

         sub = (idx % 3) == 2 ? (1 << precBits) : 0

         r = (PrevGmParams[ ref ][ idx ] >> precDiff) - sub

         gm_params[ ref ][ idx ] =

             (decode_signed_subexp_with_ref( -mx, mx + 1, r, 3 ) << precDiff) + round

     }




    AV2 Specification                                                                   Page 139 of 1169
         NOTE: When force_integer_mv is equal to 1, some fractional bits are still read for the translation
         components. However, these fractional bits will be discarded during the Setup Global MV process.

```

<a id="s-5-18-9-3"></a>

##### § 5.18.9.3 Decode signed subexp with ref syntax

```text
§   5.18.9.3. Decode signed subexp with ref syntax

     decode_signed_subexp_with_ref( low, high, r, k ) {                                           Descriptor

         x = decode_unsigned_subexp_with_ref(high - low, r - low, k)

         return x + low

     }


```

<a id="s-5-18-9-4"></a>

##### § 5.18.9.4 Decode unsigned subexp with ref syntax

```text
§   5.18.9.4. Decode unsigned subexp with ref syntax

     decode_unsigned_subexp_with_ref( mx, r, k ) {                                                Descriptor

         v = decode_subexp( mx, k )

         if ( (r << 1) <= mx ) {

             return inverse_recenter(r, v)

         } else {

             return mx - 1 - inverse_recenter(mx - 1 - r, v)

         }

     }


```

<a id="s-5-18-9-5"></a>

##### § 5.18.9.5 Decode subexp syntax

```text
§   5.18.9.5. Decode subexp syntax

     decode_subexp( numSyms, k ) {                                                                Descriptor

         i = 0

         mk = 0

         while ( 1 ) {

             b2 = i ? k + i - 1 : k

             a = 1 << b2

             if ( numSyms <= mk + 3 * a ) {

                 n = numSyms - mk

                 subexp_final_bits                                                                   ns(n)

                 return subexp_final_bits + mk

             } else {

                 subexp_more_bits                                                                    f(1)

                 if ( subexp_more_bits ) {

                     i++

                     mk += a

                 } else {

                     subexp_bits                                                                     f(b2)

                     return subexp_bits + mk

                 }

             }




    AV2 Specification                                                                             Page 140 of 1169
         }

     }


```

<a id="s-5-18-9-6"></a>

##### § 5.18.9.6 Inverse recenter function

```text
§   5.18.9.6. Inverse recenter function

     inverse_recenter( r, v ) {
         if ( v > 2 * r ) {
             return v
         } else if ( v & 1 ) {
             return r - ((v + 1) >> 1)
         } else {
             return r + (v >> 1)
         }
     }


```

<a id="s-5-18-10"></a>

#### § 5.18.10 Film grain structures

```text
§   5.18.10. Film grain structures

```

<a id="s-5-18-10-1"></a>

##### § 5.18.10.1 Film grain config syntax

```text
§   5.18.10.1. Film grain config syntax

     film_grain_config( ) {                                                                             Descriptor

         if ( !film_grain_params_present || ( !immediate_output_frame && !implicit_output_frame ) ) {

             apply_grain = 0

         } else if ( single_picture_header_flag ) {

             apply_grain = 1

         } else {

             apply_grain                                                                                   f(1)

         }

         if ( apply_grain ) {

             fgm_id                                                                                        f(3)

             load_grain_model( fgm_id )

             grain_seed                                                                                    f(16)

         }

     }


```

<a id="s-5-18-10-2"></a>

##### § 5.18.10.2 Film grain model syntax

```text
§   5.18.10.2. Film grain model syntax

     film_grain_model( monochrome, subX, subY ) {                                                       Descriptor

         if ( monochrome ) {

             chroma_scaling_from_luma = 0

         } else {

             chroma_scaling_from_luma                                                                      f(1)

         }

         num_y_points                                                                                      f(4)

         if ( num_y_points > 0) {

             point_value_increment_bits_minus_1                                                            f(3)

             bitsIncr = point_value_increment_bits_minus_1 + 1

             point_scaling_bits_minus_5                                                                    f(2)




    AV2 Specification                                                                                   Page 141 of 1169
       bitsScal = point_scaling_bits_minus_5 + 5

   }

   for ( i = 0; i < num_y_points; i++ ) {

       point_y_value[ i ]                                       f(bitsIncr)

       if ( i > 0 ) {

           point_y_value[ i ] += point_y_value[ i - 1 ]

       }

       point_y_scaling[ i ]                                     f(bitsScal)

   }

   if ( monochrome || chroma_scaling_from_luma ) {

       num_cb_points = 0

       num_cr_points = 0

   } else {

       num_cb_points                                               f(4)

       if ( num_cb_points > 0 ) {

           point_value_increment_bits_minus_1                      f(3)

           bitsIncr = point_value_increment_bits_minus_1 + 1

           point_scaling_bits_minus_5                              f(2)

           bitsScal = point_scaling_bits_minus_5 + 5

       }

       for ( i = 0; i < num_cb_points; i++ ) {

           point_cb_value[ i ]                                  f(bitsIncr)

           if ( i > 0 ) {

               point_cb_value[ i ] += point_cb_value[ i - 1 ]

           }

           point_cb_scaling[ i ]                                f(bitsScal)

       }

       num_cr_points                                               f(4)

       if ( num_cr_points > 0 ) {

           point_value_increment_bits_minus_1                      f(3)

           bitsIncr = point_value_increment_bits_minus_1 + 1

           point_scaling_bits_minus_5                              f(2)

           bitsScal = point_scaling_bits_minus_5 + 5

       }

       for ( i = 0; i < num_cr_points; i++ ) {

           point_cr_value[ i ]                                  f(bitsIncr)

           if ( i > 0 ) {

               point_cr_value[ i ] += point_cr_value[ i - 1 ]

           }

           point_cr_scaling[ i ]                                f(bitsScal)

       }




AV2 Specification                                               Page 142 of 1169
   }

   grain_scaling_minus_8                                     f(2)

   ar_coeff_lag                                              f(2)

   numPosLuma = 2 * ar_coeff_lag * ( ar_coeff_lag + 1 )

   if ( num_y_points ) {

       bits_per_ar_coeff_y_minus_5                           f(2)

       bitsCoef = bits_per_ar_coeff_y_minus_5 + 5

       numPosChroma = numPosLuma + 1

       for ( i = 0; i < numPosLuma; i++ ) {

           ar_coeffs_y[ i ]                               f(bitsCoef)

           ar_coeffs_y[ i ] -= (1 << (bitsCoef - 1))

       }

   } else {

       numPosChroma = numPosLuma

   }

   if ( chroma_scaling_from_luma || num_cb_points ) {

       bits_per_ar_coeff_cb_minus_5                          f(2)

       bitsCoef = bits_per_ar_coeff_cb_minus_5 + 5

       for ( i = 0; i < numPosChroma; i++ ) {

           ar_coeffs_cb[ i ]                              f(bitsCoef)

           ar_coeffs_cb[ i ] -= (1 << (bitsCoef - 1))

       }

   }

   if ( chroma_scaling_from_luma || num_cr_points ) {

       bits_per_ar_coeff_cr_minus_5                          f(2)

       bitsCoef = bits_per_ar_coeff_cr_minus_5 + 5

       for ( i = 0; i < numPosChroma; i++ ) {

           ar_coeffs_cr[ i ]                              f(bitsCoef)

           ar_coeffs_cr[ i ] -= (1 << (bitsCoef - 1))

       }

   }

   ar_coeff_shift_minus_6                                    f(2)

   grain_scale_shift                                         f(2)

   if ( num_cb_points ) {

       cb_mult                                               f(8)

       cb_luma_mult                                          f(8)

       cb_offset                                             f(9)

   }

   if ( num_cr_points ) {

       cr_mult                                               f(8)

       cr_luma_mult                                          f(8)

       cr_offset



AV2 Specification                                         Page 143 of 1169
                                                                         f(9)

         }

         overlap_flag                                                    f(1)

         clip_to_restricted_range                                        f(1)

         if ( clip_to_restricted_range ) {

             fg_mc_identity                                              f(1)

         } else {

             fg_mc_identity = 0

         }

         film_grain_block_size                                           f(1)

     }


```

<a id="s-5-19"></a>

### § 5.19 Tile group OBU syntax

```text
§   5.19. Tile group OBU syntax
     tile_group_obu( sz ) {                                           Descriptor

         startBitPos = get_position( )

         is_first_tile_group                                             f(1)

         if ( is_first_tile_group ) {

             frame_header_present_flag = 1

         } else {

             frame_header_present_flag                                   f(1)

         }

         if ( frame_header_present_flag ) {

             frame_header( is_first_tile_group )

         }

         if ( bru_inactive ) {

             headerBits = get_position( ) - startBitPos

             remainingBits    = sz * 8 - headerBits

             trailing_bits( remainingBits )

             return

         }

         NumTiles = TileCols * TileRows

         tile_start_and_end_present_flag = 0

         if ( NumTiles > 1 ) {

             tile_start_and_end_present_flag                             f(1)

         }

         if ( NumTiles == 1 || !tile_start_and_end_present_flag ) {

             tg_start = 0

             tg_end = NumTiles - 1

         } else {

             tileBits = TileColsLog2 + TileRowsLog2

             tg_start                                                 f(tileBits)




    AV2 Specification                                                 Page 144 of 1169
             tg_end                                                                 f(tileBits)

         }

         if ( use_bru ) {

             if ( NumTiles > 1 ) {

                 for ( TileNum = tg_start; TileNum <= tg_end; TileNum++ ) {

                     tileRow = TileNum / TileCols

                     tileCol = TileNum % TileCols

                     bru_tile_active                                                   f(1)

                     BruTileActives[ tileRow ][ tileCol ] = bru_tile_active

                 }

             } else {

                 BruTileActives[ 0 ][ 0 ] = 1

             }

         }

         byte_alignment( )

         endBitPos = get_position( )

         headerBytes = (endBitPos - startBitPos) / 8

         sz -= headerBytes

         tile_group_payload( sz )

     }


```

<a id="s-5-20"></a>

### § 5.20 Tile group payload syntax

```text
§   5.20. Tile group payload syntax
```

<a id="s-5-20-1"></a>

#### § 5.20.1 General tile group payload syntax

```text
§   5.20.1. General tile group payload syntax

     tile_group_payload( sz ) {                                                     Descriptor

         for ( TileNum = tg_start; TileNum <= tg_end; TileNum++ ) {

             tileRow = TileNum / TileCols

             tileCol = TileNum % TileCols

             lastTile = TileNum == tg_end

             if ( lastTile ) {

                 tileSize = sz

             } else if ( !IsBridge ) {

                 tile_size_minus_1                                                le(TileSizeByte
                                                                                         s)

                 tileSize = tile_size_minus_1 + 1

                 sz -= tileSize + TileSizeBytes

             }

             MiRowStart = MiRowStarts[ tileRow ]

             MiRowEnd = MiRowStarts[ tileRow + 1 ]

             MiColStart = MiColStarts[ tileCol ]

             MiColEnd = MiColStarts[ tileCol + 1 ]

             BruTileActive = use_bru ? BruTileActives[ tileRow ][ tileCol ] : 0




    AV2 Specification                                                              Page 145 of 1169
             align = Num_4x4_Blocks_High[ SbSize ]

             shift = Mi_Height_Log2[ SbSize ]

             for( r = MiRowStart; r < ((MiRowEnd + align - 1) >> shift) << shift;

                 r++) {

                 for( c = MiColStart; c < ((MiColEnd + align - 1) >> shift) << shift;

                     c++) {

                     IBCCoded[ r ][ c ] = 0

                 }

             }

             CurrentQIndex = base_q_idx

             if ( !IsBridge ) {

                 init_symbol( tileSize )

             }

             decode_tile( )

             if ( !IsBridge ) {

                 exit_symbol( )

             }

         }

         if ( tg_end == NumTiles - 1 ) {

             if ( !IsBridge ) {

                 frame_end_update_cdf( )

             }

             decode_frame_wrapup( )

             SeenFrameHeader = 0

         }

     }


```

<a id="s-5-20-2"></a>

#### § 5.20.2 Tile-level structures

```text
§   5.20.2. Tile-level structures

```

<a id="s-5-20-2-1"></a>

##### § 5.20.2.1 Decode tile syntax

```text
§   5.20.2.1. Decode tile syntax

     decode_tile( ) {                                                                   Descriptor

         clear_above_context( )

         for ( plane = 0; plane < WIENER_NS_PLANES; plane++ ) {

             for ( c = 0; c < WIENER_NS_CLASSES; c++ ) {

                 for ( i = 0; i < WIENER_NS_CHROMA_COEFFS; i++ ) {

                     min = Wiener_Ns_Taps_Min[ plane != 0 ][ i ]

                     k = Wiener_Ns_Taps_K[ plane != 0 ][ i ]

                     RefLrWienerNs[ plane ][ c ][ 0 ][ i ] = min + ((1 << k) >> 1)

                 }

                 WienerNsPtr[ plane ][ c ] = 0

                 WienerNsBankSize[ plane ][ c ] = 0

             }



    AV2 Specification                                                                   Page 146 of 1169
     }

     sbSize4 = Num_4x4_Blocks_Wide[ SbSize ]

     for ( r = MiRowStart; r < MiRowEnd; r += sbSize4 ) {

         clear_left_context( )

         for ( i = 0; i < IBC_NUM_BUFFERS; i++ ) {

             IBCBufferValid[ i ] = 0

         }

         IBCBufferCurRow = r >> (IBC_BUFFER_SIZE_LOG2 - MI_SIZE_LOG2)

         IBCBufferCurCol = 0

         for ( c = MiColStart; c < MiColEnd; c += sbSize4 ) {

             reset_refmv_bank( r, c, sbSize4, r == MiRowStart )

             ReadDeltas = delta_q_present

             clear_cdef( r, c )

             clear_block_decoded_flags( r, c, sbSize4 )

             if ( IsBridge ) {

                 bru_mode = BRU_INACTIVE

             } else if ( BruTileActive ) {

                 bru_mode                                                                    S()

             } else {

                 bru_mode = use_bru ? BRU_INACTIVE : BRU_ACTIVE

             }

             BruModes[ r ][ c ] = bru_mode

             RegionType = MIXED_REGION

             TreeType = SHARED_PART

             PlaneStart = 0

             PlaneEnd = NumPlanes

             decode_partition( r, c, SbSize, BLOCK_INVALID, 0, 1,

                      enable_extended_sdp && !FrameIsIntra )

         }

     }

 }


where Wiener_Ns_Taps_Min and Wiener_Ns_Taps_K are constant lookup tables specified as:

 Wiener_Ns_Taps_Min[ 2 ][ 18 ] = {
     {-24, -24, -14 , -14, -16, -16, -8,   -8, -8, -8, -8, -8, -8, -8, -8, -8,
      -8, -8},
     {-24, -24, -14 , -14, -16, -16, -16, -16, -16, -16, -8, -8, -8, -8, -8, -8,
      -8, -8}
 }

 Wiener_Ns_Taps_K[ 2 ][ 18 ] = {
     {6, 6, 5, 5, 5, 5, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4},
     {6, 6, 5, 5, 5, 5, 5, 5, 5, 5, 4, 4, 4, 4, 4, 4, 4, 4}
 }




AV2 Specification                                                                        Page 147 of 1169
```

<a id="s-5-20-2-2"></a>

##### § 5.20.2.2 Reset reference motion vector bank function

```text
§   5.20.2.2. Reset reference motion vector bank function

     reset_refmv_bank( r, c, sbSize4, topRow ) {
         WarpBankHits = 0
         RefMvBankHits = 0
         RefMvRemainHits = 0
         RefMvUnitHits = 0
         if ( FrameIsIntra || topRow ) {
             return
         }
         rowHits = 0
         candRow = r - 1
         candCol = c
         while ( candCol < MiCols && candCol < c + sbSize4 && rowHits < 4 ) {
             candCol2 = (candCol >> 1) << 1
             if ( IsInters[ candRow ][ candCol2 ] ) {
                 rowHits++
                 update_ref_mv_bank( RefFrames[ candRow ][ candCol2 ],
                     Mvs[ candRow ][ candCol2 ], CwpIdxs[ candRow ][ candCol2 ],0)
                 if ( MotionModes[ candRow ][ candCol2 ] >= LOCALWARP ) {
                     update_warp_param_bank( RefFrames[ candRow ][ candCol2 ],
                                             WarpParams[ candRow ][ candCol2 ],1)
                 }
             }
             candSize = MiSizes[ 0 ][ candRow ][ candCol2 ]
             candCol += Num_4x4_Blocks_Wide[ candSize ]
         }
     }


```

<a id="s-5-20-2-3"></a>

##### § 5.20.2.3 Clear block decoded flags function

```text
§   5.20.2.3. Clear block decoded flags function

     clear_block_decoded_flags( r, c, sbSize4 ) {
         for ( plane = 0; plane < NumPlanes; plane++ ) {
             subX = (plane > 0) ? SubsamplingX : 0
             subY = (plane > 0) ? SubsamplingY : 0
             sbWidth4 = ( MiColEnd - c ) >> subX
             sbHeight4 = ( MiRowEnd - r ) >> subY
             for ( y = -1; y <= ( sbSize4 >> subY ); y++ ) {
                 for ( x = -1; x <= ( (2 * sbSize4) >> subX ); x++ ) {
                     if ( y < 0 && x < sbWidth4 ) {
                         BlockDecoded[ plane ][ y ][ x ] = 1
                     } else if ( x < 0 && y < sbHeight4 ) {
                         BlockDecoded[ plane ][ y ][ x ] = 1
                     } else {
                         BlockDecoded[ plane ][ y ][ x ] = 0
                     }
                 }
             }
             BlockDecoded[ plane ][ sbSize4 >> subY ][ -1 ] = 0
         }
     }


```

<a id="s-5-20-3"></a>

#### § 5.20.3 Partition structures

```text
§   5.20.3. Partition structures

```

<a id="s-5-20-3-1"></a>

##### § 5.20.3.1 Decode partition syntax

```text
§   5.20.3.1. Decode partition syntax

     decode_partition( r, c, bSize, parentSize, chromaOffset, hasChroma, extendedSdpAllowed ) {   Descriptor

       if ( r >= MiRows || c >= MiCols ) {

         for ( y = 0; y < Num_4x4_Blocks_High[ bSize ]; y++ ) {

           for ( x = 0; x < Num_4x4_Blocks_Wide[ bSize ]; x++ ) {

               IBCCoded[ r + y ][ x + c ] = 1

           }




    AV2 Specification                                                                             Page 148 of 1169
       }

       widthChunks = Max( 1, Block_Width[ bSize ] >> 6 )

       heightChunks = Max( 1, Block_Height[ bSize ] >> 6 )

       for ( chunkY = 0; chunkY < heightChunks; chunkY++ ) {

           for ( chunkX = 0; chunkX < widthChunks; chunkX++ ) {

               miRowChunk = r + ( chunkY << 4 )

               miColChunk = c + ( chunkX << 4 )

               update_ibc_buffers(miRowChunk, miColChunk)

           }

       }

       return

   }

   if ( enable_sdp && TreeType == SHARED_PART &&

       bSize == BLOCK_64X64 && FrameIsIntra ) {

       TreeType = LUMA_PART

       PlaneStart = 0

       PlaneEnd = 1

       decode_partition( r, c, BLOCK_64X64, parentSize, 0, 1, 0 )

       TreeType = CHROMA_PART

       PlaneStart = 1

       PlaneEnd = NumPlanes

       decode_partition( r, c, BLOCK_64X64, parentSize, 0, 1, 0 )

       TreeType = SHARED_PART

       PlaneStart = 0

       PlaneEnd = NumPlanes

       return

   }

   if ( SbSize == bSize ) {

       read_lr(r, c, SbSize)

   }

   AvailU = is_inside( r - 1, c )

   AvailL = is_inside( r, c - 1 )

   num4x4wide = Num_4x4_Blocks_Wide[ bSize ]

   halfBlock4x4wide = num4x4wide >> 1

   num4x4high = Num_4x4_Blocks_High[ bSize ]

   halfBlock4x4high = num4x4high >> 1

   partition = read_partition(r, c, bSize, chromaOffset, hasChroma)

   subSize = Partition_Subsize[ partition ][ bSize ]

   usingSdp = 0

   if ( bSize != SbSize && extendedSdpAllowed &&

           TreeType == SHARED_PART &&




AV2 Specification                                                     Page 149 of 1169
           is_bsize_allowed_for_extended_sdp(bSize, partition) &&

           bru_mode == BRU_ACTIVE ) {

       region_type                                                                S()

       if ( region_type == INTRA_REGION ) {

           TreeType = LUMA_PART

           RegionType = INTRA_REGION

           PlaneStart = 0

           PlaneEnd = 1

           usingSdp = 1

       }

   }

   extendedSdpAllowed = extendedSdpAllowed && Block_Width[ subSize ] > 4 &&

                Block_Height[ subSize ] > 4

   if ( partition == PARTITION_HORZ_3 || partition == PARTITION_VERT_3 ) {

       subSize2 = H_Partition_Midsize[ bSize ]

       extendedSdpAllowed = extendedSdpAllowed &&

                  Block_Width[ subSize2 ] > 4 &&

                  Block_Height[ subSize2 ] > 4

   }

   if ( SbSize == BLOCK_128X128 ) {

       if ( bSize == BLOCK_128X128 ) {

           AllowExtraIBCRange = partition == PARTITION_HORZ ||

                    partition == PARTITION_SPLIT

       }

   } else {

       AllowExtraIBCRange = 0

   }

   if ( FrameIsIntra ) {

       if ( TreeType == LUMA_PART && bSize == BLOCK_64X64 ) {

           TopLumaHorz = partition == PARTITION_HORZ ||

                 partition == PARTITION_HORZ_3

           TopLumaVert = partition == PARTITION_VERT ||

                 partition == PARTITION_VERT_3

           TopLumaUnevenHorz = partition == PARTITION_HORZ_4A ||

                   partition == PARTITION_HORZ_4B

           TopLumaUnevenVert = partition == PARTITION_VERT_4A ||

                   partition == PARTITION_VERT_4B

           ChromaFollowsLuma = (partition == PARTITION_NONE) ||

                   TopLumaHorz || TopLumaVert

           LumaPartitions[ r ][ c ] = partition

       }




AV2 Specification                                                             Page 150 of 1169
       thisHorz = partition == PARTITION_HORZ ||

                partition == PARTITION_HORZ_3 ||

                partition == PARTITION_HORZ_4A ||

                partition == PARTITION_HORZ_4B

       thisVert = partition == PARTITION_VERT ||

                partition == PARTITION_VERT_3 ||

                partition == PARTITION_VERT_4A ||

                partition == PARTITION_VERT_4B

       if ( TreeType == CHROMA_PART && bSize == BLOCK_64X64 ) {

           if ( ChromaFollowsLuma ||

                partition == PARTITION_NONE ||

                (TopLumaHorz || TopLumaUnevenHorz) && thisHorz ||

                (TopLumaVert || TopLumaUnevenVert) && thisVert ) {

               CflAllowedInSdp = 1

           } else {

               CflAllowedInSdp = 0

           }

       }

       if ( TreeType == LUMA_PART && parentSize == BLOCK_64X64 ) {

           if ( partition == PARTITION_NONE ||

               ( TopLumaHorz && thisHorz ) ||

               ( TopLumaVert && thisVert ) ) {

               ChromaFollowsLuma = 0

           }

       }

   }

   if ( !chromaOffset && hasChroma ) {

       chromaOffset = is_chroma_offset_for_partition( partition, bSize )

       ChromaMiRow = r

       ChromaMiCol = c

       ChromaMiSize = bSize

   }

   if ( partition == PARTITION_NONE ) {

       HasChroma = hasChroma && NumPlanes > 1 && TreeType != LUMA_PART

       decode_block( r, c, subSize )

   } else if ( partition == PARTITION_HORZ ) {

       decode_partition( r, c, subSize, bSize, chromaOffset,

                   hasChroma && !chromaOffset, extendedSdpAllowed )

       decode_partition( r + halfBlock4x4high, c, subSize, bSize, chromaOffset,

                   hasChroma, extendedSdpAllowed )

   } else if ( partition == PARTITION_VERT ) {




AV2 Specification                                                                 Page 151 of 1169
     decode_partition( r, c, subSize, bSize, chromaOffset,

              hasChroma && !chromaOffset, extendedSdpAllowed )

     decode_partition( r, c + halfBlock4x4wide, subSize, bSize, chromaOffset,

              hasChroma, extendedSdpAllowed )

   } else if ( partition == PARTITION_HORZ_3 ) {

     decode_partition( r, c, subSize, bSize, chromaOffset,

              hasChroma && !chromaOffset, extendedSdpAllowed )

     middleChroma = bSize == BLOCK_8X32 && hasChroma && SubsamplingX

     if ( middleChroma ) {

         ChromaMiRow = r + (halfBlock4x4high >> 1)

         ChromaMiCol = c

         ChromaMiSize = Partition_Subsize[ PARTITION_HORZ ][ bSize ]

     }

     decode_partition( r + (halfBlock4x4high >> 1), c,

              H_Partition_Midsize[ bSize ], bSize,

              chromaOffset || middleChroma,

              hasChroma && !chromaOffset && !middleChroma,

              extendedSdpAllowed )

     decode_partition( r + (halfBlock4x4high >> 1), c + halfBlock4x4wide,

              H_Partition_Midsize[ bSize ], bSize,

              chromaOffset || middleChroma,

              hasChroma && !chromaOffset, extendedSdpAllowed )

     decode_partition( r + 3 * (halfBlock4x4high >> 1), c,

              subSize, bSize, chromaOffset,

              hasChroma, extendedSdpAllowed )

   } else if ( partition == PARTITION_HORZ_4A ) {

     bSizeBig = Partition_Subsize[ PARTITION_HORZ ][ bSize ]

     bsizeMed = Partition_Subsize[ PARTITION_HORZ ][ bSizeBig ]

     decode_partition( r, c, subSize, bSize,

              chromaOffset, hasChroma && !chromaOffset,

              extendedSdpAllowed )

     decode_partition( r + (num4x4high >> 3), c, bsizeMed, bSize,

              chromaOffset, hasChroma && !chromaOffset,

              extendedSdpAllowed )

     decode_partition( r + 3 * (num4x4high >> 3), c, bSizeBig, bSize,

              chromaOffset, hasChroma && !chromaOffset,

              extendedSdpAllowed )

     decode_partition( r + 7 * (num4x4high >> 3), c, subSize, bSize,

              chromaOffset, hasChroma, extendedSdpAllowed )

   } else if ( partition == PARTITION_HORZ_4B ) {

     bSizeBig = Partition_Subsize[ PARTITION_HORZ ][ bSize ]




AV2 Specification                                                               Page 152 of 1169
     bsizeMed = Partition_Subsize[ PARTITION_HORZ ][ bSizeBig ]

     decode_partition( r, c, subSize, bSize,

              chromaOffset, hasChroma && !chromaOffset,

              extendedSdpAllowed )

     decode_partition( r + (num4x4high >> 3), c, bSizeBig, bSize,

              chromaOffset, hasChroma && !chromaOffset,

              extendedSdpAllowed )

     decode_partition( r + 5 * (num4x4high >> 3), c, bsizeMed, bSize,

              chromaOffset, hasChroma && !chromaOffset,

              extendedSdpAllowed )

     decode_partition( r + 7 * (num4x4high >> 3), c, subSize, bSize,

              chromaOffset, hasChroma, extendedSdpAllowed )

   } else if ( partition == PARTITION_VERT_4A ) {

     bSizeBig = Partition_Subsize[ PARTITION_VERT ][ bSize ]

     bsizeMed = Partition_Subsize[ PARTITION_VERT ][ bSizeBig ]

     decode_partition( r, c, subSize, bSize,

              chromaOffset, hasChroma && !chromaOffset,

              extendedSdpAllowed )

     decode_partition( r, c + (num4x4wide >> 3), bsizeMed, bSize,

              chromaOffset, hasChroma && !chromaOffset,

              extendedSdpAllowed )

     decode_partition( r, c + 3 * (num4x4wide >> 3), bSizeBig, bSize,

              chromaOffset, hasChroma && !chromaOffset,

              extendedSdpAllowed )

     decode_partition( r, c + 7 * (num4x4wide >> 3), subSize, bSize,

              chromaOffset, hasChroma, extendedSdpAllowed )

   } else if ( partition == PARTITION_VERT_4B ) {

     bSizeBig = Partition_Subsize[ PARTITION_VERT ][ bSize ]

     bsizeMed = Partition_Subsize[ PARTITION_VERT ][ bSizeBig ]

     decode_partition( r, c, subSize, bSize,

              chromaOffset, hasChroma && !chromaOffset,

              extendedSdpAllowed )

     decode_partition( r, c + (num4x4wide >> 3), bSizeBig, bSize,

              chromaOffset, hasChroma && !chromaOffset,

              extendedSdpAllowed )

     decode_partition( r, c + 5 * (num4x4wide >> 3), bsizeMed, bSize,

              chromaOffset, hasChroma && !chromaOffset,

              extendedSdpAllowed )

     decode_partition( r, c + 7 * (num4x4wide >> 3), subSize, bSize,

              chromaOffset, hasChroma, extendedSdpAllowed )

   } else if ( partition == PARTITION_SPLIT ) {




AV2 Specification                                                       Page 153 of 1169
       decode_partition( r, c, subSize, bSize, 0,

                hasChroma, extendedSdpAllowed )

       decode_partition( r, c + halfBlock4x4wide, subSize, bSize, 0,

                hasChroma, extendedSdpAllowed )

       decode_partition( r + halfBlock4x4high, c, subSize, bSize, 0,

                hasChroma, extendedSdpAllowed )

       decode_partition( r + halfBlock4x4high, c + halfBlock4x4wide,

                subSize, bSize, 0, hasChroma, extendedSdpAllowed )

   } else {

       decode_partition( r, c, subSize, bSize, chromaOffset,

                hasChroma && !chromaOffset, extendedSdpAllowed )

       middleChroma = bSize == BLOCK_32X8 && hasChroma && SubsamplingY

       if ( middleChroma ) {

           ChromaMiRow = r

           ChromaMiCol = c + (halfBlock4x4wide >> 1)

           ChromaMiSize = Partition_Subsize[ PARTITION_VERT ][ bSize ]

       }

       decode_partition( r, c + (halfBlock4x4wide >> 1),

                H_Partition_Midsize[ bSize ], bSize,

                chromaOffset || middleChroma,

                hasChroma && !chromaOffset && !middleChroma,

                extendedSdpAllowed )

       decode_partition( r + halfBlock4x4high, c + (halfBlock4x4wide >> 1),

                H_Partition_Midsize[ bSize ], bSize,

                chromaOffset || middleChroma,

                hasChroma && !chromaOffset, extendedSdpAllowed )

       decode_partition( r, c + 3 * (halfBlock4x4wide >> 1),

                subSize, bSize, chromaOffset,

                hasChroma, extendedSdpAllowed )

   }

   if ( FrameIsIntra && TreeType == LUMA_PART && bSize == BLOCK_64X64 ) {

       ChromaPartitionKnown[ r ][ c ] = ChromaFollowsLuma

   }

   if ( usingSdp ) {

       TreeType = CHROMA_PART

       HasChroma = 1

       PlaneStart = 1

       PlaneEnd = NumPlanes

       ChromaMiRow = r

       ChromaMiCol = c

       ChromaMiSize = bSize




AV2 Specification                                                             Page 154 of 1169
             AvailU = is_inside( r - 1, c )

             AvailL = is_inside( r, c - 1 )

             decode_block( r, c, bSize )

             TreeType = SHARED_PART

             PlaneStart = 0

             PlaneEnd = NumPlanes

             RegionType = MIXED_REGION

         }

     }


    The function is_bsize_allowed_for_extended_sdp is defined as:

     is_bsize_allowed_for_extended_sdp(bSize, partition) {
         bw = Block_Width[ bSize ]
         bh = Block_Height[ bSize ]
         return bw <= INTER_SDP_MAX_BLOCK_SIZE && bh <= INTER_SDP_MAX_BLOCK_SIZE &&
                bw >= 8 && bh >= 8 &&
                partition < PARTITION_HORZ_4A && partition != PARTITION_NONE
     }


```

<a id="s-5-20-3-2"></a>

##### § 5.20.3.2 Read partition syntax

```text
§   5.20.3.2. Read partition syntax

     Rect_Part_Table[ 2 ][ 2 ][ NUM_UNEVEN_4WAY_PARTS ][ NUM_RECT_PARTS ] = {
         {
             {
                 { PARTITION_HORZ, PARTITION_VERT },
                 { PARTITION_HORZ, PARTITION_VERT },
             },
             {
                 { PARTITION_HORZ, PARTITION_VERT },
                 { PARTITION_HORZ, PARTITION_VERT },
             },
         },
         {
             {
                 { PARTITION_HORZ_3, PARTITION_VERT_3 },
                 { PARTITION_HORZ_3, PARTITION_VERT_3 },
             },
             {
                 { PARTITION_HORZ_4A, PARTITION_VERT_4A },
                 { PARTITION_HORZ_4B, PARTITION_VERT_4B },
             }
         }
     }


     read_partition(r, c, bSize, chromaOffset, hasChroma) {                           Descriptor

         (implied,partition) = partition_implied(r, c, bSize)

         (numAllowed, allowed) = init_allowed_partitions( r, c, bSize,

                                chromaOffset, hasChroma )

         if ( implied && allowed[ partition ] ) {

             return partition

         }

         if ( numAllowed == 1 ) {

             for ( p = 0; p < EXT_PARTITION_TYPES; p++ ) {



    AV2 Specification                                                                 Page 155 of 1169
           if ( allowed[ p ] ) {

               return p

           }

       }

   }

   if ( bru_mode != BRU_ACTIVE ) {

       return PARTITION_NONE

   }

   if ( allowed[ PARTITION_NONE ] ) {

       do_split                                         S()

       if ( !do_split ) {

           return PARTITION_NONE

       }

   }

   if ( allowed[ PARTITION_SPLIT ] ) {

       do_square_split                                  S()

       if ( do_square_split ) {

           return PARTITION_SPLIT

       }

   }

   rectType = rect_type_implied_by_bsize( bSize )

   if ( rectType == RECT_INVALID ) {

       allowHorz = ( allowed[ PARTITION_HORZ ] ||

                 allowed[ PARTITION_HORZ_3 ] ||

                 allowed[ PARTITION_HORZ_4A ] ||

                 allowed[ PARTITION_HORZ_4B ] )

       allowVert = ( allowed[ PARTITION_VERT ] ||

                 allowed[ PARTITION_VERT_3 ] ||

                 allowed[ PARTITION_VERT_4A ] ||

                 allowed[ PARTITION_VERT_4B ] )

       if ( !allowHorz ) {

           rectType = RECT_VERT

       } else if ( !allowVert ) {

           rectType = RECT_HORZ

       }

   }

   if ( rectType == RECT_INVALID ) {

       rect_type                                        S()

       rectType = rect_type

   }

   if ( rectType == RECT_HORZ ) {




AV2 Specification                                   Page 156 of 1169
         nonExtAllowed = allowed[ PARTITION_HORZ ]

         extAllowed3 = allowed[ PARTITION_HORZ_3 ]

         extAllowed4 = allowed[ PARTITION_HORZ_4A ] ||

                 allowed[ PARTITION_HORZ_4B ]

     } else {

         nonExtAllowed = allowed[ PARTITION_VERT ]

         extAllowed3 = allowed[ PARTITION_VERT_3 ]

         extAllowed4 = allowed[ PARTITION_VERT_4A ] ||

                 allowed[ PARTITION_VERT_4B ]

     }

     if ( nonExtAllowed && ( extAllowed3 || extAllowed4 ) ) {

         do_ext_partition                                                                             S()

     } else {

         do_ext_partition = extAllowed3 || extAllowed4

     }

     do_uneven_4way_partition = 0

     uneven_4way_partition_type = 0

     if ( do_ext_partition ) {

         if ( extAllowed3 && extAllowed4 ) {

             do_uneven_4way_partition                                                                 S()

         } else {

             do_uneven_4way_partition = extAllowed4

         }

         if ( do_uneven_4way_partition ) {

             uneven_4way_partition_type                                                              L(1)

         }

     }

     return Rect_Part_Table[ do_ext_partition ][ do_uneven_4way_partition ]

                  [ uneven_4way_partition_type ][ rectType ]

 }


where init_allowed_partitions, is_partition_allowed, is_chroma_offset_for_partition,
is_chroma_offset_for_subsize, check_chroma, block_coded, rect_type_implied_by_bsize, is_ext_partition_allowed,
partition_implied_at_bo undary, partition_implied, and is_uneven_4way_partition_allowed are functions defined
as:

 block_coded(r,c) {
     return r < MiRows && c < MiCols
 }


 check_chroma(bSize) {
     if ( get_plane_residual_size( bSize, 1 ) == BLOCK_INVALID ) {
         return 0
     }
     return ( TreeType == LUMA_PART &&




AV2 Specification                                                                                 Page 157 of 1169
                Block_Width[ bSize ] >= 64 &&
                Block_Height[ bSize ] >= 64 )
 }


 is_chroma_offset_for_subsize( subSize ) {
     if ( SubsamplingY && Mi_Height_Log2[ subSize ] == 0 ) {
         return 1
     }
     if ( SubsamplingX && Mi_Width_Log2[ subSize ] == 0 ) {
         return 1
     }
     return 0
 }


 is_chroma_offset_for_partition( p, bSize ) {
     if ( is_chroma_offset_for_subsize( Partition_Subsize[ p ][ bSize ] ) ) {
         return 1
     }
     if ( p == PARTITION_HORZ_3 ) {
         middleChroma = bSize == BLOCK_8X32 && SubsamplingX
         if ( !middleChroma ) {
              if ( is_chroma_offset_for_subsize( H_Partition_Midsize[bSize] ) ) {
                  return 1
              }
         }
     }
     return 0
 }


 is_partition_allowed(r,c,p,bSize,chromaOffset,hasChroma,numPlanes) {
     subSize = Partition_Subsize[ p ][ bSize ]
     if ( subSize == BLOCK_INVALID ) {
         return 0
     }
     if ( !FrameIsIntra && RegionType == MIXED_REGION && subSize == BLOCK_4X4 ) {
         return 0
     }
     rectType = rect_type_implied_by_bsize( bSize )
     if ( rectType == RECT_VERT &&
             (p == PARTITION_HORZ ||
             p == PARTITION_HORZ_3 ||
             p == PARTITION_HORZ_4A ||
             p == PARTITION_HORZ_4B) ) {
         return 0
     }
     if ( rectType == RECT_HORZ &&
             (p == PARTITION_VERT ||
             p == PARTITION_VERT_3 ||
             p == PARTITION_VERT_4A ||
             p == PARTITION_VERT_4B) ) {
         return 0
     }
     bw = Block_Width[ subSize ]
     bh = Block_Height[ subSize ]
     if ( bw > bh * MaxPbAspectRatio || bh > bw * MaxPbAspectRatio ) {
         if (p == PARTITION_NONE) {
             return 0
         }
         if ( bw >= bh * 8 || bh >= bw * 8 ) {
             return 0
         }
     }
     num4x4wide = Num_4x4_Blocks_Wide[ bSize ]
     num4x4high = Num_4x4_Blocks_High[ bSize ]
     halfBlock4x4wide = num4x4wide >> 1
     halfBlock4x4high = num4x4high >> 1
     if ( hasChroma && TreeType != CHROMA_PART ) {
         if ( !chromaOffset ) {



AV2 Specification                                                                   Page 158 of 1169
                chromaOffset = is_chroma_offset_for_partition( p, bSize )
           }
      }
      if ( (hasChroma && !chromaOffset && TreeType != LUMA_PART) ||
            check_chroma(bSize) ) {
          if ( get_plane_residual_size( subSize, 1 ) == BLOCK_INVALID ) {
               return 0
          }
      }
      if ( p == PARTITION_HORZ_3 ) {
          if ( !is_ext_partition_allowed( bSize, RECT_HORZ) ) {
               return 0
          }
      } else if ( p == PARTITION_VERT_3 ) {
          if ( !is_ext_partition_allowed( bSize, RECT_VERT) ) {
               return 0
          }
      } else if ( p == PARTITION_HORZ_4A || p == PARTITION_HORZ_4B ) {
          if ( !is_ext_partition_allowed( bSize, RECT_HORZ) ||
                 !is_uneven_4way_partition_allowed( bSize, RECT_HORZ ) ) {
               return 0
          }
      } else if ( p == PARTITION_VERT_4A || p == PARTITION_VERT_4B ) {
          if ( !is_ext_partition_allowed( bSize, RECT_VERT) ||
                 !is_uneven_4way_partition_allowed( bSize, RECT_VERT ) ) {
               return 0
          }
      } else if ( p == PARTITION_NONE ) {
          hasRows = ( r + halfBlock4x4high ) < MiRows
          hasCols = ( c + halfBlock4x4wide ) < MiCols
          if ( (TreeType != CHROMA_PART || bSize != BLOCK_8X8) &&
                 (!hasRows || !hasCols) ) {
               return 0
          }
      }
      if ( hasChroma && TreeType != LUMA_PART && numPlanes > 1 ) {
          if ( chromaOffset ) {
               if ( p == PARTITION_HORZ ) {
                    return block_coded( r + halfBlock4x4high, c )
               } else if ( p == PARTITION_VERT ) {
                    return block_coded( r, c + halfBlock4x4wide )
               } else if ( p == PARTITION_HORZ_3 ) {
                    return block_coded( r + 3 * (halfBlock4x4high >> 1), c )
               } else if ( p == PARTITION_VERT_3 ) {
                    return block_coded( r, c + 3 * (halfBlock4x4wide >> 1) )
               } else if ( p == PARTITION_HORZ_4A || p == PARTITION_HORZ_4B ) {
                    h4 = Num_4x4_Blocks_High[ subSize ]
                    return block_coded( r + 7 * h4, c )
               } else if ( p == PARTITION_VERT_4A || p == PARTITION_VERT_4B ) {
                    w4 = Num_4x4_Blocks_Wide[ subSize ]
                    return block_coded( r, c + 7 * w4 )
               }
          }
      }
      return 1
 }


 init_allowed_partitions(r,c,bSize,chromaOffset,hasChroma) {
     numAllowed = 0
     for ( p = 0; p < EXT_PARTITION_TYPES; p++ ) {
         good = is_partition_allowed(r,c,p,bSize,chromaOffset,
                                     hasChroma,NumPlanes)
         numAllowed += good
         allowed[ p ] = good
     }
     if ( numAllowed == 0 ) {
         allowed[ PARTITION_NONE ] = 1
         numAllowed = 1




AV2 Specification                                                                 Page 159 of 1169
      }
      return (numAllowed,allowed)
 }


 rect_type_implied_by_bsize(bSize) {
     if ( bSize == BLOCK_4X8 || bSize == BLOCK_64X128 ||
         bSize == BLOCK_128X256 || bSize == BLOCK_4X16 ) {
         return RECT_HORZ
     }
     if ( bSize == BLOCK_8X4 || bSize == BLOCK_128X64 ||
         bSize == BLOCK_256X128 || bSize == BLOCK_16X4 ) {
         return RECT_VERT
     }
     if ( TreeType == CHROMA_PART ) {
         if ( bSize == BLOCK_8X16 || bSize == BLOCK_8X32 ) {
             return RECT_HORZ
         }
         if ( bSize == BLOCK_16X8 || bSize == BLOCK_32X8 ) {
             return RECT_VERT
         }
     }
     return RECT_INVALID
 }


 is_ext_partition_allowed(bSize, rectType) {
     if ( !enable_ext_partitions ) {
         return 0
     }
     return TreeType != CHROMA_PART ||
         (rectType == RECT_HORZ &&
             Block_Height[ bSize ] > 16 && Block_Width[ bSize ] > 8) ||
         (rectType == RECT_VERT &&
             Block_Width[ bSize ] > 16 && Block_Height[ bSize ] > 8)
 }


 partition_implied_at_boundary(r, c, bSize) {
     numWide4x4 = Num_4x4_Blocks_Wide[ bSize ]
     numHigh4x4 = Num_4x4_Blocks_High[ bSize ]
     hasRows = ( r + (numHigh4x4 >> 1) ) < MiRows
     hasCols = ( c + (numWide4x4 >> 1) ) < MiCols
     if ( hasRows && hasCols ) {
         return (0, PARTITION_NONE)
     }
     impliedPartition = PARTITION_NONE
     if ( numWide4x4 == numHigh4x4 ) {
         impliedPartition = hasRows ? PARTITION_VERT : PARTITION_HORZ
     } else if ( numHigh4x4 > numWide4x4 ) {
         if ( !hasRows ) {
              impliedPartition = PARTITION_HORZ
         } else {
              subHasCols = ( c + (numWide4x4 >> 2) ) < MiCols
              if ( numWide4x4 >= 4 && !subHasCols ) {
                  impliedPartition = PARTITION_HORZ
              }
         }
     } else {
         if ( !hasCols ) {
              impliedPartition = PARTITION_VERT
         } else {
              subHasRows = ( r + (numHigh4x4 >> 2) ) < MiRows
              if ( numHigh4x4 >= 4 && !subHasRows ) {
                  impliedPartition = PARTITION_VERT
              }
         }




AV2 Specification                                                         Page 160 of 1169
                 }
                 return (impliedPartition != PARTITION_NONE, impliedPartition)
     }


     partition_implied(r, c, bSize) {
         if ( bSize == BLOCK_4X4 || bSize >= BLOCK_4X32 ) {
             return (1, PARTITION_NONE)
         }
         if ( TreeType == CHROMA_PART && bSize == BLOCK_8X8 ) {
             return (1, PARTITION_NONE)
         }
         if ( TreeType == CHROMA_PART && bSize == BLOCK_64X64 &&
              ChromaPartitionKnown[ r ][ c ] ) {
             return (1, LumaPartitions[ r ][ c ])
         }
         return partition_implied_at_boundary(r, c, bSize)
     }


     is_uneven_4way_partition_allowed(bSize, rectType) {
         if ( !enable_uneven_4way_partitions ) {
             return 0
         }
         return TreeType != CHROMA_PART ||
                (rectType == RECT_HORZ && Block_Height[ bSize ] == 64) ||
                (rectType == RECT_VERT && Block_Width[ bSize ] == 64)
     }


```

<a id="s-5-20-4"></a>

#### § 5.20.4 Block decoding structures

```text
§   5.20.4. Block decoding structures

```

<a id="s-5-20-4-1"></a>

##### § 5.20.4.1 Decode block syntax

```text
§   5.20.4.1. Decode block syntax

     decode_block( r, c, subSize ) {                                             Descriptor

         MiRow = r

         MiCol = c

         MiSize = subSize

         bw4 = Num_4x4_Blocks_Wide[ subSize ]

         bh4 = Num_4x4_Blocks_High[ subSize ]

         update_ibc_buffers(r, c)

         for ( y = 0; y < bh4; y++ ) {

             for ( x = 0; x < bw4; x++ ) {

                 IBCCoded[ r + y ][ x + c ] = 1

             }

         }

         if ( HasChroma ) {

             AvailUChroma = is_inside( ChromaMiRow - 1, ChromaMiCol )

             AvailLChroma = is_inside( ChromaMiRow, ChromaMiCol - 1 )

         } else {

             AvailUChroma = 0

             AvailLChroma = 0

         }

         NNum = 0

         NNumBuf = 0



    AV2 Specification                                                            Page 161 of 1169
   add_neighbor( r + bh4 - 1, c - 1 )

   add_neighbor( r - 1, c + bw4 - 1 )

   add_neighbor( r, c - 1 )

   add_neighbor( r - 1, c )

   for ( n = 0; n < NNumBuf; n++ ) {

       for ( list = 0; list < 2; list++ ) {

           NRefFrame[ n ][ list ] =

            RefFrames[ NPosBuf[ n ][ 0 ] ][ NPosBuf[ n ][ 1 ] ][ list ]

       }

       NIntra[ n ] = !IsInters[ NPosBuf[ n ][ 0 ] ][ NPosBuf[ n ][ 1 ] ]

       NSingle[ n ] = !is_inter_ref_frame( NRefFrame[ n ][ 1 ] )

   }

   mode_info( )

   palette_tokens( )

   if ( TreeType != CHROMA_PART ) {

       read_block_tx_size( )

   }

   if ( skip_flag ) {

       reset_block_context( bw4, bh4 )

   }

   isCompound = is_inter_ref_frame(RefFrame[ 1 ])

   for ( y = 0; y < bh4; y++ ) {

       for ( x = 0; x < bw4; x++ ) {

           if ( PlaneStart == 0 ) {

            IntraJointModes[ r + y ][ c + x ] = IntraJointMode

            YModes [ r + y ][ c + x ] = YMode

            AngleDeltaYs[ r + y ][ c + x ] = AngleDeltaY

            for ( refList = 0; refList < 2; refList++ ) {

                RefFrames[ r + y ][ c + x ][ refList ] = RefFrame[ refList ]

            }

            MiSizes[ 0 ][ r + y ][ c + x ] = MiSize

            w = bw4 * 4

            h = bh4 * 4

            TipSizes16x16[ r + y ][ c + x ] = enable_tip_refinemv ?

                              (w == 256 && h == 256) :

                              (w >= 16 && h >= 16)

            LeftMiSizes[ 0 ][ r + y ] = MiSize

            AboveMiSizes[ 0 ][ c + x ] = MiSize

            MiColStartGrid[ r + y ][ c + x ] = MiColStart

            MiRowStartGrid[ r + y ][ c + x ] = MiRowStart

            MiColEndGrid[ r + y ][ c + x ] = MiColEnd




AV2 Specification                                                              Page 162 of 1169
               MiRowEndGrid[ r + y ][ c + x ] = MiRowEnd

               MiColBase[ 0 ][ r + y ][ c + x ] = MiCol

               MiRowBase[ 0 ][ r + y ][ c + x ] = MiRow

               if ( is_inter ) {

                   if ( !use_intrabc ) {

                       CompGroupIdxs[ r + y ][ c + x ] = comp_group_idx

                   }

                   InterpFilters[ r + y ][ c + x ] = interp_filter

                   for ( refList = 0; refList < 1 + isCompound; refList++ ) {

                       Mvs[r + y][c + x][refList] = BlockMvs[refList]

                       SubMvs[r + y][c + x][refList] = BlockMvs[refList]

                   }

               }

               SubPuColBase[ 0 ][ r + y ][ c + x ] = c

               SubPuRowBase[ 0 ][ r + y ][ c + x ] = r

               SubPuSize[ 0 ][ r + y ][ c + x ] = Max_Tx_Size_Rect[ MiSize ]

           }

       }

   }

   if ( HasChroma ) {

       uvSmooth = !is_inter && (UVMode == SMOOTH_PRED ||

                   UVMode == SMOOTH_V_PRED || UVMode == SMOOTH_H_PRED)

       for ( y = 0; y < Num_4x4_Blocks_High[ ChromaMiSize ]; y++ ) {

           for ( x = 0; x < Num_4x4_Blocks_Wide[ ChromaMiSize ]; x++ ) {

               MiSizes[ 1 ][ ChromaMiRow + y ][ ChromaMiCol + x ] =

                   ChromaMiSize

               LeftMiSizes[ 1 ][ ChromaMiRow + y ] = ChromaMiSize

               AboveMiSizes[ 1 ][ ChromaMiCol + x ] = ChromaMiSize

               MiColBase[ 1 ][ ChromaMiRow + y ][ ChromaMiCol + x ] =

                   ChromaMiCol

               MiRowBase[ 1 ][ ChromaMiRow + y ][ ChromaMiCol + x ] =

                   ChromaMiRow

               UVSmooth[ ChromaMiRow + y ][ ChromaMiCol + x ] = uvSmooth

               UVCfls[ ChromaMiRow + y ][ ChromaMiCol + x ] =

                   !is_inter && (UVMode == UV_CFL_PRED)

               SubPuColBase[ 1 ][ ChromaMiRow + y ][ ChromaMiCol + x ] =

                   ChromaMiCol

               SubPuRowBase[ 1 ][ ChromaMiRow + y ][ ChromaMiCol + x ] =

                   ChromaMiRow

               SubPuSize[ 1 ][ ChromaMiRow + y ][ ChromaMiCol + x ] =

                   Max_Tx_Size_Rect[ ChromaMiSize ]




AV2 Specification                                                               Page 163 of 1169
               RegionTypes[ ChromaMiRow + y ][ ChromaMiCol + x ] =

                   RegionType

               ChromaSegmentIds[ ChromaMiRow + y ][ ChromaMiCol + x ] =

                   segment_id

               ChromaQIndex[ ChromaMiRow + y ][ ChromaMiCol + x ] =

                   CurrentQIndex

           }

       }

   }

   compute_prediction( )

   residual( )

   if ( is_inter && motion_mode >= LOCALWARP ) {

       update_warp_param_bank( RefFrame, LocalWarpParams, 0 )

   }

   if ( enable_refmvbank &&

       bru_mode == BRU_ACTIVE &&

       RefMvBankHits < MAX_RMB_SB_HITS ) {

       if ( is_inter ) {

           update_ref_mv_bank( RefFrame, BlockMvs, CwpIdx, 1 )

       } else {

           update_ref_mv_count( )

       }

   }

   for ( y = 0; y < bh4; y++ ) {

       for ( x = 0; x < bw4; x++ ) {

           if ( PlaneStart == 0 ) {

               for ( refList = 0;refList < 1 + isCompound; refList++ ) {

                   for ( i = 0; i < 6; i++ ) {

                       WarpParams[ r + y ][ c + x ][ refList ][ i ] =

                        LocalWarpParams[ refList ][ i ]

                   }

               }

               IsInters[ r + y ][ c + x ] = is_inter

               SkipModes[ r + y ][ c + x ] = skip_mode

               Skips[ r + y ][ c + x ] = skip_flag

               CwpIdxs[ r + y ][ c + x ] = CwpIdx

               FscModes[ r + y ][ c + x ] = fsc_mode

               UsesMrls[ r + y ][ c + x ] =

                   (mrl_index > 0 ? ( mrl_sec_index ? 2 : 1) : 0)

               UsesAmvds[ r + y ][ c + x ] = use_amvd

               UseDip[ r + y ][ c + x ] = use_dip




AV2 Specification                                                          Page 164 of 1169
                 UseMostProbablePrecisions[ r + y ][ c + x ] =

                     use_most_probable_precision

                 MvPrecisions[ r + y ][ c + x ] =

                     use_intrabc ? FrameMvPrecision : MvPrecision

                 MorphPreds[ r + y ][ c + x ] = use_intrabc && morph_pred

                 SegmentIds[ r + y ][ c + x ] = segment_id

                 PaletteSizes[ r + y ][ c + x ] = PaletteSizeY

                 for ( i = 0; i < PaletteSizeY; i++ ) {

                     PaletteColors[ r + y ][ c + x ][ i ] =

                      palette_colors_y[ i ]

                 }

                 MotionModes[ r + y ][ c + x ] = motion_mode

                 LumaQIndex[ r + y ][ c + x ] = CurrentQIndex

             }

         }

     }

     if ( PlaneStart == 0 ) {

         if ( isCompound && opfl_allowed_for_refs( RefFrame ) && use_optflow ) {

             motion_field_motion_vector_storage(r, c, subSize, 1)

         } else if ( isCompound && compound_type == COMPOUND_AVERAGE &&

                     use_refinemv ) {

             motion_field_motion_vector_storage(r, c, subSize, 2)

         } else if ( RefFrame[ 0 ] == TIP_FRAME ) {

             if ( store_refined_mvs() ) {

                 motion_field_motion_vector_storage(r, c, subSize,

                     LumaUseOptflowRefinement ? 1 : 2 )

             } else {

                 motion_field_motion_vector_storage(r, c, subSize, 0)

             }

         } else {

             motion_field_motion_vector_storage(r, c, subSize, 0)

         }

     }

 }


where reset_block_context( ) is specified as:

 reset_block_context( bw4, bh4 ) {
     for ( plane = 0; plane < 1 + 2 * HasChroma; plane++ ) {
         c = plane > 0 ? ChromaMiCol : MiCol
         r = plane > 0 ? ChromaMiRow : MiRow
         w4 = plane > 0 ? Num_4x4_Blocks_Wide[ ChromaMiSize ] : bw4
         h4 = plane > 0 ? Num_4x4_Blocks_High[ ChromaMiSize ] : bh4
         subX = plane > 0 ? SubsamplingX : 0
         subY = plane > 0 ? SubsamplingY : 0



AV2 Specification                                                                  Page 165 of 1169
           for ( i = c >> subX; i < ( ( c + w4 ) >> subX ); i++) {
               AboveLevelContext[ plane ][ i ] = 0
               AboveDcContext[ plane ][ i ] = 0
           }
           for ( i = r >> subY; i < ( ( r + h4 ) >> subY ); i++) {
               LeftLevelContext[ plane ][ i ] = 0
               LeftDcContext[ plane ][ i ] = 0
           }
      }
 }


update_warp_param_bank is specified as:


 update_warp_param_bank( refFrames , params, candFromSbAbove ) {
     isCompound = is_inter_ref_frame( refFrames[ 1 ] ) && !candFromSbAbove
     for ( refList = 0;refList < 1 + isCompound; refList++ ) {
         if ( WarpBankHits >= MAX_WARP_SB_HITS ) {
             return
         }
         WarpBankHits++
         ref = refFrames[ refList ]
         found = -1
         count = WarpBankSize[ ref ]
         start = WarpBankStart[ ref ]
         for ( i = 0; i < count; i++ ) {
             idx = (start + i) % WARP_PARAM_BANK_SIZE
             if ( params_equal( WarpBankParams[ ref ][ idx ],
                                  params[ refList ] ) ) {
                  found = i
                  break
             }
         }
         if ( found >= 0 ) {
             for ( j = 0; j < 6; j++ ) {
                  tmpParams[ j ] = WarpBankParams[ ref ][ idx ][ j ]
             }
             for ( i = found; i < count - 1; i++ ) {
                  idx0 = (start + i) % WARP_PARAM_BANK_SIZE
                  idx1 = (start + i + 1) % WARP_PARAM_BANK_SIZE
                  for ( j = 0; j < 6; j++ ) {
                      WarpBankParams[ ref ][ idx0 ][ j ] =
                          WarpBankParams[ ref ][ idx1 ][ j ]
                  }
             }
             tail = (start + count - 1) % WARP_PARAM_BANK_SIZE
             for ( j = 0; j < 6; j++ ) {
                  WarpBankParams[ ref ][ tail ][ j ] = tmpParams[ j ]
             }
         } else {
             idx = (start + count) % WARP_PARAM_BANK_SIZE
             for ( j = 0; j < 6; j++ ) {
                  WarpBankParams[ ref ][ idx ][ j ] = params[ refList ][ j ]
             }
             if ( count < WARP_PARAM_BANK_SIZE ) {
                  WarpBankSize[ ref ] = count + 1
             } else {
                  WarpBankStart[ ref ] = (start + 1) % WARP_PARAM_BANK_SIZE
             }
         }
     }
 }




AV2 Specification                                                              Page 166 of 1169
The function params_equal (which checks if the non-translational parts of two warps are equal) is defined
as:

 params_equal( paramsA , paramsB ) {
     for ( i = 2; i < 6; i++ ) {
         if ( paramsA[ i ] != paramsB[ i ]) {
              return 0
         }
     }
     return 1
 }


update_ref_mv_bank (which ensures the current parameters are at the tail of the appropriate bank of motion
vectors) is specified as:

 update_ref_mv_bank( refFrames, mvs, cwpIdx, fromWithinSb ) {
     if ( fromWithinSb ) {
         update_ref_mv_count( )
         if ( RefMvRemainHits == 0 || RefMvUnitHits >= 16 ) {
             return
         }
         RefMvRemainHits--
         RefMvUnitHits++
     }
     RefMvBankHits++
     r0 = refFrames[ 0 ]
     r1 = refFrames[ 1 ]
     isCompound = is_inter_ref_frame(r1)
     ref = get_rmb_list_index( refFrames )
     for ( i = 0; i < 6; i++ ) {
         p[ i ] = 0
     }
     p[ 0 ] = cwpIdx
     p[ 1 ] = isCompound ? r0 + (r1 + 1) * BANK_REFS_PER_FRAME : r0
     p[ 2 ] = mvs[ 0 ][ 0 ]
     p[ 3 ] = mvs[ 0 ][ 1 ]
     if ( isCompound ) {
         p[ 4 ] = mvs[ 1 ][ 0 ]
         p[ 5 ] = mvs[ 1 ][ 1 ]
     }
     found = -1
     count = RefMvBankSize[ ref ]
     start = RefMvBankStart[ ref ]
     for ( i = 0; i < count; i++ ) {
         idx = (start + i) % REF_MV_BANK_SIZE
         if ( rmb_params_equal(RefMvBankParams[ ref ][ idx ],p) ) {
             found = i
             break
         }
     }
     if ( found >= 0 ) {
         for ( i = 0; i < 6; i++ ) {
             tmpParams[ i ] = RefMvBankParams[ ref ][ idx ][ i ]
         }
         for ( i = found; i < count - 1; i++ ) {
             idx0 = (start + i) % REF_MV_BANK_SIZE
             idx1 = (start + i + 1) % REF_MV_BANK_SIZE
             for ( j = 0; j < 6; j++ ) {
                 RefMvBankParams[ ref ][ idx0 ][ j ] =
                     RefMvBankParams[ ref ][ idx1 ][ j ]
             }
         }
         tail = (start + count - 1) % REF_MV_BANK_SIZE
         for ( j = 0; j < 6; j++ ) {
             RefMvBankParams[ ref ][ tail ][ j ] = tmpParams[ j ]
         }



AV2 Specification                                                                            Page 167 of 1169
          return
      }
      idx = (start + count) % REF_MV_BANK_SIZE
      for ( j = 0; j < 6; j++ ) {
          RefMvBankParams[ ref ][ idx ][ j ] = p[ j ]
      }
      if ( count < REF_MV_BANK_SIZE ) {
          RefMvBankSize[ ref ] = count + 1
      } else {
          RefMvBankStart[ ref ] = (start + 1) % REF_MV_BANK_SIZE
      }
 }


add_neighbor is specified as:


 add_neighbor(nRow, nCol) {
     aboveSbBoundary = (MiRow >> Mi_Width_Log2[ SbSize ]) !=
                       (nRow >> Mi_Width_Log2[ SbSize ])
     if ( NNum < 2 && is_inside(nRow,nCol) && !aboveSbBoundary ) {
         NPos[ NNum ][ 0 ] = nRow
         NPos[ NNum ][ 1 ] = nCol
         NNum += 1
     }
     if ( NNumBuf < 2 && is_inside(nRow,nCol) ) {
         NPosBuf[ NNumBuf ][ 0 ] = nRow
         NPosBuf[ NNumBuf ][ 1 ] = nCol
         NNumBuf += 1
     }
 }



  NOTE: NPos will only contain locations that are in the same superblock row as the current block.
  NPosBuf contains locations that may require buffered access to a different superblock row.


update_ref_mv_count is specified as:


 update_ref_mv_count() {
     if ( TreeType != CHROMA_PART ) {
         sbSize4 = Num_4x4_Blocks_Wide[ SbSize ]
         unitSize4 = sbSize4 >> 3
         unitCount = Max( Num_4x4_Blocks_Wide[ MiSize ] / unitSize4 , 1) *
                     Max( Num_4x4_Blocks_High[ MiSize ] / unitSize4 , 1)
         if ( MiRow % sbSize4 == 0 && MiCol % sbSize4 == 0 ) {
             RefMvRemainHits = Max( unitCount , 4 )
             RefMvUnitHits = 0
         } else if ( MiRow % unitSize4 == 0 && MiCol % unitSize4 == 0 ) {
             RefMvRemainHits += unitCount
             RefMvUnitHits = 0
         }
     }
 }


rmb_params_equal is specified as:



 rmb_params_equal( paramsA, paramsB ) {
     for ( i = 1; i < 6; i++ ) {
         if ( paramsA[ i ] != paramsB[ i ] ) {
              return 0
         }
     }
     return 1
 }




AV2 Specification                                                                        Page 168 of 1169
    update_ibc_buffers is specified as:


     update_ibc_buffers(miRow, miCol) {
         bufRow = miRow >> (IBC_BUFFER_SIZE_LOG2 - MI_SIZE_LOG2)
         bufCol = miCol >> (IBC_BUFFER_SIZE_LOG2 - MI_SIZE_LOG2)
         if ( bufRow != IBCBufferCurRow || bufCol != IBCBufferCurCol ) {
             blkIdx = ibc_buffer_index(IBCBufferCurRow, IBCBufferCurCol)
             IBCBufferRow[ blkIdx ] = IBCBufferCurRow
             IBCBufferCol[ blkIdx ] = IBCBufferCurCol
             IBCBufferValid[ blkIdx ] = 1
             if ( SbSize == BLOCK_64X64 ) {
                 bruRow = IBCBufferCurRow << (IBC_BUFFER_SIZE_LOG2 - MI_SIZE_LOG2)
                 bruCol = IBCBufferCurCol << (IBC_BUFFER_SIZE_LOG2 - MI_SIZE_LOG2)
                 if ( BruModes[ bruRow ][ bruCol ] == BRU_INACTIVE ) {
                     for ( i = 0; i < IBC_NUM_BUFFERS; i++ ) {
                         IBCBufferValid[ i ] = 0
                     }
                 }
             }
             IBCBufferCurRow = bufRow
             IBCBufferCurCol = bufCol
         }
     }



      NOTE: Calls to update_ibc_buffers are only needed for bitstream conformance checks. However, a
      decoder implementation may wish to use the same logic for updating a local cache of information
      available for intra block copy.


    ibc_buffer_index is specified as:


     ibc_buffer_index(row, col) {
         if ( SbSize == BLOCK_64X64 ) {
             return col & 3
         } else {
             return (col & 1) | ((row & 1) << 1)
         }
     }


    store_refined_mvs is specified as:


     store_refined_mvs() {
         return Tip_Weighting_Factor[ tip_global_wtd_index ] == CWP_EQUAL &&
                enable_tip_refinemv && NumFutureRefs > 0 && NumPastRefs > 0
     }


```

<a id="s-5-20-5"></a>

#### § 5.20.5 Mode information structures

```text
§   5.20.5. Mode information structures

```

<a id="s-5-20-5-1"></a>

##### § 5.20.5.1 Mode info syntax

```text
§   5.20.5.1. Mode info syntax

     mode_info( ) {                                                                         Descriptor

       if ( bru_mode != BRU_ACTIVE ) {

         bru_mode_info( )

       } else if ( FrameIsIntra ) {

         intra_frame_mode_info( )

       } else {

         inter_frame_mode_info( )



    AV2 Specification                                                                      Page 169 of 1169
         }

     }


```

<a id="s-5-20-5-2"></a>

##### § 5.20.5.2 BRU mode info syntax

```text
§   5.20.5.2. BRU mode info syntax

     bru_mode_info( ) {                                                        Descriptor

         use_intrabc = 0

         skip_flag = 1

         segment_id = 0

         Lossless = LosslessArray[ segment_id ]

         skip_mode = 0

         is_inter = 1

         RefFrame[ 0 ] = IsBridge ? 0 : bru_ref

         RefFrame[ 1 ] = NONE

         mrl_index = 0

         use_dip = 0

         fsc_mode = 0

         use_dpcm_y = 0

         use_dpcm_uv = 0

         PaletteSizeY = 0

         MvPrecision = MV_PRECISION_ONE_PEL

         use_most_probable_precision = 0

         IntraJointMode = DC_PRED

         use_bawp = 0

         use_amvd = 0

         CwpIdx = CWP_EQUAL

         CurrentQIndex = base_q_idx

         use_optflow = 0

         use_refinemv = 0

         YMode = NEWMV

         motion_mode = SIMPLE

         BlockMvs[ 0 ][ 0 ] = 0

         BlockMvs[ 0 ][ 1 ] = 0

         interp_filter = EIGHTTAP_SHARP

         read_gdf( )

         read_ccso( )

         for ( r = MiRow; r < MiRow + Num_4x4_Blocks_High[ MiSize ]; r++ ) {

             LeftSegPredContext[ r ] = 0

         }

         for ( c = MiCol; c < MiCol + Num_4x4_Blocks_Wide[ MiSize ]; c++ ) {

             AboveSegPredContext[ c ] = 0

         }




    AV2 Specification                                                          Page 170 of 1169
     }


```

<a id="s-5-20-5-3"></a>

##### § 5.20.5.3 Intra frame mode info syntax

```text
§   5.20.5.3. Intra frame mode info syntax

     intra_frame_mode_info( ) {                             Descriptor

         skip_flag = 0

         if ( SegIdPreSkip ) {

             intra_segment_id( )

         }

         skip_mode = 0

         use_most_probable_precision = 0

         MvPrecision = FrameMvPrecision

         CwpIdx = CWP_EQUAL

         motion_mode = SIMPLE

         if ( allow_intrabc && TreeType != CHROMA_PART &&

             Block_Width[ MiSize ] <= 64 &&

             Block_Height[ MiSize ] <= 64 &&

             MiSize != BLOCK_64X64 ) {

             use_intrabc                                        S()

         } else {

             use_intrabc = 0

         }

         if ( use_intrabc ) {

             read_skip( )

         } else {

             skip_flag = 0

         }

         if ( !SegIdPreSkip ) {

             intra_segment_id( )

         }

         if ( TreeType != CHROMA_PART ) {

             read_gdf( )

             read_cdef( )

             read_ccso( )

             read_delta_qindex( )

         }

         ReadDeltas = 0

         RefFrame[ 0 ] = INTRA_FRAME

         RefFrame[ 1 ] = NONE

         fsc_mode = 0

         if ( use_intrabc ) {

             is_inter = 1




    AV2 Specification                                       Page 171 of 1169
             mrl_index = 0

             read_intrabc_info()

         } else {

             is_inter = 0

             PaletteSizeY = 0

             if ( TreeType != CHROMA_PART ) {

                 read_intra_y_mode()

             } else {

                 YMode = YModes[ MiRow ][ MiCol ]

                 AngleDeltaY = AngleDeltaYs[ MiRow ][ MiCol ]

                 PaletteSizeY = PaletteSizes[ MiRow ][ MiCol ]

             }

             if ( HasChroma ) {

                 read_intra_uv_mode()

                 if ( UVMode == UV_CFL_PRED ) {

                     read_cfl_alphas( )

                 }

             }

             if ( MiSize >= BLOCK_8X8 &&

                 Block_Width[ MiSize ] <= 64   &&

                 Block_Height[ MiSize ] <= 64 &&

                 allow_screen_content_tools ) {

                 palette_mode_info( )

             }

             if ( TreeType != CHROMA_PART ) {

                 dip_mode_info( )

             }

         }

     }


```

<a id="s-5-20-5-4"></a>

##### § 5.20.5.4 Read intra block copy syntax

```text
§   5.20.5.4. Read intra block copy syntax

     read_intrabc_info() {                                       Descriptor

         IntraJointMode = DC_PRED

         mrl_index = 0

         use_dip = 0

         fsc_mode = 0

         AngleDeltaY = 0

         use_bawp = 0

         use_amvd = 0

         warpmv_with_mvd = 0

         use_refinemv = 0




    AV2 Specification                                            Page 172 of 1169
     DecidedAgainstRefinemv = 0

     use_dpcm_y = 0

     use_dpcm_uv = 0

     CwpIdx = CWP_EQUAL

     YMode = DC_PRED

     UVMode = DC_PRED

     motion_mode = SIMPLE

     compound_type = COMPOUND_AVERAGE

     PaletteSizeY = 0

     interp_filter = BILINEAR

     RefFrame[ 0 ] = INTRA_FRAME

     RefFrame[ 1 ] = NONE

     MvPrecision = force_integer_mv ? MV_PRECISION_ONE_PEL :

                       MV_PRECISION_QUARTER_PEL

     use_most_probable_precision = 0

     DeriveWrl = 0

     IsAdaptiveMvd = 0

     find_mv_stack( 0 )

     m = max_bvp_drl_bits_minus_1 + 1

     intrabc_mode                                                             S()

     RefMvIdx = 0

     for ( idx = 0; idx < m; idx++ ) {

         intrabc_drl_mode                                                    L(1)

         if ( intrabc_drl_mode == 0 ) {

             RefMvIdx = idx

             break

         }

         RefMvIdx = idx + 1

     }

     if ( intrabc_mode == 0 && !force_integer_mv ) {

         intrabc_precision                                                    S()

         MvPrecision = intrabc_precision ? MV_PRECISION_QUARTER_PEL :

                          MV_PRECISION_ONE_PEL

     }

     assign_mv( 0 )

     if ( FrameIsIntra && allow_screen_content_tools && enable_bawp ) {

         morph_pred                                                           S()

     } else {

         morph_pred = 0

     }

 }




AV2 Specification                                                         Page 173 of 1169
```

<a id="s-5-20-5-5"></a>

##### § 5.20.5.5 Read intra Y mode syntax

```text
§   5.20.5.5. Read intra Y mode syntax

     read_intra_y_mode( ) {                                                      Descriptor

       if ( Lossless ) {

           use_dpcm_y                                                                S()

       } else {

           use_dpcm_y = 0

       }

       if ( use_dpcm_y ) {

           dpcm_mode_y                                                               S()

           AngleDeltaY = 0

           mrl_index = 0

           if ( dpcm_mode_y ) {

               YMode = H_PRED

               IntraJointMode = 50

           } else {

               YMode = V_PRED

               IntraJointMode = 22

           }

           if ( allow_fsc_intra() ) {

               fsc_mode                                                              S()

           }

           return

       }

       y_mode_set                                                                    S()

       if ( y_mode_set == 0 ) {

           y_mode_index                                                              S()

           modeIdx = y_mode_index

           if ( y_mode_index == MODE_INDEX_COUNT - 1 ) {

               y_mode_offset                                                         S()

               modeIdx += y_mode_offset

           }

       } else {

           y_second_mode                                                            L(4)

           modeIdx = FIRST_MODE_COUNT + (y_mode_set - 1) * SECOND_MODE_COUNT +

                 y_second_mode

       }

       modeDelta = get_intra_y_mode_set(modeIdx)

       IntraJointMode = modeDelta

       if ( modeDelta < NON_DIRECTIONAL_MODES_COUNT ) {

           YMode = Reordered_Y_Mode[ modeDelta ]

           AngleDeltaY = 0




    AV2 Specification                                                            Page 174 of 1169
     } else {

         modeDelta -= NON_DIRECTIONAL_MODES_COUNT

         YMode = Reordered_Y_Mode[ modeDelta / TOTAL_ANGLE_DELTA_COUNT +

              NON_DIRECTIONAL_MODES_COUNT ]

         AngleDeltaY = (modeDelta % TOTAL_ANGLE_DELTA_COUNT) - MAX_ANGLE_DELTA

     }

     if ( TreeType != CHROMA_PART && allow_fsc_intra() ) {

         fsc_mode                                                                            S()

     }

     if (enable_mrls && is_directional_mode(YMode)) {

         mrl_index                                                                           S()

         if ( mrl_index > 0 ) {

             mrl_sec_index                                                                   S()

         }

     } else {

         mrl_index = 0

     }

 }


where Reordered_Y_Mode, Default_Mode_List_Y, get_intra_y_mode_set, get_joint_mode, and
allow_fsc_intra are defined as:

 Reordered_Y_Mode[ INTRA_MODES ] = {
     DC_PRED,   SMOOTH_PRED, SMOOTH_V_PRED, SMOOTH_H_PRED, PAETH_PRED,
     D45_PRED, D67_PRED,     V_PRED,        D113_PRED,     D135_PRED,
     D157_PRED, H_PRED,      D203_PRED
 }

 Default_Mode_List_Y[ DIRECTIONAL_MODES_COUNT ] = {
     17, 45, 3, 10, 24, 31, 38, 52,
     15, 19, 43, 47, 1, 5, 8, 12, 22, 26, 29, 33, 36, 40, 50, 54,
     16, 18, 44, 46, 2, 4, 9, 11, 23, 25, 30, 32, 37, 39, 51, 53,
     14, 20, 42, 48, 0, 6, 7, 13, 21, 27, 28, 34, 35, 41, 49, 55
 }


 get_joint_mode( dir ) {
     if ( dir ) {
         mvRow = MiRow - 1
         mvCol = MiCol + Num_4x4_Blocks_Wide[ MiSize ] - 1
     } else {
         mvCol = MiCol - 1
         mvRow = MiRow + Num_4x4_Blocks_High[ MiSize ] - 1
     }
     if ( is_inside( mvRow, mvCol ) ) {
         return IntraJointModes[ mvRow ][ mvCol ]
     }
     return DC_PRED
 }

 get_intra_y_mode_set( modeIdx ) {
     if ( modeIdx < NON_DIRECTIONAL_MODES_COUNT ) {
         return modeIdx
     }
     modeIdx -= NON_DIRECTIONAL_MODES_COUNT
     for ( i = 0; i < DIRECTIONAL_MODES_COUNT; i++ ) {



AV2 Specification                                                                        Page 175 of 1169
               isDirSelected[ i ] = 0
           }
           if ( MiSize >= BLOCK_8X8 ) {
               count = 0
               for ( dir = 0; dir < 2; dir++ ) {
                   mode = get_joint_mode( dir )
                   if ( mode >= NON_DIRECTIONAL_MODES_COUNT ) {
                       mode -= NON_DIRECTIONAL_MODES_COUNT
                       if ( count == 0 || mode != dirModes[ 0 ] ) {
                           if ( modeIdx == 0 ) {
                               return mode + NON_DIRECTIONAL_MODES_COUNT
                           }
                           modeIdx -= 1
                           isDirSelected[ mode ] = 1
                           dirModes[ count ] = mode
                           count += 1
                       }
                   }
               }
               if ( Block_Width[ MiSize ] * Block_Height[ MiSize ] > 64 ) {
                   for ( i = 1; i <= 4; i++ ) {
                       for ( j = 0; j < count; j++ ) {
                           for ( sgn = -1 ; sgn <= 1 ; sgn += 2 ) {
                               mode = dirModes[ j ] + i * sgn
                               if (mode < 0) {
                                   mode += DIRECTIONAL_MODES_COUNT
                               }
                               else if (mode >= DIRECTIONAL_MODES_COUNT)
                                   mode -= DIRECTIONAL_MODES_COUNT
                               if ( !isDirSelected[ mode ] ) {
                                   if ( modeIdx == 0 ) {
                                        return mode + NON_DIRECTIONAL_MODES_COUNT
                                   }
                                   modeIdx -= 1
                                   isDirSelected[ mode ] = 1
                               }
                           }
                       }
                   }
               }
           }

           for ( i = 0; i < DIRECTIONAL_MODES_COUNT; i++ ) {
               mode = Default_Mode_List_Y[ i ]
               if ( !isDirSelected[ mode ] ) {
                   if ( modeIdx == 0 ) {
                       return mode + NON_DIRECTIONAL_MODES_COUNT
                   }
                   modeIdx -= 1
               }
           }
     }

     allow_fsc_intra( ) {
         w = Block_Width[ MiSize ]
         h = Block_Height[ MiSize ]
         return enable_idtx_intra && w <= FSC_MAX && h <= FSC_MAX
     }


```

<a id="s-5-20-5-6"></a>

##### § 5.20.5.6 Read intra UV mode syntax

```text
§   5.20.5.6. Read intra UV mode syntax

     read_intra_uv_mode( ) {                                                        Descriptor

         if ( Lossless ) {

          use_dpcm_uv                                                                   S()

         } else {

          use_dpcm_uv = 0



    AV2 Specification                                                               Page 176 of 1169
   }

   if ( use_dpcm_uv ) {

       dpcm_mode_uv                                                                  S()

       if ( dpcm_mode_uv ) {

           UVMode = H_PRED

       } else {

           UVMode = V_PRED

       }

       if ( UVMode == YMode ) {

           AngleDeltaUV = AngleDeltaY

       } else {

           AngleDeltaUV = 0

       }

       return

   }

   planeSz = get_plane_residual_size( ChromaMiSize, 1 )

   if ( !enable_cfl_intra ) {

       cflAllowed = 0

   } else if ( TreeType == CHROMA_PART && FrameIsIntra && !CflAllowedInSdp ) {

       cflAllowed = 0

   } else if ( Lossless ) {

       cflAllowed = planeSz == BLOCK_4X4

   } else {

       cflAllowed = Block_Width[ planeSz ] <= 64 &&

              Block_Height[ planeSz ] <= 64

   }

   if ( cflAllowed || is_mhccp_allowed() ) {

       is_cfl                                                                        S()

       if ( is_cfl ) {

           AngleDeltaUV = 0

           UVMode = UV_CFL_PRED

           return

       }

   }

   uv_mode                                                                           S()

   if ( uv_mode == CHROMA_MODE_COUNT - 1 ) {

       uv_mode_idx                                                                  L(3)

       uv_mode += uv_mode_idx

   }

   UVMode = get_intra_uv_mode_set( uv_mode )

   if ( UVMode == YMode ) {




AV2 Specification                                                                Page 177 of 1169
             AngleDeltaUV = AngleDeltaY

         } else {

             AngleDeltaUV = 0

         }

     }


    where Default_Mode_List_Uv, get_intra_uv_mode_set, and is_mhccp_allowed are defined as:

     Default_Mode_List_Uv[ UV_INTRA_MODES_CFL_NOT_ALLOWED ] = {
         DC_PRED, SMOOTH_PRED, SMOOTH_V_PRED, SMOOTH_H_PRED, PAETH_PRED,
         V_PRED,   H_PRED,    D45_PRED, D135_PRED,
         D67_PRED, D113_PRED, D157_PRED, D203_PRED
     }


     get_intra_uv_mode_set( modeIdx ) {
         if ( is_directional_mode( YMode ) ) {
             if ( modeIdx == 0 ) {
                 return YMode
             }
             modeIdx -= 1
         }
         for ( i = 0; i < UV_INTRA_MODES_CFL_NOT_ALLOWED; i++ ) {
             mode = Default_Mode_List_Uv[ i ]
             if ( mode != YMode || !is_directional_mode( YMode ) ) {
                 if ( modeIdx == 0 ) {
                     return mode
                 }
                 modeIdx -= 1
             }
         }
     }


     is_mhccp_allowed( ) {
         planeSz = get_plane_residual_size( ChromaMiSize, 1 )
         if ( !enable_mhccp ) {
             return 0
         } else if ( TreeType == CHROMA_PART && FrameIsIntra && !CflAllowedInSdp ) {
             return 0
         } else if ( Lossless ) {
             return planeSz == BLOCK_4X4
         } else {
             w = Block_Width[ planeSz ]
             h = Block_Height[ planeSz ]
             return ( w > 4 || h > 4 ) && w <= 32 && h <= 32
         }
     }


```

<a id="s-5-20-5-7"></a>

##### § 5.20.5.7 Intra segment ID syntax

```text
§   5.20.5.7. Intra segment ID syntax

     intra_segment_id( ) {                                                                    Descriptor

         if ( TreeType == CHROMA_PART ) {

             segment_id = SegmentIds[ MiRow ][ MiCol ]

         } else if ( segmentation_enabled ) {

             read_segment_id( )

         } else {

             segment_id = 0




    AV2 Specification                                                                         Page 178 of 1169
         }

         Lossless = LosslessArray[ segment_id ]

     }


```

<a id="s-5-20-5-8"></a>

##### § 5.20.5.8 Read segment ID syntax

```text
§   5.20.5.8. Read segment ID syntax

     read_segment_id( ) {                                       Descriptor

         if ( AvailU && AvailL ) {

             prevUL = SegmentIds[ MiRow - 1 ][ MiCol - 1 ]

         } else {

             prevUL = -1

         }

         if ( AvailU ) {

             prevU = SegmentIds[ MiRow - 1 ][ MiCol ]

         } else {

             prevU = -1

         }

         if ( AvailL ) {

             prevL = SegmentIds[ MiRow ][ MiCol - 1 ]

         } else {

             prevL = -1

         }

         if ( prevU == -1 ) {

             pred = (prevL == -1) ? 0 : prevL

         } else if ( prevL == -1 ) {

             pred = prevU

         } else {

             pred = (prevUL == prevU) ? prevU : prevL

         }

         if ( skip_flag && !HasLosslessSegment ) {

             segment_id = pred

         } else {

             if ( enable_ext_seg ) {

                 seg_id_ext_flag                                    S()

             } else {

                 seg_id_ext_flag = 0

             }

             segment_id                                             S()

             if ( seg_id_ext_flag ) {

                 segment_id += 8

             }

             segment_id = neg_deinterleave( segment_id, pred,




    AV2 Specification                                           Page 179 of 1169
                         LastActiveSegId + 1 )

         }

     }


    where neg_deinterleave is a function defined as:

     neg_deinterleave(diff, ref, max) {
         if ( !ref ) {
             return diff
         }
         if ( ref >= (max - 1) ) {
             return max - diff - 1
         }
         if ( 2 * ref < max ) {
             if ( diff <= 2 * ref ) {
                  if ( diff & 1 ) {
                      return ref + ((diff + 1) >> 1)
                  } else {
                      return ref - (diff >> 1)
                  }
             }
             return diff
         } else {
             if ( diff <= 2 * (max - ref - 1) ) {
                  if ( diff & 1 ) {
                      return ref + ((diff + 1) >> 1)
                  } else {
                      return ref - (diff >> 1)
                  }
             }
             return max - (diff + 1)
         }
     }


```

<a id="s-5-20-5-9"></a>

##### § 5.20.5.9 Skip mode syntax

```text
§   5.20.5.9. Skip mode syntax

     read_skip_mode() {                                                              Descriptor

         if ( seg_feature_active( SEG_LVL_SKIP ) ||

             seg_feature_active( SEG_LVL_GLOBALMV ) ||

             !skip_mode_present ||

             !is_comp_ref_allowed( ) ||

             RegionType == INTRA_REGION ) {

             skip_mode = 0

         } else {

             skip_mode                                                                   S()

         }

     }


    where is_comp_ref_allowed is a function that checks the block size as follows:


     is_comp_ref_allowed( ) {
         w = Block_Width[ MiSize ]
         h = Block_Height[ MiSize ]
         return ( Min( w, h ) >= 8 ) || is_thin_4xn_nx4_block()
     }




    AV2 Specification                                                                Page 180 of 1169
```

<a id="s-5-20-5-10"></a>

##### § 5.20.5.10 Skip syntax

```text
§   5.20.5.10. Skip syntax

     read_skip() {                                                                      Descriptor

         if ( SegIdPreSkip && seg_feature_active( SEG_LVL_SKIP ) ) {

             skip_flag = 1

         } else {

             skip_flag                                                                      S()

         }

     }


```

<a id="s-5-20-5-11"></a>

##### § 5.20.5.11 Quantizer index delta syntax

```text
§   5.20.5.11. Quantizer index delta syntax

     read_delta_qindex( ) {                                                             Descriptor

         if ( !(MiSize == SbSize && skip_flag) && ReadDeltas ) {

             delta_q_abs                                                                    S()

             if ( delta_q_abs == DELTA_Q_SMALL ) {

                 delta_q_rem_bits                                                          L(3)

                 delta_q_rem_bits++

                 delta_q_abs_bits                                                     L(delta_q_rem_b
                                                                                            its)

                 delta_q_abs = delta_q_abs_bits + (1 << delta_q_rem_bits) +

                      DELTA_Q_SMALL - 2

             }

             if ( delta_q_abs ) {

                 delta_q_sign_bit                                                          L(1)

                 reducedDeltaQIndex = delta_q_sign_bit ? -delta_q_abs : delta_q_abs

                 CurrentQIndex = Clip3(1, MaxQ,

                  CurrentQIndex + (reducedDeltaQIndex << delta_q_res))

             }

         }

         if ( delta_q_present ) {

             CurrentQIndex = Clip3(1, MaxQ, CurrentQIndex)

         }

     }


```

<a id="s-5-20-5-12"></a>

##### § 5.20.5.12 Segmentation feature active function

```text
§   5.20.5.12. Segmentation feature active function

     seg_feature_active_idx( idx, feature ) {
         return segmentation_enabled && FeatureEnabled[ idx ][ feature ]
     }

     seg_feature_active( feature ) {
         return seg_feature_active_idx( segment_id, feature )
     }




    AV2 Specification                                                                  Page 181 of 1169
```

<a id="s-5-20-6"></a>

#### § 5.20.6 Transform and quantization structures

```text
§   5.20.6. Transform and quantization structures

```

<a id="s-5-20-6-1"></a>

##### § 5.20.6.1 TX size syntax

```text
§   5.20.6.1. TX size syntax

     read_tx_size( allowSelect ) {                                                Descriptor

         if ( Lossless ) {

             if ( MiSize == BLOCK_4X4 ||

                 ( !is_inter && !fsc_mode ) ||

                 !allowSelect ) {

                 TxSize = TX_4X4

             } else {

                 lossless_tx_size                                                     S()

                 if ( lossless_tx_size ) {

                     TxSize = find_tx_size( Min(32, Block_Width[ MiSize ] ),

                                Min(32, Block_Height[ MiSize ] ) )

                 } else {

                     TxSize = TX_4X4

                 }

             }

             return 0

         }

         maxRectTxSize = Max_Tx_Size_Rect[ MiSize ]

         TxSize = maxRectTxSize

         if ( MiSize > BLOCK_4X4 && allowSelect && TxMode == TX_MODE_SELECT ) {

             widthChunks = Block_Width[ MiSize ] >> 6

             heightChunks = Block_Height[ MiSize ] >> 6

             if ( widthChunks > 1 || heightChunks > 1 ) {

                 for ( chunkY = 0; chunkY < heightChunks; chunkY++ ) {

                     for ( chunkX = 0; chunkX < widthChunks; chunkX++ ) {

                         miRowChunk = MiRow + ( chunkY << 4 )

                         miColChunk = MiCol + ( chunkX << 4 )

                         set_tx_size( miRowChunk, miColChunk, 16, 16, 0, 0 )

                     }

                 }

             } else {

                 read_tx_partition( MiRow, MiCol, maxRectTxSize )

             }

             return 1

         }

         return 0

     }




    AV2 Specification                                                             Page 182 of 1169
         NOTE:             The same transform partition is used for all chunks when read_tx_size is called.

```

<a id="s-5-20-6-2"></a>

##### § 5.20.6.2 Block TX size syntax

```text
§   5.20.6.2. Block TX size syntax

     read_block_tx_size( ) {                                                                                  Descriptor

         bw4 = Num_4x4_Blocks_Wide[ MiSize ]

         bh4 = Num_4x4_Blocks_High[ MiSize ]

         if ( TxMode == TX_MODE_SELECT &&

             MiSize > BLOCK_4X4 && is_inter &&

             !skip_flag && !Lossless ) {

             maxTxSz = Max_Tx_Size_Rect[ MiSize ]

             txW4 = Tx_Width[ maxTxSz ] / MI_SIZE

             txH4 = Tx_Height[ maxTxSz ] / MI_SIZE

             for ( row = MiRow; row < MiRow + bh4; row += txH4 ) {

                 for ( col = MiCol; col < MiCol + bw4; col += txW4 ) {

                     read_tx_partition( row, col, maxTxSz)

                 }

             }

         } else {

             if ( read_tx_size( !skip_flag || !is_inter ) == 0 ) {

                 for ( row = MiRow; row < MiRow + bh4; row++ ) {

                     for ( col = MiCol; col < MiCol + bw4; col++ ) {

                         LumaTxSizes[ row ][ col ] = TxSize

                         LumaTxMiddle[ row ][ col ] = 0

                         LumaTxScanOrder[ row ][ col ] = 0

                     }

                 }

             }

         }

     }


```

<a id="s-5-20-6-3"></a>

##### § 5.20.6.3 Read TX partition syntax

```text
§   5.20.6.3. Read TX partition syntax

     read_tx_partition( row, col, txSz) {                                                                     Descriptor

         if ( row >= MiRows || col >= MiCols ) {

             return

         }

         horzTxSz = find_tx_size(Tx_Width[ txSz ], Tx_Height[ txSz ] >> 1)

         vertTxSz = find_tx_size(Tx_Width[ txSz ] >> 1, Tx_Height[ txSz ])

         allowHorz = horzTxSz != TX_INVALID

         allowVert = vertTxSz != TX_INVALID

         txPartition = TX_PARTITION_NONE




    AV2 Specification                                                                                         Page 183 of 1169
   if ( Block_Width[ MiSize ] <= 64 && Block_Height[ MiSize ] <= 64 ) {

       tx_do_partition                                                               S()

       if ( tx_do_partition ) {

           if ( allowHorz && allowVert ) {

               tx_partition_type                                                     S()

               txPartition = tx_partition_type + 1

           } else if ( Size_To_Tx_Type_Group_Vert_Or_Horz[ MiSize ] > 0 ) {

               if ( reduced_tx_part_set ) {

                   tx_2or3_partition_type = 0

               } else {

                   tx_2or3_partition_type                                            S()

               }

               if ( allowHorz ) {

                   txPartition = tx_2or3_partition_type ? TX_PARTITION_HORZ4 :

                                   TX_PARTITION_HORZ

               } else {

                   txPartition = tx_2or3_partition_type ? TX_PARTITION_VERT4 :

                                   TX_PARTITION_VERT

               }

           } else {

               txPartition = allowHorz ? TX_PARTITION_HORZ : TX_PARTITION_VERT

           }

       }

   }

   w4 = Tx_Width[ txSz ] / MI_SIZE

   h4 = Tx_Height[ txSz ] / MI_SIZE

   if ( txPartition == TX_PARTITION_NONE ) {

       TxSize = set_tx_size(row, col, h4 , w4, 0, 0)

   } else if ( txPartition == TX_PARTITION_HORZ ) {

       h4 = h4 >> 1

       set_tx_size(row, col, h4, w4, 0, 0)

       row += h4

       TxSize = set_tx_size(row, col, h4 , w4, 0, 0)

   } else if ( txPartition == TX_PARTITION_VERT ) {

       w4 = w4 >> 1

       set_tx_size(row, col, h4, w4, 0, 0)

       col += w4

       TxSize = set_tx_size(row, col, h4 , w4, 0, 0)

   } else if ( txPartition == TX_PARTITION_HORZ4 ) {

       h4 = h4 >> 2

       set_tx_size(row, col, h4, w4, 0, 0)




AV2 Specification                                                                Page 184 of 1169
     row += h4

     set_tx_size(row, col, h4, w4, 0, 0)

     row += h4

     set_tx_size(row, col, h4, w4, 0, 0)

     row += h4

     TxSize = set_tx_size(row, col, h4, w4, 0, 0)

   } else if ( txPartition == TX_PARTITION_VERT4 ) {

     w4 = w4 >> 2

     set_tx_size(row, col, h4, w4, 0, 0)

     col += w4

     set_tx_size(row, col, h4, w4, 0, 0)

     col += w4

     set_tx_size(row, col, h4, w4, 0, 0)

     col += w4

     TxSize = set_tx_size(row, col, h4, w4, 0, 0)

   } else if ( txPartition == TX_PARTITION_HORZ5 ) {

     h4 = h4 >> 2

     w4 = w4 >> 1

     set_tx_size(row, col, h4, w4, 0, 0)

     col += w4

     set_tx_size(row, col, h4, w4, 1, 0)

     col -= w4

     row += h4

     h4 = h4 << 1

     w4 = w4 << 1

     set_tx_size(row, col, h4, w4, 1, 0)

     row += h4

     h4 = h4 >> 1

     w4 = w4 >> 1

     set_tx_size(row, col, h4, w4, 1, 0)

     col += w4

     TxSize = set_tx_size(row, col, h4, w4, 1, 0)

   } else if ( txPartition == TX_PARTITION_VERT5 ) {

     h4 = h4 >> 1

     w4 = w4 >> 2

     set_tx_size(row, col, h4, w4, 0, 1)

     row += h4

     set_tx_size(row, col, h4, w4, 1, 1)

     col += w4

     row -= h4

     h4 = h4 << 1




AV2 Specification                                      Page 185 of 1169
             w4 = w4 << 1

             set_tx_size(row, col, h4, w4, 1, 1)

             col += w4

             h4 = h4 >> 1

             w4 = w4 >> 1

             set_tx_size(row, col, h4, w4, 1, 1)

             row += h4

             TxSize = set_tx_size(row, col, h4, w4, 1, 1)

         } else { // TX_PARTITION_SPLIT

             w4 = w4 >> 1

             h4 = h4 >> 1

             set_tx_size(row, col + w4, h4, w4, 0, 0)

             set_tx_size(row, col, h4, w4, 0, 0)

             set_tx_size(row + h4, col, h4, w4, 0, 0)

             TxSize = set_tx_size(row + h4, col + w4, h4, w4, 0, 0)

         }

     }


    where the function find_tx_size finds the transform block size for the given dimensions and is defined as:

     find_tx_size( w, h ) {
         for ( txSz = 0; txSz < TX_SIZES_ALL; txSz++ ) {
             if ( Tx_Width[ txSz ] == w && Tx_Height[ txSz ] == h ) {
                 return txSz
             }
         }
         return TX_INVALID
     }


    and the function set_tx_size saves the transform size as follows:

     set_tx_size(row, col, h4, w4, mid, scanOrder) {
         subTxSz = find_tx_size( w4 << 2, h4 << 2 )
         for ( i = 0; i < h4; i++ ) {
             for ( j = 0; j < w4; j++ ) {
                 LumaTxSizes[ row + i ][ col + j ] = subTxSz
                 LumaTxMiddle[ row + i ][ col + j ] = mid
                 LumaTxScanOrder[ row + i ][ col + j ] = scanOrder
             }
         }
         return subTxSz
     }


```

<a id="s-5-20-7"></a>

#### § 5.20.7 Motion vector and prediction structures

```text
§   5.20.7. Motion vector and prediction structures

```

<a id="s-5-20-7-1"></a>

##### § 5.20.7.1 Inter frame mode info syntax

```text
§   5.20.7.1. Inter frame mode info syntax

     inter_frame_mode_info( ) {                                                                   Descriptor

         use_intrabc = 0

         skip_flag = 0




    AV2 Specification                                                                            Page 186 of 1169
         inter_segment_id( 1 )

         read_skip_mode( )

         read_is_inter( )

         if ( is_inter ) {

             read_skip( )

         } else {

             skip_flag = 0

         }

         if ( !SegIdPreSkip ) {

             inter_segment_id( 0 )

         }

         Lossless = LosslessArray[ segment_id ]

         if ( TreeType != CHROMA_PART ) {

             read_gdf( )

             read_cdef( )

             read_ccso( )

             read_delta_qindex( )

         }

         ReadDeltas = 0

         if ( use_intrabc ) {

             read_intrabc_info( )

         } else if ( is_inter ) {

             inter_block_mode_info( )

         } else {

             intra_block_mode_info( )

         }

     }


```

<a id="s-5-20-7-2"></a>

##### § 5.20.7.2 Inter segment ID syntax

```text
§   5.20.7.2. Inter segment ID syntax

    This is called before (preSkip equal to 1) and after (preSkip equal to 0) the skip_flag syntax element has
    been read.

     inter_segment_id( preSkip ) {                                                                Descriptor

         if ( TreeType == CHROMA_PART ) {

             segment_id = SegmentIds[ MiRow ][ MiCol ]

         } else if ( segmentation_enabled ) {

             predictedSegmentId = get_segment_id( )

             if ( segmentation_update_map ) {

              if ( preSkip && !SegIdPreSkip ) {

                  segment_id = 0

                  return

              }




    AV2 Specification                                                                             Page 187 of 1169
                 if ( !preSkip ) {

                     if ( skip_flag ) {

                         seg_id_predicted = 0

                         for ( i = 0; i < Num_4x4_Blocks_Wide[ MiSize ]; i++ ) {

                             AboveSegPredContext[ MiCol + i ] = seg_id_predicted

                         }

                         for ( i = 0; i < Num_4x4_Blocks_High[ MiSize ]; i++ ) {

                             LeftSegPredContext[ MiRow + i ] = seg_id_predicted

                         }

                         read_segment_id( )

                         return

                     }

                 }

                 if ( segmentation_temporal_update == 1 ) {

                     seg_id_predicted                                                  S()

                     if ( seg_id_predicted ) {

                         segment_id = predictedSegmentId

                     } else {

                         read_segment_id( )

                     }

                     for ( i = 0; i < Num_4x4_Blocks_Wide[ MiSize ]; i++ ) {

                         AboveSegPredContext[ MiCol + i ] = seg_id_predicted

                     }

                     for ( i = 0; i < Num_4x4_Blocks_High[ MiSize ]; i++ ) {

                         LeftSegPredContext[ MiRow + i ] = seg_id_predicted

                     }

                 } else {

                     read_segment_id( )

                 }

             } else {

                 segment_id = predictedSegmentId

             }

         } else {

             segment_id = 0

         }

     }


```

<a id="s-5-20-7-3"></a>

##### § 5.20.7.3 Is inter syntax

```text
§   5.20.7.3. Is inter syntax

     read_is_inter( ) {                                                            Descriptor

         if ( RegionType == INTRA_REGION ) {

             is_inter = 0




    AV2 Specification                                                              Page 188 of 1169
         } else if ( skip_mode ) {

             is_inter = 1

         } else if ( seg_feature_active ( SEG_LVL_GLOBALMV ) ) {

             is_inter = 1

         } else if ( TreeType == SHARED_PART && MiSize != ChromaMiSize ) {

             is_inter = 1

         } else {

             is_inter                                                                             S()

         }

         if ( !is_inter && allow_intrabc &&

             Block_Width[ MiSize ] <= 64 &&

             Block_Height[ MiSize ] <= 64 &&

             MiSize != BLOCK_64X64 &&

             RegionType == MIXED_REGION ) {

             use_intrabc                                                                          S()

             if ( use_intrabc ) {

                 is_inter = 1

             }

         } else {

             use_intrabc = 0

         }

     }


```

<a id="s-5-20-7-4"></a>

##### § 5.20.7.4 Get segment ID function

```text
§   5.20.7.4. Get segment ID function

    The predicted segment id is the smallest value found in the on-screen region of the segmentation map
    covered by the current block.

     get_segment_id( ) {
         bw4 = Num_4x4_Blocks_Wide[ MiSize ]
         bh4 = Num_4x4_Blocks_High[ MiSize ]
         xMis = Min( MiCols - MiCol, bw4 )
         yMis = Min( MiRows - MiRow, bh4 )
         seg = MAX_SEGMENTS - 1
         for ( y = 0; y < yMis; y++ ) {
             for ( x = 0; x < xMis; x++ ) {
                 seg = Min( seg, PrevSegmentIds[ MiRow + y ][ MiCol + x ] )
             }
         }
         return seg
     }


```

<a id="s-5-20-7-5"></a>

##### § 5.20.7.5 Intra block mode info syntax

```text
§   5.20.7.5. Intra block mode info syntax

     intra_block_mode_info( ) {                                                               Descriptor

         RefFrame[ 0 ] = INTRA_FRAME

         RefFrame[ 1 ] = NONE

         motion_mode = SIMPLE




    AV2 Specification                                                                         Page 189 of 1169
         fsc_mode = 0

         use_most_probable_precision = 0

         MvPrecision = FrameMvPrecision

         CwpIdx = CWP_EQUAL

         PaletteSizeY = 0

         motion_mode = SIMPLE

         if ( TreeType != CHROMA_PART ) {

             read_intra_y_mode()

         } else {

             YMode = YModes[ MiRow ][ MiCol ]

             AngleDeltaY = AngleDeltaYs[ MiRow ][ MiCol ]

             PaletteSizeY = PaletteSizes[ MiRow ][ MiCol ]

         }

         if ( HasChroma ) {

             read_intra_uv_mode()

             if ( UVMode == UV_CFL_PRED ) {

                 read_cfl_alphas( )

             }

         }

         if ( MiSize >= BLOCK_8X8 &&

             Block_Width[ MiSize ] <= 64   &&

             Block_Height[ MiSize ] <= 64 &&

             allow_screen_content_tools ) {

             palette_mode_info( )

         }

         if ( TreeType != CHROMA_PART ) {

             dip_mode_info( )

         }

     }


```

<a id="s-5-20-7-6"></a>

##### § 5.20.7.6 Inter block mode info syntax

```text
§   5.20.7.6. Inter block mode info syntax

     inter_block_mode_info( ) {                              Descriptor

         mrl_index = 0

         use_dip = 0

         fsc_mode = 0

         use_dpcm_y = 0

         use_dpcm_uv = 0

         PaletteSizeY = 0

         use_most_probable_precision = 0

         MvPrecision = FrameMvPrecision

         IntraJointMode = DC_PRED




    AV2 Specification                                        Page 190 of 1169
   use_bawp = 0

   use_amvd = 0

   read_ref_frames( )

   isCompound = is_inter_ref_frame( RefFrame[ 1 ] )

   DeriveWrl = !skip_mode && !isCompound && RefFrame[ 0 ] != TIP_FRAME &&

             Block_Width[ MiSize ] >= 8 && Block_Height[ MiSize ] >= 8

   find_mode_ctx( isCompound )

   if ( skip_mode ) {

     YMode = NEAR_NEARMV

     use_optflow = 0

   } else if ( seg_feature_active( SEG_LVL_SKIP ) ||

     seg_feature_active( SEG_LVL_GLOBALMV ) ) {

     YMode = GLOBALMV

     use_optflow = 0

   } else if ( isCompound ) {

     if ( RefFrame[ 0 ] == RefFrame[ 1 ] ) {

         compound_mode_same_refs                                                S()

         if ( compound_mode_same_refs < 2 ) {

             YMode = NEAR_NEARMV + compound_mode_same_refs

         } else {

             YMode = NEAR_NEARMV + compound_mode_same_refs + 1

         }

     } else {

         is_joint                                                               S()

         if ( is_joint ) {

             YMode = JOINT_NEWMV

         } else {

             compound_mode_non_joint                                            S()

             YMode = NEAR_NEARMV + compound_mode_non_joint

         }

     }

     if ( opfl_refine_type == REFINE_SWITCHABLE &&

         opfl_allowed_for_refs( RefFrame ) &&

         Block_Width[ MiSize ] >= 8 && Block_Height[ MiSize ] >= 8 &&

         YMode != GLOBAL_GLOBALMV ) {

         use_optflow                                                            S()

     } else {

         use_optflow = 0

     }

     if ( allow_amvd_mode( YMode ) ) {

         use_amvd                                                               S()




AV2 Specification                                                           Page 191 of 1169
     }

   } else {

     use_optflow = 0

     if ( RefFrame[ 0 ] == TIP_FRAME ) {

         tip_pred_mode                                                            S()

         YMode = Tip_Pred_Index_To_Mode[ tip_pred_mode ]

         if ( allow_amvd_mode( YMode ) ) {

             use_amvd                                                             S()

         }

     } else {

         if ( allow_warpmv_mode &&

             Min(Block_Width[ MiSize ], Block_Height[ MiSize ]) >= 8 ) {

             is_warp                                                              S()

         } else {

             is_warp = 0

         }

         if ( is_warp ) {

             if ( force_integer_mv ) {

                 warp_mv = 1

             } else {

                 warp_mv                                                          S()

             }

             YMode = warp_mv ? WARPMV : WARP_NEWMV

         } else {

             single_mode                                                          S()

             YMode = NEARMV + single_mode

             if ( allow_amvd_mode( YMode ) ) {

                 use_amvd                                                         S()

             }

             if ( allow_bawp && !is_scaled( RefFrame[ 0 ], 1 ) &&

                 Min(Block_Width[ MiSize ], Block_Height[ MiSize ]) >= 8 &&

                 FrameType != SWITCH_FRAME && YMode != GLOBALMV ) {

                 use_bawp                                                         S()

                 if ( use_bawp ) {

                     explicit_bawp                                                S()

                     if ( explicit_bawp ) {

                         explicit_bawp_scale                                      S()

                     }

                 } else {

                     explicit_bawp = 0

                 }




AV2 Specification                                                             Page 192 of 1169
                   if ( use_bawp && HasChroma ) {

                       use_bawp_chroma                                   S()

                   }

               }

           }

       }

   }

   if ( skip_mode ) {

       find_mv_stack( isCompound )

   } else if ( has_second_drl( YMode ) ) {

       r0 = RefFrame[ 0 ]

       r1 = RefFrame[ 1 ]

       RefFrame[ 0 ] = r0

       RefFrame[ 1 ] = NONE

       find_mv_stack( 0 )

       for ( i = 0; i < MAX_REF_MV_STACK_SIZE; i++ ) {

           RefStack0Mvs[ i ] = RefStackMv[ i ][ 0 ]

       }

       RefFrame[ 0 ] = r1

       RefFrame[ 1 ] = NONE

       find_mv_stack( 0 )

       for ( i = 0; i < MAX_REF_MV_STACK_SIZE; i++ ) {

           RefStack1Mvs[ i ] = RefStackMv[ i ][ 0 ]

       }

       RefFrame[ 0 ] = r0

       RefFrame[ 1 ] = r1

   } else {

       find_mv_stack( isCompound )

   }

   motion_mode = read_motion_mode( isCompound )

   RefWarpIdx = 0

   if ( YMode == WARPMV || motion_mode == DELTAWARP ) {

       for ( idx = 0; idx < MAX_WARP_REF_CANDIDATES - 1; idx++ ) {

           warp_idx                                                      S()

           if ( warp_idx == 0 ) {

               RefWarpIdx = idx

               break

           }

           RefWarpIdx = idx + 1

       }

   }




AV2 Specification                                                    Page 193 of 1169
   if ( YMode == WARPMV && RefWarpIdx < 2 ) {

       warpmv_with_mvd                                                              S()

   } else {

       warpmv_with_mvd = 0

   }

   if ( is_joint_mvd_coding_mode(YMode) ) {

       jmvd_scale_mode                                                              S()

   }

   RefMvIdx = 0

   if ( has_newmv(YMode) || has_nearmv(YMode) ) {

       m = max_drl_bits_minus_1 + 1

       if ( has_second_drl( YMode ) ) {

           RefMvIdx0 = read_drl_idx( 0, m )

           start = ( RefFrame[ 0 ]==RefFrame[ 1 ] && YMode == NEAR_NEARMV ) ?

                      RefMvIdx0 + 1 : 0

           RefMvIdx1 = read_drl_idx( start, m )

       } else {

           RefMvIdx = read_drl_idx( 0, m )

       }

   }

   IsAdaptiveMvd = enable_adaptive_mvd && use_amvd

   if ( IsAdaptiveMvd ) {

       MvPrecision = FrameMvPrecision

       use_most_probable_precision = 1

   } else if ( enable_flex_mvres && UsePerBlockMvPrecision &&

               has_newmv( YMode ) ) {

       use_most_probable_precision                                                  S()

       if ( use_most_probable_precision ) {

           MvPrecision = FrameMvPrecision

       } else {

           pb_mv_precision                                                          S()

           adjustedPrecision = Max( MV_PRECISION_ONE_PEL,

                        FrameMvPrecision - 2) -

                      pb_mv_precision

           if ( adjustedPrecision <= MV_PRECISION_TWO_PEL ) {

               MvPrecision = adjustedPrecision - 1

           } else {

               MvPrecision = adjustedPrecision

           }

       }

   } else {




AV2 Specification                                                               Page 194 of 1169
       MvPrecision = FrameMvPrecision

       use_most_probable_precision = 1

   }

   assign_mv( isCompound )

   if ( motion_mode == DELTAWARP ) {

       read_warp_delta( )

   }

   if ( YMode == WARPMV ) {

       read_interintra_mode( 1 )

   }

   read_refinemv( isCompound )

   read_compound_type( isCompound )

   CwpIdx = CWP_EQUAL

   if ( enable_cwp ) {

       if ( isCompound && skip_mode ) {

           CwpIdx = RefStackCwp[ RefMvIdx ]

       } else if ( isCompound && !use_refinemv &&

                   compound_type == COMPOUND_AVERAGE &&

                   motion_mode == SIMPLE && !use_optflow ) {

           if ( YMode == NEAR_NEARMV || (is_joint_mvd_coding_mode(YMode) &&

                               jmvd_scale_mode==0) ) {

               for ( idx = 0; idx < MAX_CWP_NUM - 1; idx++ ) {

                   cwp_idx                                                          S()

                   if ( cwp_idx == 0 ) {

                       break

                   }

               }

               CwpIdx = Cwp_Weighting_Factor[ is_same_side() ][ idx ]

           }

       }

   }

   if ( isCompound && opfl_refine_type == REFINE_ALL &&

       compound_type == COMPOUND_AVERAGE &&

       YMode != GLOBAL_GLOBALMV &&

       !skip_mode &&

       CwpIdx == CWP_EQUAL &&

       opfl_allowed_for_refs( RefFrame ) &&

       Block_Width[ MiSize ] >= 8 && Block_Height[ MiSize ] >= 8) {

       use_optflow = 1

   }

   if ( skip_mode || use_optflow || use_refinemv || DecidedAgainstRefinemv ||




AV2 Specification                                                               Page 195 of 1169
         RefFrame[ 0 ] == TIP_FRAME ) {

         interp_filter = EIGHTTAP_SHARP

     } else if ( interpolation_filter == SWITCHABLE ) {

         if ( needs_interp_filter( ) ) {

             interp_filter                                                   S()

         } else {

             interp_filter = EIGHTTAP

         }

     } else {

         interp_filter = interpolation_filter

     }

 }


The function has_nearmv is defined as:

 has_nearmv( mode ) {
     return (mode == NEARMV || mode == NEAR_NEARMV
             || mode == NEAR_NEWMV || mode == NEW_NEARMV)
 }


The function has_newmv is defined as:

 has_newmv( mode ) {
     return (mode == NEWMV ||
             mode == NEW_NEWMV ||
             mode == NEAR_NEWMV ||
             mode == NEW_NEARMV ||
             mode == WARP_NEWMV ||
             mode == JOINT_NEWMV
             )
 }


The function needs_interp_filter is defined as:

 needs_interp_filter( ) {
     large = (Min(Block_Width[ MiSize ], Block_Height[ MiSize ]) >= 8)
     if ( motion_mode >= LOCALWARP ) {
         return 0
     } else if ( large && YMode == GLOBALMV ) {
         return 0
     } else if ( large && YMode == GLOBAL_GLOBALMV ) {
         return 0
     } else {
         return 1
     }
 }


The function is_inter_ref_frame is defined as:

 is_inter_ref_frame(ref) {
     return ref != INTRA_FRAME && ref != NONE
 }




AV2 Specification                                                        Page 196 of 1169
    The function is_joint_mvd_coding_mode is defined as:

     is_joint_mvd_coding_mode(mode) {
         return mode == JOINT_NEWMV
     }


    The function has_second_drl is defined as:

     has_second_drl(mode) {
         return (mode == NEAR_NEARMV || mode == NEAR_NEWMV) && !skip_mode &&
                !use_optflow
     }



      NOTE:       Two reference lists can be used for NEAR_NEWMV, but only one for NEW_NEARMV.


    The constant table Cwp_Weighting_Factor is defined as:

     Cwp_Weighting_Factor[ 2 ][ MAX_CWP_NUM ] = {
         { 8, 12, 4, 10, 6 },
         { 8, 12, 4, 20, -4 }
     }


    The function opfl_allowed_for_refs is defined as:

     opfl_allowed_for_refs( refFrames ) {
         if ( FrameType == SWITCH_FRAME ||
              is_scaled( refFrames[ 0 ], 1 ) ||
              is_scaled( refFrames[ 1 ], 1 ) ) {
             return 0
         }
         d0 = get_relative_dist( OrderHint, OrderHints[ refFrames[ 0 ] ] )
         d1 = get_relative_dist( OrderHint, OrderHints[ refFrames[ 1 ] ] )
         return (d0 <= 0) ^ (d1 <= 0)
     }


    The constant table Tip_Pred_Index_To_Mode is defined as:

     Tip_Pred_Index_To_Mode[ 2 ] = {
         NEARMV,
         NEWMV
     }


    The function allow_amvd_mode is defined as:

     allow_amvd_mode( mode ) {
         return enable_adaptive_mvd &&
             (mode == NEWMV ||
              mode == NEW_NEWMV ||
              mode == NEAR_NEWMV ||
              mode == NEW_NEARMV ||
              mode == JOINT_NEWMV)
     }


```

<a id="s-5-20-7-7"></a>

##### § 5.20.7.7 Read warp delta syntax

```text
§   5.20.7.7. Read warp delta syntax

     read_warp_delta( ) {                                                                  Descriptor



    AV2 Specification                                                                     Page 197 of 1169
         for ( i = 0; i < 6; i++ ) {

             params[ i ] = WarpParamStack[ RefWarpIdx ][ i ]

         }

         useSixParam = enable_six_param_warp_delta && RefWarpIdx == 1

         if ( YMode == WARP_NEWMV && (useSixParam || RefWarpIdx == 0) ) {

             warp_delta_precision                                                    S()

             params[ 0 ] = 0

             params[ 1 ] = 0

             params[ 2 ] += read_warp_delta_param( 2, warp_delta_precision )

             params[ 3 ] += read_warp_delta_param( 3, warp_delta_precision )

             if ( useSixParam ) {

                 params[ 4 ] += read_warp_delta_param(4, warp_delta_precision)

                 params[ 5 ] += read_warp_delta_param(5, warp_delta_precision)

             } else {

                 params[ 4 ] = -params[ 3 ]

                 params[ 5 ] = params[ 2 ]

             }

         }

         LocalWarpParams[ 0 ] = reduce_warp_model( params )

         (LocalWarpParams[ 0 ][ 0 ], LocalWarpParams[ 0 ][ 1 ]) =

             get_warp_translation( LocalWarpParams[ 0 ], 0 )

     }


    where the function read_warp_delta_param is specified as:

     read_warp_delta_param( idx, highPrec ) {
         S() warp_delta_param_low;
         v = warp_delta_param_low
         if ( highPrec && v == WARP_DELTA_NUM_SYMBOLS_LOW - 1 ) {
             S() warp_delta_param_high;
             v += warp_delta_param_high
         }
         if ( v != 0 ) {
             S() warp_delta_param_sign;
             if ( warp_delta_param_sign ) {
                 v = -v
             }
         }
         return v << ( WARP_DELTA_STEP_BITS + 1 - highPrec )
     }


```

<a id="s-5-20-7-8"></a>

##### § 5.20.7.8 Read drl idx syntax

```text
§   5.20.7.8. Read drl idx syntax

     read_drl_idx(start,m) {                                                     Descriptor

         for ( idx = start; idx < m; idx++ ) {

             drl_mode                                                                S()

             if ( drl_mode == 0 ) {

                 return idx




    AV2 Specification                                                            Page 198 of 1169
             }

         }

         return m

     }


```

<a id="s-5-20-7-9"></a>

##### § 5.20.7.9 DIP mode info syntax

```text
§   5.20.7.9. DIP mode info syntax

     dip_mode_info( ) {                                                       Descriptor

         use_dip = 0

         if ( enable_dip &&

                 YMode == DC_PRED && PaletteSizeY == 0 &&

                 Block_Width[ MiSize ] > 4 && Block_Height[ MiSize ] > 4 &&

                 Block_Width[ MiSize ] * Block_Height[ MiSize ] >= 128 ) {

             use_dip                                                              S()

             if ( use_dip ) {

                 dip_transpose                                                   L(1)

                 dip_mode                                                         S()

             }

         }

     }


```

<a id="s-5-20-7-10"></a>

##### § 5.20.7.10 Ref frames syntax

```text
§   5.20.7.10. Ref frames syntax

     read_ref_frames( ) {                                                     Descriptor

         if ( skip_mode ) {

             (RefFrame[ 0 ], RefFrame[ 1 ]) = skip_mode_frames( )

             return

         }

         bw4 = Num_4x4_Blocks_Wide[ MiSize ]

         bh4 = Num_4x4_Blocks_High[ MiSize ]

         if ( TipFrameMode != TIP_FRAME_DISABLED &&

             !skip_mode && Min( bw4, bh4 ) >= 2 &&

             MiSize == ChromaMiSize ) {

             tip_mode                                                             S()

             if ( tip_mode ) {

                 RefFrame[ 0 ] = TIP_FRAME

                 RefFrame[ 1 ] = NONE

                 return

             }

         }

         if ( seg_feature_active( SEG_LVL_SKIP ) ||

             seg_feature_active( SEG_LVL_GLOBALMV ) ) {

             RefFrame[ 0 ] = SkipSegFrame




    AV2 Specification                                                         Page 199 of 1169
             RefFrame[ 1 ] = NONE

         } else {

             if ( reference_select && is_comp_ref_allowed( ) ) {

                 comp_mode                                                    S()

             } else {

                 comp_mode = SINGLE_REFERENCE

             }

             if ( comp_mode == COMPOUND_REFERENCE ) {

                 read_compound_ref()

             } else {

                 RefFrame[ 0 ] = read_single_ref()

                 RefFrame[ 1 ] = NONE

             }

         }

     }


    where skip_mode_frames is specified as:

     skip_mode_frames() {
         for ( n = 0; n < NNumBuf; n++ ) {
             if ( NRefFrame[ n ][ 0 ] == TIP_FRAME ) {
                 return ( Min(ClosestPast, ClosestFuture),
                          Max(ClosestPast, ClosestFuture) )
             }
             if ( is_inter_ref_frame( NRefFrame[ n ][ 0 ] ) &&
                  is_inter_ref_frame( NRefFrame[ n ][ 1 ] ) ) {
                 return (NRefFrame[ n ][ 0 ], NRefFrame[ n ][ 1 ])
             }
             if ( is_inter_ref_frame( NRefFrame[ n ][ 0 ] ) ) {
                 break
             }
         }
         return (SkipModeFrame[ 0 ], SkipModeFrame[ 1 ])
     }


```

<a id="s-5-20-7-11"></a>

##### § 5.20.7.11 Read compound ref syntax

```text
§   5.20.7.11. Read compound ref syntax

     read_compound_ref() {                                                Descriptor

         RefFrame[ 0 ] = NumTotalRefs - 1

         RefFrame[ 1 ] = NumTotalRefs - 1

         nFound = 0

         for ( ref = 0; ref < NumTotalRefs - 1 && nFound < 2; ref++ ) {

             if ( nFound == 0 && ref == 2 ) {

                 comp_ref = 1

             } else if ( nFound == 0 &&

                    ref + 1 >= NumSameRefCompound &&

                    ref + 1 == NumTotalRefs - 1 ) {

                 comp_ref = 1

             } else {



    AV2 Specification                                                     Page 200 of 1169
                 comp_ref                                                           S()

             }

             if ( comp_ref ) {

                 RefFrame[ nFound ] = ref

                 nFound++

                 if ( ref < NumSameRefCompound ) {

                     ref--

                 }

             }

         }

     }


```

<a id="s-5-20-7-12"></a>

##### § 5.20.7.12 Read single ref syntax

```text
§   5.20.7.12. Read single ref syntax

     read_single_ref() {                                                        Descriptor

         for ( ref = 0; ref < NumTotalRefs - 1; ref++ ) {

             single_ref                                                             S()

             if ( single_ref ) {

                 return ref

             }

         }

         return        NumTotalRefs - 1

     }


```

<a id="s-5-20-7-13"></a>

##### § 5.20.7.13 Assign MV syntax

```text
§   5.20.7.13. Assign MV syntax

     assign_mv( isCompound ) {                                                  Descriptor

         mvdRead[ 0 ] = 0

         mvdRead[ 1 ] = 0

         baseList = 0

         firstDist = 0

         secondDist = 0

         if (is_joint_mvd_coding_mode(YMode)) {

             firstDist = Abs(get_relative_dist( OrderHints[ RefFrame[ 0 ] ],

                                OrderHint ))

             secondDist = Abs(get_relative_dist( OrderHints[ RefFrame[ 1 ] ],

                                OrderHint ))

             restrict0 = OrderHints[ RefFrame[ 0 ] ] == RESTRICTED_OH

             restrict1 = OrderHints[ RefFrame[ 1 ] ] == RESTRICTED_OH

             if ( firstDist < secondDist || ( !restrict0 && restrict1 ) ) {

                 baseList = 1

                 (firstDist, secondDist) = (secondDist, firstDist)

             }




    AV2 Specification                                                           Page 201 of 1169
       if (!is_same_side()) {

           secondDist = -secondDist

       }

   }

   for ( i = 0; i < 1 + isCompound; i++ ) {

       if ( use_intrabc ) {

           compMode = intrabc_mode ? NEARMV : NEWMV

       } else {

           compMode = get_mode( i, baseList )

       }

       if ( use_intrabc ) {

           PredMvs[ 0 ] = RefStackMv[ RefMvIdx ][ 0 ]

       } else if ( compMode == GLOBALMV ) {

           PredMvs[ i ] = GlobalMvs[ i ]

       } else if ( compMode == WARPMV ) {

           PredMvs[ 0 ] = get_warp_motion_vector(

                      WarpParamStack[ RefWarpIdx ],

                      warpmv_with_mvd ? FrameMvPrecision :

                             MV_PRECISION_EIGHTH_PEL)

       } else if (has_second_drl(YMode)) {

           if ( i == 0 ) {

               PredMvs[ i ] = RefStack0Mvs[ RefMvIdx0 ]

           } else {

               PredMvs[ i ] = RefStack1Mvs[ RefMvIdx1 ]

           }

       } else {

           PredMvs[ i ] = RefStackMv[ RefMvIdx ][ i ]

       }

       if ( compMode == NEWMV || warpmv_with_mvd || compMode == WARP_NEWMV ) {

           if ( !warpmv_with_mvd && MvPrecision < MV_PRECISION_HALF_PEL &&

               !IsAdaptiveMvd ) {

               lower_mv_precision( MvPrecision, PredMvs[ i ] )

           }

           diffMvs[ i ] = read_mv( )

           mvdRead[ i ] = 1

       } else {

           for ( comp = 0; comp < 2; comp++ ) {

               diffMvs[ i ][ comp ] = 0

           }

       }

   }




AV2 Specification                                                                Page 202 of 1169
   shift = MV_PRECISION_EIGHTH_PEL - MvPrecision

   lastSign = 0

   numNonzero = 0

   for ( i = 0; i < 1 + isCompound; i++ ) {

       if ( mvdRead[ i ] ) {

           for ( comp = 0; comp < 2; comp++ ) {

               if ( diffMvs[ i ][ comp ] != 0 ) {

                   lastRef = i

                   lastComp = comp

                   lastSign += diffMvs[ i ][ comp ] >> shift

                   numNonzero++

               }

           }

       }

   }

   thresh = YMode == NEW_NEWMV ? 4 : 1

   allowed = is_mvd_sign_derive_allowed(isCompound) && numNonzero >= thresh

   for ( i = 0; i < 1 + isCompound; i++ ) {

       if ( mvdRead[ i ] ) {

           for ( comp = 0; comp < 2; comp++ ) {

               if ( diffMvs[ i ][ comp ] != 0 ) {

                   if ( allowed && i == lastRef && comp == lastComp ) {

                       mv_sign = lastSign & 1

                   } else {

                       mv_sign                                                      L(1)

                   }

                   diffMvs[ i ][ comp ] = mv_sign ? -diffMvs[ i ][ comp ] :

                                   diffMvs[ i ][ comp ]

               }

           }

       }

   }

   if ( is_joint_mvd_coding_mode( YMode ) ) {

       projMv = get_mv_projection( diffMvs[ baseList ], secondDist, firstDist)

       if ( use_amvd ) {

           for ( comp = 0; comp < 2; comp++ ) {

               if ( jmvd_scale_mode == 1 ) {

                   projMv[ comp ] = projMv[ comp ] * 2

               } else if ( jmvd_scale_mode == 2 ) {

                   projMv[ comp ] = projMv[ comp ] / 2

               }




AV2 Specification                                                                Page 203 of 1169
             }

         } else if ( jmvd_scale_mode > 0 ) {

             comp = (jmvd_scale_mode - 1) & 1

             if ( jmvd_scale_mode <= 2 ) {

                 projMv[ comp ] = projMv[ comp ] * 2

             } else {

                 projMv[ comp ] = projMv[ comp ] / 2

             }

         }

         for ( comp = 0; comp < 2; comp++ ) {

             BlockMvs[ baseList ][ comp ] = mv_clamp_to_integer(

                 PredMvs[ baseList ][ comp ] + diffMvs[ baseList ][ comp ] )

             BlockMvs[ 1 - baseList ][ comp ] = mv_clamp_to_integer(

                 PredMvs[ 1 - baseList ][ comp ] + projMv[ comp ] )

         }

     } else {

         for ( i = 0; i < 1 + isCompound; i++ ) {

             for ( comp = 0; comp < 2; comp++ ) {

                 BlockMvs[ i ][ comp ] = mv_clamp_to_integer(

                  PredMvs[ i ][ comp ] + diffMvs[ i ][ comp ] )

             }

         }

     }

 }


where the function is_same_side is defined as:

 is_same_side() {
     return ( FrameDistance[ RefFrame[ 0 ] ] < 0 &&
              FrameDistance[ RefFrame[ 1 ] ] < 0) ||
            ( FrameDistance[ RefFrame[ 0 ] ] > 0 &&
              FrameDistance[ RefFrame[ 1 ] ] > 0)
 }


and the function is_mvd_sign_derive_allowed is defined as:

 is_mvd_sign_derive_allowed(isCompound) {
     if ( use_intrabc ||
          !enable_mvd_sign_derive ||
          motion_mode != SIMPLE ||
          IsAdaptiveMvd || skip_mode ||
          allow_screen_content_tools ||
          FrameMvPrecision > MV_PRECISION_QUARTER_PEL ||
          MvPrecision >= MV_PRECISION_QUARTER_PEL ||
          has_nearmv(YMode) ) {
         return 0
     }
     if ( isCompound ) {
         return RefMvIdx == 0
     } else {



AV2 Specification                                                              Page 204 of 1169
                  return 1
              }
     }


    and the function lower_mv_precision (which modifies the contents of the input motion vector to the target
    precision) is defined as:

     lower_mv_precision( precision, candMv ) {
         bits = MV_PRECISION_EIGHTH_PEL - precision
         radix = 1 << bits
         for ( i = 0; i < 2; i++ ) {
             a = Abs( candMv[ i ] )
             aInt = Round2( a - 1, bits )
             if ( candMv[ i ] >= 0 ) {
                 candMv[ i ] = aInt << bits
             } else {
                 candMv[ i ] = (-aInt) << bits
             }
             if ((aInt << bits) != a) {
                 candMv[ i ] = Clip3( MV_LOW + radix, MV_UPP - radix, candMv[ i ] )
             }
         }
     }


    and the function mv_clamp_to_integer (which adjusts a motion vector component to an integer location if
    it would have overflowed the allowed range) is defined as:

     mv_clamp_to_integer( v ) {
         if ( v < MV_LOW + 1 ) {
             return MV_LOW + 8
         } else if ( v > MV_UPP - 1 ) {
             return MV_UPP - 8
         } else {
             return v
         }
     }


```

<a id="s-5-20-7-14"></a>

##### § 5.20.7.14 Read motion mode syntax

```text
§   5.20.7.14. Read motion mode syntax

     read_motion_mode( isCompound ) {                                                           Descriptor

         motion_mode_allowed( isCompound )

         inter_intra = 0

         localAllowed = AllowedMotionModes[ LOCALWARP ] &&

                  frame_enabled_motion_modes[ LOCALWARP ]

         if ( YMode == WARPMV ) {

             return DELTAWARP

         }

         if ( YMode == WARP_NEWMV ) {

             extendAllowed = AllowedMotionModes[ EXTENDWARP ] &&

                    frame_enabled_motion_modes[ EXTENDWARP ]

             if ( extendAllowed ) {

              use_extend_warp                                                                      S()

              if ( use_extend_warp ) {




    AV2 Specification                                                                          Page 205 of 1169
                 return EXTENDWARP

             }

         }

         if ( localAllowed ) {

             use_local_warp                                                           S()

             if ( use_local_warp ) {

                 return LOCALWARP

             }

         }

         return DELTAWARP

     }

     if ( AllowedMotionModes[ INTERINTRA ] &&

         frame_enabled_motion_modes[ INTERINTRA ] ) {

         read_interintra_mode( 0 )

         if ( inter_intra ) {

             return INTERINTRA

         }

     }

     if ( localAllowed ) {

         use_local_warp                                                               S()

         if ( use_local_warp ) {

             return LOCALWARP

         }

     }

     return SIMPLE

 }


The function motion_mode_allowed works out the allowed motion modes as follows:

 motion_mode_allowed(isCompound) {
     for ( i = 0; i < MOTION_MODES; i++ ) {
         AllowedMotionModes[ i ] = 0
     }
     if ( YMode == WARPMV ) {
         AllowedMotionModes[ DELTAWARP ] = 1
         return
     }
     if ( YMode == WARP_NEWMV ) {
         AllowedMotionModes[ LOCALWARP ] = WarpSampleFound[ 0 ]
         AllowedMotionModes[ EXTENDWARP ] = WarpSampleFound[ 0 ]
         AllowedMotionModes[ DELTAWARP ] = 1
         return
     }
     if ( skip_mode || RefFrame[ 0 ] == INTRA_FRAME || use_bawp ||
          RefFrame[ 0 ] == TIP_FRAME ||
          seg_feature_active(SEG_LVL_SKIP) ||
          seg_feature_active(SEG_LVL_GLOBALMV) ||
          ( isCompound && is_thin_4xn_nx4_block() ) ) {
         return
     }
     AllowedMotionModes[ INTERINTRA ] = (!isCompound &&



AV2 Specification                                                                 Page 206 of 1169
                                                     MiSize >= BLOCK_8X8 &&
                                                     Block_Width[ MiSize ] <= 64 &&
                                                     Block_Height[ MiSize ] <= 64)
                 if ( RefFrame[ 0 ] == RefFrame[ 1 ] ) {
                     return
                 }
                 if ( !force_integer_mv &&
                      ( YMode == GLOBALMV || YMode == GLOBAL_GLOBALMV ) &&
                      GmType[ RefFrame[ 0 ] ] > IDENTITY ) {
                     return
                 }
                 if ( Min( Block_Width[ MiSize ], Block_Height[ MiSize ] ) < 8 ) {
                     return
                 }
                 AllowedMotionModes[ LOCALWARP ] = !force_integer_mv && YMode == NEW_NEWMV &&
                                                 !use_optflow &&
                                                 opfl_refine_type != REFINE_ALL &&
                                                 WarpSampleFound[ 0 ] &&
                                                 WarpSampleFound[ 1 ]
     }


    where is_scaled is a function that determines whether a reference frame uses scaling and is specified as:

     is_scaled( refFrame, checkRestricted ) {
         if ( checkRestricted && OrderHints[ refFrame ] == RESTRICTED_OH ) {
             return 1
         }
         refIdx = ref_frame_idx[ refFrame ]
         xScale = ( ( RefFrameWidth[ refIdx ] << REF_SCALE_SHIFT ) +
                      ( FrameWidth / 2 ) ) / FrameWidth
         yScale = ( ( RefFrameHeight[ refIdx ] << REF_SCALE_SHIFT ) +
                      ( FrameHeight / 2 ) ) / FrameHeight
         noScale = 1 << REF_SCALE_SHIFT
         return xScale != noScale || yScale != noScale
     }


    and is_thin_4xn_nx4_block is a function that tests the block size as follows:

     is_thin_4xn_nx4_block( ) {
         w = Block_Width[ MiSize ]
         h = Block_Height[ MiSize ]
         return (w == 4 && h >= 16) || (h == 4 && w >= 16)
     }


```

<a id="s-5-20-7-15"></a>

##### § 5.20.7.15 Read inter intra syntax

```text
§   5.20.7.15. Read inter intra syntax

     read_interintra_mode( isWarp ) {                                                            Descriptor

         if ( isWarp ) {

             if ( Block_Width[ MiSize ] <= 64 && Block_Height[ MiSize ] <= 64 ) {

                 warp_inter_intra                                                                   S()

                 inter_intra = warp_inter_intra

             } else {

                 inter_intra = 0

             }

         } else {

             inter_intra                                                                            S()

         }




    AV2 Specification                                                                           Page 207 of 1169
         if ( inter_intra ) {

             interintra_mode                                                   S()

             RefFrame[ 1 ] = INTRA_FRAME

             AngleDeltaY = 0

             AngleDeltaUV = 0

             UVMode = DC_PRED

             if ( Wedge_Bits[ MiSize ] == 0 ) {

                 wedge_interintra = 0

             } else {

                 wedge_interintra                                              S()

             }

             if ( wedge_interintra ) {

                 read_wedge_mode()

                 wedge_sign = 0

             }

         }

     }


```

<a id="s-5-20-7-16"></a>

##### § 5.20.7.16 Read compound type syntax

```text
§   5.20.7.16. Read compound type syntax

     read_compound_type( isCompound ) {                                    Descriptor

         comp_group_idx = 0

         if ( skip_mode || use_optflow ||

             ( YMode == JOINT_NEWMV && use_amvd ) ||

             ( use_refinemv && is_switchable_refinemv() ) ) {

             compound_type = COMPOUND_AVERAGE

             return

         }

         if ( isCompound ) {

             n = Wedge_Bits[ MiSize ]

             if ( enable_masked_compound && !is_thin_4xn_nx4_block() ) {

                 comp_group_idx                                                S()

                 if ( comp_group_idx != 0 && use_refinemv ) {

                     DecidedAgainstRefinemv = 1

                     use_refinemv = 0

                 }

             }

             if ( comp_group_idx == 0 ) {

                 compound_type = COMPOUND_AVERAGE

             } else {

                 if ( n == 0 ) {

                     compound_type = COMPOUND_DIFFWTD




    AV2 Specification                                                      Page 208 of 1169
                 } else {

                     compound_type                                                        S()

                 }

             }

             if ( compound_type == COMPOUND_WEDGE ) {

                 read_wedge_mode()

                 wedge_sign                                                              L(1)

             } else if ( compound_type == COMPOUND_DIFFWTD ) {

                 mask_type                                                               L(1)

             }

         } else {

             if ( inter_intra ) {

                 compound_type = wedge_interintra ? COMPOUND_WEDGE : COMPOUND_INTRA

             } else {

                 compound_type = COMPOUND_AVERAGE

             }

         }

     }


```

<a id="s-5-20-7-17"></a>

##### § 5.20.7.17 Read refine mv syntax

```text
§   5.20.7.17. Read refine mv syntax

     read_refinemv( isCompound ) {                                                    Descriptor

         use_refinemv = 0

         DecidedAgainstRefinemv = 0

         if ( enable_refinemv &&

                 isCompound &&

                 (Block_Width[ MiSize ] >= 16 || Block_Height[ MiSize ] >= 16) &&

                 (Block_Width[ MiSize ] >= 8 && Block_Height[ MiSize ] >= 8) &&

                 is_refinemv_allowed_mode() &&

                 is_refinemv_allowed_reference(RefFrame)

             ) {

             if (is_switchable_refinemv()) {

                 use_refinemv                                                             S()

             } else {

                 use_refinemv = 1

             }

         }

     }




    AV2 Specification                                                                 Page 209 of 1169
    where the functions is_refinemv_allowed_mode, is_switchable_refinemv, is_refinemv_allowed_reference
    are specified as:

     is_refinemv_allowed_mode() {
         if ( skip_mode || YMode == GLOBAL_GLOBALMV || motion_mode != SIMPLE ) {
             return 0
         }
         if ( opfl_refine_type == REFINE_SWITCHABLE &&
              has_newmv( YMode ) &&
              !use_optflow ) {
             return 0
         }
         return 1
     }

     is_switchable_refinemv() {
         if ( YMode == NEAR_NEARMV ||
              (YMode == JOINT_NEWMV && use_optflow &&
                  opfl_refine_type == REFINE_SWITCHABLE)) {
             return 0
         }
         return 1
     }

     is_refinemv_allowed_reference( refFrames ) {
         if ( FrameType == SWITCH_FRAME ||
              is_scaled( refFrames[ 0 ], 1 ) ||
              is_scaled( refFrames[ 1 ], 1 ) ) {
             return 0
         }
         d0 = get_relative_dist( OrderHint, OrderHints[ refFrames[ 0 ] ] )
         d1 = get_relative_dist( OrderHint, OrderHints[ refFrames[ 1 ] ] )
         return d0 != 0 && d0 == -d1
     }


```

<a id="s-5-20-7-18"></a>

##### § 5.20.7.18 Read wedge mode syntax

```text
§   5.20.7.18. Read wedge mode syntax

     read_wedge_mode() {                                                                     Descriptor

         wedge_quad                                                                             S()

         wedge_angle                                                                            S()

         wedgeAngle = wedge_quad * 5 + wedge_angle

         if ( (wedgeAngle >= H_WEDGE_ANGLES) ||

             (wedgeAngle == WEDGE_90) ||

             (wedgeAngle == WEDGE_0) ) {

             wedge_dist2                                                                        S()

             wedgeDist = wedge_dist2 + 1

         } else {

             wedge_dist1                                                                        S()

             wedgeDist = wedge_dist1

         }

         WedgeIndex = Wedge_Angle_Dist_2_Index[ wedgeAngle ][ wedgeDist ]

     }




    AV2 Specification                                                                       Page 210 of 1169
    where the lookup table Wedge_Angle_Dist_2_Index is specified as:

     Wedge_Angle_Dist_2_Index[ WEDGE_ANGLES ][ NUM_WEDGE_DIST ] = {
         { -1, 0, 1, 2 },
         { 3, 4, 5, 6 },
         { 7, 8, 9, 10 },
         { 11, 12, 13, 14 },
         { 15, 16, 17, 18 },
         { -1, 19, 20, 21 },
         { 22, 23, 24, 25 },
         { 26, 27, 28, 29 },
         { 30, 31, 32, 33 },
         { 34, 35, 36, 37 },
         { -1, 38, 39, 40 },
         { -1, 41, 42, 43 },
         { -1, 44, 45, 46 },
         { -1, 47, 48, 49 },
         { -1, 50, 51, 52 },
         { -1, 53, 54, 55 },
         { -1, 56, 57, 58 },
         { -1, 59, 60, 61 },
         { -1, 62, 63, 64 },
         { -1, 65, 66, 67 }
     }


```

<a id="s-5-20-7-19"></a>

##### § 5.20.7.19 Get mode function

```text
§   5.20.7.19. Get mode function

     get_mode( refList, baseList ) {
         if ( YMode == JOINT_NEWMV ) {
             if ( refList == baseList ) {
                  compMode = NEWMV
             } else {
                  compMode = NEARMV
             }
         } else if ( refList == 0 ) {
             if ( YMode == NEW_NEWMV || YMode == NEW_NEARMV ) {
                  compMode = NEWMV
             } else if ( YMode < NEAR_NEARMV ) {
                  compMode = YMode
             } else if ( YMode == NEAR_NEARMV || YMode == NEAR_NEWMV ) {
                  compMode = NEARMV
             } else {
                  compMode = GLOBALMV
             }
         } else {
             if ( YMode == NEW_NEWMV || YMode == NEAR_NEWMV ) {
                  compMode = NEWMV
             } else if ( YMode == NEAR_NEARMV || YMode == NEW_NEARMV ) {
                  compMode = NEARMV
             } else {
                  compMode = GLOBALMV
             }
         }
         return compMode
     }


```

<a id="s-5-20-7-20"></a>

##### § 5.20.7.20 MV syntax

```text
§   5.20.7.20. MV syntax

     read_mv( ) {                                                          Descriptor

       diffMv[ 0 ] = 0

       diffMv[ 1 ] = 0

       if ( use_intrabc ) {

         MvCtx = MV_INTRABC_CONTEXT



    AV2 Specification                                                      Page 211 of 1169
   } else {

       MvCtx = 0

   }

   if ( IsAdaptiveMvd ) {

       mv_joint                                                                      S()

       if ( mv_joint == MV_JOINT_HZVNZ || mv_joint == MV_JOINT_HNZVNZ ) {

           diffMv[ 0 ] = read_mv_component( 0 )

       }

       if ( mv_joint == MV_JOINT_HNZVZ || mv_joint == MV_JOINT_HNZVNZ ) {

           diffMv[ 1 ] = read_mv_component( 1 )

       }

   } else {

       shell_set                                                                     S()

       shell_class                                                                   S()

       shellClass = shell_class

       if ( shell_set ) {

           shellClass += (11 + MvPrecision) >> 1

           if ( MvPrecision == MV_PRECISION_EIGHTH_PEL && shell_class == 7 ) {

               joint_shell_last_two_classes                                          S()

               shellClass += joint_shell_last_two_classes

           }

       }

       shellClassOffset = 0

       if ( shellClass < 2 ) {

           shell_offset_low_class                                                    S()

           shellClassOffset = shell_offset_low_class

       } else if ( shellClass == 2 ) {

           for ( i = 0; i < 3; i++ ) {

               if ( i == 0 ) {

                   shell_offset_class2                                               S()

                   shellClassOffset = shell_offset_class2

               } else {

                   shell_offset_class2_high                                         L(1)

                   shellClassOffset = shell_offset_class2_high + i

               }

               if ( shellClassOffset == i ) {

                   break

               }

           }

       } else {

           for ( i = 0; i < shellClass; i++ ) {

               shell_offset_other_class



AV2 Specification                                                                Page 212 of 1169
                                                                                   S()

             shellClassOffset |= shell_offset_other_class << i

         }

     }

     shellClassBaseIndex = (shellClass == 0) ? 0 : (1 << shellClass)

     shellIndex = shellClassBaseIndex + shellClassOffset

     if ( shellIndex > 0 ) {

         col = 0

         maximumPairIndex = shellIndex >> 1

         if ( maximumPairIndex > 0 ) {

             maxIdxBits = Min(maximumPairIndex, MAX_COL_TRUNCATED_UNARY_VAL)

             for ( i = 0; i < maxIdxBits; i++ ) {

                 col_mv_greater                                                    S()

                 col = i + col_mv_greater

                 if ( col_mv_greater == 0 ) {

                     break

                 }

             }

             if ( maximumPairIndex > MAX_COL_TRUNCATED_UNARY_VAL &&

                 col == MAX_COL_TRUNCATED_UNARY_VAL ) {

                 n = maximumPairIndex - 1

                 col_remainder                                                    NS(n)

                 col = col_remainder + MAX_COL_TRUNCATED_UNARY_VAL

             }

         }

         skipCodingColBit = (col == maximumPairIndex) &&

                       ((shellIndex & 1) == 0)

         if ( skipCodingColBit ) {

             diffMv[ 1 ] = maximumPairIndex

         } else {

             col_mv_index                                                          S()

             if ( col_mv_index == 0 ) {

                 diffMv[ 1 ] = col

             } else {

                 diffMv[ 1 ] = shellIndex - col

             }

         }

         diffMv[ 0 ] = shellIndex - diffMv[ 1 ]

         shift = MV_PRECISION_EIGHTH_PEL - MvPrecision

         diffMv[ 0 ] = diffMv[ 0 ] << shift

         diffMv[ 1 ] = diffMv[ 1 ] << shift

     }



AV2 Specification                                                              Page 213 of 1169
         }

         return diffMv

     }


```

<a id="s-5-20-7-21"></a>

##### § 5.20.7.21 MV component syntax

```text
§   5.20.7.21. MV component syntax

     read_mv_component( comp ) {                                                       Descriptor

         amvd_index                                                                        S()

         return Amvd_Index_To_Mvd[ amvd_index ]

     }


    where the constant table Amvd_Index_To_Mvd is defined as:

     Amvd_Index_To_Mvd[ MAX_AMVD_INDEX ] = {
         2, 4, 6, 8, 16, 32, 64, 128
     }


```

<a id="s-5-20-7-22"></a>

##### § 5.20.7.22 Compute prediction syntax

```text
§   5.20.7.22. Compute prediction syntax

     compute_prediction() {                                                            Descriptor

         sbMask = Num_4x4_Blocks_Wide[ SbSize ] - 1

         for ( plane = PlaneStart; plane < 1 + HasChroma * 2; plane++ ) {

             planeSz = get_plane_residual_size( plane > 0 ? ChromaMiSize : MiSize,

                             plane )

             num4x4W = Num_4x4_Blocks_Wide[ planeSz ]

             num4x4H = Num_4x4_Blocks_High[ planeSz ]

             log2W = MI_SIZE_LOG2 + Mi_Width_Log2[ planeSz ]

             log2H = MI_SIZE_LOG2 + Mi_Height_Log2[ planeSz ]

             subX = (plane > 0) ? SubsamplingX : 0

             subY = (plane > 0) ? SubsamplingY : 0

             candRow = plane > 0 ? ChromaMiRow : MiRow

             candCol = plane > 0 ? ChromaMiCol : MiCol

             baseX = (candCol >> subX) * MI_SIZE

             baseY = (candRow >> subY) * MI_SIZE

             subBlockMiRow = candRow & sbMask

             subBlockMiCol = candCol & sbMask

             if ( FrameIsIntra ) {

                 sub8x8Inter = 0

             } else {

                 sub8x8Inter = (plane > 0 && MiSize != ChromaMiSize)

             }

             isInterIntra = is_inter && RefFrame[ 1 ] == INTRA_FRAME && !sub8x8Inter

             if ( isInterIntra ) {

                 if ( interintra_mode == II_DC_PRED ) {




    AV2 Specification                                                                  Page 214 of 1169
             mode = DC_PRED

         }

         else if ( interintra_mode == II_V_PRED ) mode = V_PRED

         else if ( interintra_mode == II_H_PRED ) mode = H_PRED

         else mode = SMOOTH_PRED

         predict_intra( plane, baseX, baseY,

                      plane == 0 ? AvailL : AvailLChroma,

                      plane == 0 ? AvailU : AvailUChroma,

                      count_top_right_avail( plane,

                               ( subBlockMiCol >> subX ),

                               ( subBlockMiRow >> subY ),

                               num4x4W),

                      count_bottom_left_avail( plane,

                                ( subBlockMiCol >> subX ),

                                ( subBlockMiRow >> subY ),

                                num4x4H),

                      mode,

                      log2W, log2H )

         for ( i = 0; i < num4x4H * 4; i++ ) {

             for ( j = 0; j < num4x4W * 4; j++ ) {

                 IntraPred[ i ][ j ] =

                     CurrFrame[ plane ][ baseY + i ][ baseX + j ]

             }

         }

     }

     if ( is_inter ) {

         for ( r = 0; r < num4x4H << subY ; r++ ) {

             for ( c = 0; c < num4x4W << subX ; c++ ) {

                 if ( FrameIsIntra ) {

                     doBlock = r==0 && c==0

                     predSize = plane > 0 ? ChromaMiSize : MiSize

                     mvRow = MiRow

                     mvCol = MiCol

                 } else {

                     mvRow = candRow + r

                     mvCol = candCol + c

                     doBlock = mvRow < MiRows && mvCol < MiCols &&

                          MiRowBase[ 0 ][ mvRow ][ mvCol ] == mvRow &&

                          MiColBase[ 0 ][ mvRow ][ mvCol ] == mvCol

                     predSize = MiSizes[ 0 ][ mvRow ][ mvCol ]

                 }




AV2 Specification                                                        Page 215 of 1169
                         if ( doBlock ) {

                             predW = Block_Width[ predSize ] >> subX

                             predH = Block_Height[ predSize ] >> subY

                             x = (c * 4) >> subX

                             y = (r * 4) >> subY

                             predict_inter( plane, baseX + x, baseY + y,

                                   predW, predH,

                                   mvRow, mvCol, 0, sub8x8Inter)

                         }

                     }

                 }

                 if ( isInterIntra ) {

                     h = num4x4H * 4

                     w = num4x4W * 4

                     if ( compound_type == COMPOUND_WEDGE && plane == 0 ) {

                         wedge_mask( w, h )

                     } else if (compound_type == COMPOUND_INTRA) {

                         intra_mode_variant_mask( w, h )

                     }

                     mask_blend( plane, baseX, baseY, w, h )

                 }

             }

         }

     }


```

<a id="s-5-20-7-23"></a>

##### § 5.20.7.23 Residual syntax

```text
§   5.20.7.23. Residual syntax

     residual( ) {                                                                      Descriptor

         widthChunks = Max( 1, Block_Width[ MiSize ] >> 6 )

         heightChunks = Max( 1, Block_Height[ MiSize ] >> 6 )

         miSizeChunk = ( widthChunks > 1 || heightChunks > 1 ) ? BLOCK_64X64 : MiSize

         doubleChromaW = SubsamplingX && widthChunks > 1 && !Lossless

         doubleChromaH = SubsamplingY && heightChunks > 1 && !Lossless

         for ( startChunkY = 0; startChunkY < heightChunks; startChunkY += 2 ) {

         for ( startChunkX = 0; startChunkX < widthChunks; startChunkX += 2 ) {

             for( chunkY = startChunkY;

                 chunkY < Min(startChunkY + 2, heightChunks) ; chunkY++ ) {

             for ( chunkX = startChunkX;

                     chunkX < Min(startChunkX + 2, widthChunks); chunkX++ ) {

                 miRowChunk = MiRow + ( chunkY << 4 )

                 miColChunk = MiCol + ( chunkX << 4 )

                 update_ibc_buffers( miRowChunk, miColChunk )




    AV2 Specification                                                                   Page 216 of 1169
       isCfl = !is_inter && UVMode == UV_CFL_PRED

       atStart = (!doubleChromaW || (chunkX&1) == 0) &&

               (!doubleChromaH || (chunkY&1) == 0)

       atEnd = (!doubleChromaW || (chunkX&1) == 1) &&

               (!doubleChromaH || (chunkY&1) == 1)

       if ( HasChroma && isCfl && (doubleChromaW || doubleChromaH) ) {

           doChromaParse = atStart

           doChromaRecon = 0

           doChromaReconAfter = atEnd

       } else {

           doChromaParse = HasChroma && atStart

           doChromaRecon = doChromaParse

           doChromaReconAfter = 0

       }

       for ( plane = PlaneStart; plane < 1 + doChromaParse * 2; plane++ ) {

           if ( plane > 0 && ChromaMiSize != MiSize ) {

               planeSz = get_plane_residual_size( ChromaMiSize, plane )

           } else {

               planeSz = get_plane_residual_size( miSizeChunk, plane )

           }

           num4x4W = Num_4x4_Blocks_Wide[ planeSz ]

           num4x4H = Num_4x4_Blocks_High[ planeSz ]

           doRecon = plane == 0 || doChromaRecon

           doPred = doRecon

           if ( plane > 0 && doubleChromaW ) {

               num4x4W = num4x4W << 1

           }

           if ( plane > 0 && doubleChromaH ) {

               num4x4H = num4x4H << 1

           }

           subX = (plane > 0) ? SubsamplingX : 0

           subY = (plane > 0) ? SubsamplingY : 0

           if ( miRowChunk < MiRows && miColChunk < MiCols ) {

               baseXBlock =

                (plane > 0 ? ChromaMiCol >> subX : MiCol) * MI_SIZE

               baseYBlock =

                (plane > 0 ? ChromaMiRow >> subY : MiRow) * MI_SIZE

               txSz = Lossless ? TX_4X4 : get_tx_size( plane, TX_4X4 )

               stepX = Tx_Width[ txSz ] >> 2

               stepY = Tx_Height[ txSz ] >> 2

               allowCorners = 1




AV2 Specification                                                             Page 217 of 1169
           if ( plane == 0 &&

             LumaTxScanOrder[ miRowChunk ][ miColChunk ] ) {

             for ( x4 = 0; x4 < num4x4W; x4 += stepX ) {

                 col = miColChunk + x4

                 for ( y4 = 0; y4 < num4x4H; y4 += stepY ) {

                     row = miRowChunk + y4

                     if ( row >= MiRows || col >= MiCols ) {

                         break

                     }

                     txSz = LumaTxSizes[ row ][ col ]

                     allowCorners = !LumaTxMiddle[ row ][ col ]

                     stepX = Tx_Width[ txSz ] >> 2

                     stepY = Tx_Height[ txSz ] >> 2

                     transform_block( plane, baseXBlock, baseYBlock,

                                 txSz,

                                 x4 + ( (chunkX << 4) >> subX ),

                                 y4 + ( (chunkY << 4) >> subY ),

                                 allowCorners, doParse = 1,

                                 doPred = 1, doRecon = 1,

                                 eob = 0 )

                 }

                 if ( col >= MiCols ) {

                     break

                 }

             }

           } else {

             for ( y4 = 0; y4 < num4x4H; y4 += stepY ) {

                 for ( x4 = 0; x4 < num4x4W; x4 += stepX ) {

                     if ( plane == 0 ) {

                         row = miRowChunk + y4

                         col = miColChunk + x4

                         if ( row >= MiRows ||    col >= MiCols ) {

                             break

                         }

                         txSz = LumaTxSizes[ row ][ col ]

                         allowCorners = !LumaTxMiddle[ row ][ col ]

                         stepX = Tx_Width[ txSz ] >> 2

                         stepY = Tx_Height[ txSz ] >> 2

                     }

                     eobs[ plane ] =

                         transform_block( plane, baseXBlock,




AV2 Specification                                                      Page 218 of 1169
                                             baseYBlock, txSz,

                                             x4 + ( (chunkX << 4) >> subX ),

                                             y4 + ( (chunkY << 4) >> subY ),

                                             allowCorners, doParse = 1,

                                             doPred, doRecon, eob = 0 )

                                 }

                                 if ( plane == 0 && row >= MiRows ) {

                                     break

                                 }

                             }

                         }

                     }

                 }

                 if ( doChromaReconAfter ) {

                     for ( plane = 1; plane < 3; plane++ ) {

                         miRowChunk = MiRow + ( (chunkY - doubleChromaH) << 4 )

                         miColChunk = MiCol + ( (chunkX - doubleChromaW) << 4 )

                         if ( miRowChunk < MiRows && miColChunk < MiCols ) {

                             subX = SubsamplingX

                             subY = SubsamplingY

                             baseXBlock = (ChromaMiCol >> subX) * MI_SIZE

                             baseYBlock = (ChromaMiRow >> subY) * MI_SIZE

                             txSz = get_tx_size( plane, TX_4X4 )

                             transform_block( plane, baseXBlock, baseYBlock, txSz,

                                 ( ( (chunkX - doubleChromaW) << 4 ) >> subX ),

                                 ( ( (chunkY - doubleChromaH) << 4 ) >> subY ),

                                 allowCorners = 1, doParse = 0, doPred = 1,

                                 doRecon = 1, eobs[ plane ] )

                         }

                     }

                 }

             }

             }

         }

         }

     }


```

<a id="s-5-20-7-24"></a>

##### § 5.20.7.24 Transform block syntax

```text
§   5.20.7.24. Transform block syntax

     transform_block( plane, baseX, baseY, txSz, x, y, allowCorners, doParse, doPred, doRecon,   Descriptor
     eob ) {

         startX = baseX + 4 * x

         startY = baseY + 4 * y



    AV2 Specification                                                                            Page 219 of 1169
   subX = (plane > 0) ? SubsamplingX : 0

   subY = (plane > 0) ? SubsamplingY : 0

   maxX = (MiCols * MI_SIZE) >> subX

   maxY = (MiRows * MI_SIZE) >> subY

   if ( startX >= maxX || startY >= maxY ) {

       return 0

   }

   row = ( startY << subY ) >> MI_SIZE_LOG2

   col = ( startX << subX ) >> MI_SIZE_LOG2

   if (plane == 0 || !is_cctx_allowed()) {

       if ( doPred ) {

           make_intra_prediction(plane,startX,startY,txSz,x,y,allowCorners)

       }

       if ( !skip_flag ) {

           if ( doParse ) {

               eob = coeffs( plane, startX, startY, txSz )

           }

           if ( doParse && eob > 0 ) {

               dequant( plane, txSz )

               save_dequant(plane, txSz)

           }

           if ( doRecon && eob > 0 ) {

               get_dequant(plane, txSz, CCTX_NONE)

               reconstruct( plane, startX, startY, txSz )

           }

       }

       store_tx_info( plane, row, col, txSz, eob, doParse, doPred )

       return eob

   } else if ( plane == 1 ) {

       return 0

   } else {

       if ( doParse && !skip_flag ) {

           for ( p = 1; p <= 2; p++ ) {

               eob = coeffs( p, startX, startY, txSz )

               CctxEobs[ p ] = eob

               if ( eob > 0 ) {

                   dequant( p, txSz )

               }

               save_dequant(p, txSz)

           }

       }




AV2 Specification                                                             Page 220 of 1169
         for ( p = 1; p <= 2; p++ ) {

             if ( doPred ) {

                 make_intra_prediction( p, startX, startY, txSz, x, y,

                            allowCorners)

             }

             if ( doRecon && !skip_flag ) {

                 planeEob = CctxEobs[ p ]

                 if ( planeEob > 0 || cctx_type != CCTX_NONE ) {

                     get_dequant(p, txSz, cctx_type)

                     reconstruct( p, startX, startY, txSz )

                 }

             }

             store_tx_info( p, row, col, txSz, 0, doParse, doPred )

         }

         return 0

     }

 }


The function store_tx_info is defined as:

 store_tx_info(plane, row, col, txSz, eob, doParse, doPred) {
     subX = (plane > 0) ? SubsamplingX : 0
     subY = (plane > 0) ? SubsamplingY : 0
     sbMask = Num_4x4_Blocks_Wide[ SbSize ] - 1
     subBlockMiRow = row & sbMask
     subBlockMiCol = col & sbMask
     stepX = Tx_Width[ txSz ] >> MI_SIZE_LOG2
     stepY = Tx_Height[ txSz ] >> MI_SIZE_LOG2
     for ( i = 0; i < stepY; i++ ) {
         for ( j = 0; j < stepX; j++ ) {
             if ( doParse ) {
                 if ( plane == 0 ) {
                     LrTxSkip[ row + i ][ col + j ] = skip_flag || (eob == 0)
                 }
                 DeblockingTxSizes[ plane ]
                                      [ (row >> subY) + i ]
                                      [ (col >> subX) + j ] = txSz
                 TxColBase[ plane ]
                              [ (row >> subY) + i ]
                              [ (col >> subX) + j ] = col
                 TxRowBase[ plane ]
                              [ (row >> subY) + i ]
                              [ (col >> subX) + j ] = row
             }
             if ( doPred ) {
                 BlockDecoded[ plane ]
                              [ ( subBlockMiRow >> subY ) + i ]
                              [ ( subBlockMiCol >> subX ) + j ] = 1
             }
         }
     }
 }




AV2 Specification                                                               Page 221 of 1169
The function make_intra_prediction (which calls intra prediction processes) is defined as:

 make_intra_prediction(plane, startX, startY, txSz, x, y, allowCorners) {
     if ( !is_inter ) {
         stepX = Tx_Width[ txSz ] >> MI_SIZE_LOG2
         stepY = Tx_Height[ txSz ] >> MI_SIZE_LOG2
         subX = (plane > 0) ? SubsamplingX : 0
         subY = (plane > 0) ? SubsamplingY : 0
         row = ( startY << subY ) >> MI_SIZE_LOG2
         col = ( startX << subX ) >> MI_SIZE_LOG2
         sbMask = Num_4x4_Blocks_Wide[ SbSize ] - 1
         subBlockMiRow = row & sbMask
         subBlockMiCol = col & sbMask
         if ( plane == 0 && PaletteSizeY ) {
             predict_palette( startX, startY, x, y, txSz )
         } else {
             isCfl = ( plane > 0 && UVMode == UV_CFL_PRED )
             if ( plane == 0 ) {
                  mode = YMode
             } else {
                  mode = ( isCfl ) ? DC_PRED : UVMode
             }
             log2W = Tx_Width_Log2[ txSz ]
             log2H = Tx_Height_Log2[ txSz ]
             predict_intra( plane, startX, startY,
                               ( plane == 0 ? AvailL : AvailLChroma ) || x > 0,
                               ( plane == 0 ? AvailU : AvailUChroma ) || y > 0,
                               allowCorners ? count_top_right_avail( plane,
                                                  ( subBlockMiCol >> subX ),
                                                  ( subBlockMiRow >> subY ),
                                                  stepX) : 0,
                               allowCorners ? count_bottom_left_avail( plane,
                                                  ( subBlockMiCol >> subX ),
                                                  ( subBlockMiRow >> subY ),
                                                  stepY) : 0,
                               mode,
                               log2W, log2H )
             if ( isCfl ) {
                  predict_chroma_from_luma( plane, startX, startY, txSz )
             }
         }
     }
 }


The functions count_top_right_avail and count_bottom_left_avail (which count how many samples have
already been decoded in the corners) are defined as:

 count_top_right_avail(plane, x4, y4, w4) {
     numTopRight = 0
     for ( i = 0; i < w4; i++ ) {
         if ( BlockDecoded[ plane ][ y4 - 1 ][ x4 + w4 + i ] ) {
             numTopRight = i + 1
         } else {
             break
         }
     }
     return numTopRight
 }

 count_bottom_left_avail(plane, x4, y4, h4) {
     numBottomLeft = 0
     for ( i = 0; i < h4; i++ ) {
         if ( BlockDecoded[ plane ][ y4 + h4 + i ][ x4 - 1 ] ) {
             numBottomLeft = i + 1
         } else {
             break
         }



AV2 Specification                                                                            Page 222 of 1169
          }
          return numBottomLeft
     }


```

<a id="s-5-20-7-25"></a>

##### § 5.20.7.25 Get TX size function

```text
§   5.20.7.25. Get TX size function

     get_tx_size( plane, txSz ) {
         if ( plane == 0 ) {
             return txSz
         }
         uvTx = Max_Tx_Size_Rect[ get_plane_residual_size( ChromaMiSize, plane ) ]
         return uvTx
     }


```

<a id="s-5-20-7-26"></a>

##### § 5.20.7.26 Get plane residual size function

```text
§   5.20.7.26. Get plane residual size function

    The get_plane_residual_size function returns the size of a residual block for the specified plane. (The
    residual block will always have width and height at least equal to 4.)

     get_plane_residual_size( subsize, plane ) {
         subx = plane > 0 ? SubsamplingX : 0
         suby = plane > 0 ? SubsamplingY : 0
         return Subsampled_Size[ subsize ][ subx ][ suby ]
     }


    The Subsampled_Size table is defined as:

     Subsampled_Size[ BLOCK_SIZES ][ 2 ][ 2 ] = {
       { { BLOCK_4X4,    BLOCK_4X4},      {BLOCK_4X4,     BLOCK_4X4} },
       { { BLOCK_4X8,    BLOCK_4X4},      {BLOCK_INVALID, BLOCK_4X4} },
       { { BLOCK_8X4,    BLOCK_INVALID}, {BLOCK_4X4,      BLOCK_4X4} },
       { { BLOCK_8X8,    BLOCK_8X4},      {BLOCK_4X8,     BLOCK_4X4} },
       { {BLOCK_8X16,    BLOCK_8X8},      {BLOCK_4X16,    BLOCK_4X8} },
       { {BLOCK_16X8,    BLOCK_16X4},     {BLOCK_8X8,     BLOCK_8X4} },
       { {BLOCK_16X16,   BLOCK_16X8},     {BLOCK_8X16,    BLOCK_8X8} },
       { {BLOCK_16X32,   BLOCK_16X16},    {BLOCK_8X32,    BLOCK_8X16} },
       { {BLOCK_32X16,   BLOCK_32X8},     {BLOCK_16X16,   BLOCK_16X8} },
       { {BLOCK_32X32,   BLOCK_32X16},    {BLOCK_16X32,   BLOCK_16X16} },
       { {BLOCK_32X64,   BLOCK_32X32},    {BLOCK_16X64,   BLOCK_16X32} },
       { {BLOCK_64X32,   BLOCK_64X16},    {BLOCK_32X32,   BLOCK_32X16} },
       { {BLOCK_64X64,   BLOCK_64X32},    {BLOCK_32X64,   BLOCK_32X32} },
       { {BLOCK_64X128, BLOCK_64X64},     {BLOCK_INVALID, BLOCK_32X64} },
       { {BLOCK_128X64, BLOCK_INVALID}, {BLOCK_64X64,     BLOCK_64X32} },
       { {BLOCK_128X128, BLOCK_128X64},   {BLOCK_64X128, BLOCK_64X64} },
       { {BLOCK_128X256, BLOCK_128X128 }, {BLOCK_INVALID, BLOCK_64X128 } },
       { {BLOCK_256X128, BLOCK_INVALID }, {BLOCK_128X128, BLOCK_128X64 } },
       { {BLOCK_256X256, BLOCK_256X128 }, {BLOCK_128X256, BLOCK_128X128 } },
       { {BLOCK_4X16,    BLOCK_4X8},      {BLOCK_INVALID, BLOCK_4X8} },
       { {BLOCK_16X4,    BLOCK_INVALID}, {BLOCK_8X4,      BLOCK_8X4} },
       { {BLOCK_8X32,    BLOCK_8X16 },    { BLOCK_4X32,   BLOCK_4X16 } },
       { {BLOCK_32X8,    BLOCK_32X4 },    { BLOCK_16X8,   BLOCK_16X4 } },
       { {BLOCK_16X64,   BLOCK_16X32 },   { BLOCK_8X64,   BLOCK_8X32 } },
       { {BLOCK_64X16,   BLOCK_64X8 },    { BLOCK_32X16, BLOCK_32X8 } },
       { {BLOCK_4X32,    BLOCK_4X16},     { BLOCK_INVALID,BLOCK_4X16 } },
       { {BLOCK_32X4,    BLOCK_INVALID }, { BLOCK_16X4,   BLOCK_16X4 } },
       { {BLOCK_8X64,    BLOCK_8X32 },    { BLOCK_INVALID,BLOCK_4X32 } },
       { {BLOCK_64X8,    BLOCK_INVALID }, { BLOCK_32X8,   BLOCK_32X4 } }
     }




    AV2 Specification                                                                             Page 223 of 1169
```

<a id="s-5-20-7-27"></a>

##### § 5.20.7.27 Coefficients syntax

```text
§   5.20.7.27. Coefficients syntax

     coeffs( plane, startX, startY, txSz ) {                                 Descriptor

       x4 = startX >> 2

       y4 = startY >> 2

       w4 = Tx_Width[ txSz ] >> 2

       h4 = Tx_Height[ txSz ] >> 2

       txSzCtx = ( Tx_Size_Sqr[ txSz ] + Tx_Size_Sqr_Up[ txSz ] + 1 ) >> 1

       ptype = plane > 0

       segEob = Min( 32, Tx_Width[ txSz ] ) * Min( Tx_Height[ txSz ], 32 )

       for ( c = 0; c < segEob; c++ ) {

           Quant[ c ] = 0

           QuantSign[ c ] = 0

       }

       for ( i = 0; i < Min(32, 4 * h4); i++ ) {

           for ( j = 0; j < Min(32, 4 * w4); j++ ) {

               Dequant[ i ][ j ] = 0

               Level[ i ][ j ] = 0

           }

       }

       eob = 0

       culLevel = 0

       dcCategory = 0

       all_zero                                                                  S()

       if ( all_zero ) {

           if ( plane == 1 ) {

               EobU = 0

               cctx_type = 0

           }

           c = 0

           if ( plane == 0 ) {

               for ( i = 0; i < w4; i++ ) {

                   for ( j = 0; j < h4; j++ ) {

                       TxTypes[ y4 + j ][ x4 + i ] = DCT_DCT

                   }

               }

           }

       } else {

           eobMultisize = Min( Tx_Width_Log2[ txSz ], 5) +

                        Min( Tx_Height_Log2[ txSz ], 5) - 4

           eobCtx = (plane > 0) ? 2 : is_inter

           if ( eobMultisize == 0 ) {




    AV2 Specification                                                        Page 224 of 1169
         eob_pt_16                                                      S()

         eobPt = eob_pt_16 + 1

     } else if ( eobMultisize == 1 ) {

         eob_pt_32                                                      S()

         eobPt = eob_pt_32 + 1

     } else if ( eobMultisize == 2 ) {

         eob_pt_64                                                      S()

         eobPt = eob_pt_64 + 1

     } else if ( eobMultisize == 3 ) {

         eob_pt_128                                                     S()

         eobPt = eob_pt_128 + 1

     } else if ( eobMultisize == 4 ) {

         eob_pt_256                                                     S()

         if ( eob_pt_256 == 7 ) {

             eob_pt_256_extra                                          L(1)

             eobPt = 8 + eob_pt_256_extra

         } else {

             eobPt = eob_pt_256 + 1

         }

     } else if ( eobMultisize == 5 ) {

         eob_pt_512                                                     S()

         if ( eob_pt_512 == 7 ) {

             eob_pt_512_extra                                          L(2)

             eobPt = 8 + eob_pt_512_extra

         } else {

             eobPt = eob_pt_512 + 1

         }

     } else {

         eob_pt_1024                                                    S()

         if ( eob_pt_1024 == 7 ) {

             eob_pt_1024_extra                                         L(2)

             eobPt = 8 + eob_pt_1024_extra

         } else {

             eobPt = eob_pt_1024 + 1

         }

     }

     eob = ( eobPt < 2 ) ? eobPt : ( ( 1 << ( eobPt - 2 ) ) + 1 )

     if ( eobPt >= 3 ) {

         eob_extra                                                      S()

         if ( eob_extra ) {

             eob += 1 << (eobPt - 3)




AV2 Specification                                                   Page 225 of 1169
         }

         for ( i = eobPt - 4; i >= 0; i-- ) {

             eob_extra_bit                                                 L(1)

             if ( eob_extra_bit ) {

                 eob += 1 << i

             }

         }

     }

     if ( plane == 0 ) {

         transform_type( x4, y4, txSz, eob )

     } else if ( plane == 1 ) {

         if ( (is_inter || eob != 1) && is_cctx_allowed() ) {

             cctx_type                                                      S()

         } else {

             cctx_type = 0

         }

     }

     PlaneTxType = compute_tx_type( plane, txSz, x4, y4 )

     txClass = get_tx_class(PlaneTxType)

     scan = get_scan( txSz, txClass )

     useFsc = enable_fsc && PlaneTxType == IDTX && plane == 0 &&

             (fsc_mode || is_inter)

     if ( plane == 1 ) {

         EobU = eob

     }

     parityHiding = allow_parity_hiding && !Lossless && plane == 0 &&

                  PlaneTxType != IDTX

     numNz = 0

     sumAbs1 = 0

     isHidden = 0

     useTcq = allow_tcq && plane == 0 && !Lossless &&

             txClass == TX_CLASS_2D && !useFsc

     tcqState = 0

     hrLevelAvg = 0

     if ( useFsc ) {

         bob = segEob - eob

         eob = segEob

         for ( c = bob; c < eob; c++ ) {

             pos = scan[ c ]

             (row, col) = get_tx_row_col(pos, txSz)

             if ( c == bob ) {

                 coeff_base_bob



AV2 Specification                                                       Page 226 of 1169
                                                                        S()

               level = coeff_base_bob + 1

           } else {

               coeff_base_idtx                                          S()

               level = coeff_base_idtx

           }

           if ( level > NUM_BASE_LEVELS ) {

               coeff_br_idtx                                            S()

               level += coeff_br_idtx

           }

           Level[ row ][ col ] = level

       }

       for ( c = 0; c < eob; c += 1 ) {

           pos = scan[ c ]

           (row, col) = get_tx_row_col(pos, txSz)

           level = Level[ row ][ col ]

           if ( level != 0 ) {

               idtx_sign                                                S()

               sign = idtx_sign

           } else {

               sign = 0

           }

           (quant,hrLevelAvg) = read_quant(level, pos, 0,

                          NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1,

                          hrLevelAvg, 0 )

           if ( pos == 0 && quant > 0 ) {

               dcCategory = sign ? 1 : 2

           }

           culLevel = Min(4, culLevel + quant)

           if ( sign ) {

               quant = -quant

           }

           Quant[ pos ] = quant

           if ( level != 0 ) {

               QuantSign[ pos ] = sign ? -1 : 1

           }

       }

     } else {

       for ( c = eob - 1; c >= 0; c-- ) {

           pos = scan[ c ]

           (row, col) = get_tx_row_col(pos, txSz)

           isLf = get_lf_limits(row, col, txClass, plane)



AV2 Specification                                                   Page 227 of 1169
           if ( c == eob - 1 ) {

               coeff_base_eob                                               S()

               level = coeff_base_eob + 1

           } else {

               coeff_base                                                   S()

               level = coeff_base

           }

           baseLevels = isLf ? LF_NUM_BASE_LEVELS : NUM_BASE_LEVELS

           if ( level > baseLevels && !(isLf && plane > 0) ) {

               coeff_br                                                     S()

               level += coeff_br

           }

           if ( useTcq ) {

               tcqState = Tcq_Next_State[ tcqState ][ level & 1 ]

           }

           if ( parityHiding ) {

               if ( c > 0 ) {

                   sumAbs1 ^= Min( level,

                          NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1) & 1

                   if ( level != 0 ) {

                       numNz += 1

                       isHidden = numNz >= PHTHRESH

                   }

               }

           }

           Level[ row ][ col ] = level

       }

       tcqState = 0

       for ( c = eob - 1; c >= 0; c -= 1 ) {

           pos = scan[ c ]

           (row, col) = get_tx_row_col(pos, txSz)

           level = Level[ row ][ col ]

           if ( level != 0 || (isHidden && c == 0 && sumAbs1 > 0) ) {

               if ( row == 0 && col == 0 && plane == 0 ) {

                   dc_sign                                                  S()

                   sign = dc_sign

               } else if ( txClass == TX_CLASS_HORIZ && col == 0 &&

                        plane == 0 ) {

                   dc_sign_horz_vert                                        S()

                   sign = dc_sign_horz_vert

               } else if ( txClass == TX_CLASS_VERT && row == 0 &&




AV2 Specification                                                       Page 228 of 1169
                      plane == 0 ) {

                   dc_sign_horz_vert                                              S()

                   sign = dc_sign_horz_vert

               } else {

                   sign_bit                                                      L(1)

                   sign = sign_bit

               }

           } else {

               sign = 0

           }

           if ( get_lf_limits(row, col, txClass, plane) ) {

               maxLevel = ( plane == 0 ) ?

                       (LF_NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1) :

                       (LF_NUM_BASE_LEVELS + 1)

           } else {

               maxLevel = NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1

           }

           if ( isHidden && c == 0 ) {

               maxLevel = NUM_BASE_LEVELS + 1

           }

           (quant,hrLevelAvg) = read_quant( level, pos, isHidden, maxLevel,

                              hrLevelAvg, useTcq )

           if ( c == 0 && isHidden ) {

               quant = 2 * quant + sumAbs1

           }

           if ( pos == 0 && quant > 0 ) {

               dcCategory = sign ? 1 : 2

           }

           culLevel = Min(4, culLevel + quant)

           if ( !Lossless && useTcq ) {

               q0 = ((tcqState >> 1) & 1)

               tcqState = Tcq_Next_State[ tcqState ][ quant & 1 ]

               if ( quant > 0 ) {

                   quant = quant * 2 - q0

               }

           }

           if ( sign ) {

               quant = -quant

           }

           Quant[ pos ] = quant

       }




AV2 Specification                                                             Page 229 of 1169
         }

     }

     for ( i = 0; i < w4; i++ ) {

         AboveLevelContext[ plane ][ x4 + i ] = culLevel

         AboveDcContext[ plane ][ x4 + i ] = dcCategory

     }

     for ( i = 0; i < h4; i++ ) {

         LeftLevelContext[ plane ][ y4 + i ] = culLevel

         LeftDcContext[ plane ][ y4 + i ] = dcCategory

     }

     return eob

 }


where get_tx_row_col (which extracts the row and column for a position in raster order) is defined as:

 get_tx_row_col(pos, txSz) {
     adjTxSz = Adjusted_Tx_Size[ txSz ]
     bwl = Tx_Width_Log2[ adjTxSz ]
     row = pos >> bwl
     col = pos - (row << bwl)
     return (row, col)
 }


and get_lf_limits (which determines if this is a low frequency coefficient) is defined as:

 get_lf_limits(row, col, txClass, plane) {
     if ( txClass == TX_CLASS_2D ) {
         return plane == 0 ? ((row + col) < 4) : ((row + col) < 1)
     } else if ( txClass == TX_CLASS_HORIZ ) {
         return plane == 0 ? (col < 2) : (col < 1)
     } else {
         return plane == 0 ? (row < 2) : (row < 1)
     }
 }


and is_cctx_allowed is defined as:

 is_cctx_allowed( ) {                                                                        Descriptor

     is420 = SubsamplingX && SubsamplingY

     planeSz = get_plane_residual_size( ChromaMiSize, 1 )

     return enable_cctx &&

             !Lossless &&

             (is420 || Block_Width[planeSz] < 32 || Block_Height[planeSz] < 32)

 }


and Tcq_Next_State (which updates the TCQ state based on the current state and parity) is defined as:

 Tcq_Next_State[ 8 ][ 2 ] = {
     {0, 4},
     {4, 0},




AV2 Specification                                                                            Page 230 of 1169
                 {1, 5},
                 {5, 1},
                 {6, 2},
                 {2, 6},
                 {7, 3},
                 {3, 7}
     }


```

<a id="s-5-20-7-28"></a>

##### § 5.20.7.28 Read quantized coefficient syntax

```text
§   5.20.7.28. Read quantized coefficient syntax

     read_quant(level, pos, isHidden, maxLevel, hrLevelAvg, allowTcq ) {   Descriptor

         quant = level

         if ( quant >= maxLevel - allowTcq ) {

             lvlShift = (pos == 0 && isHidden) ? 1 : 0

             predLevel = hrLevelAvg >> lvlShift

             m = Clip3( 1, 6, GetMsb( predLevel ) )

             k = m + 1

             cMax = Min( m + 4, 6 )

             for ( q = 0 ; q < cMax; q++ ) {

                 q_length_bit                                                 L(1)

                 if ( q_length_bit ) {

                     break

                 }

             }

             if ( q == cMax ) {

                 length = -1

                 do {

                     length++

                     golomb_length_bit                                        L(1)

                 } while ( !golomb_length_bit )

                 length += k

                 xBase = (q << m) + (1 << length) - (1 << k)

             } else {

                 length = m

                 xBase = q << m

             }

             coeff_rem                                                      L(length)

             x = xBase + coeff_rem

             hrLevelAvg = ((x << lvlShift) + hrLevelAvg) >> 1

             quant += x << (allowTcq ? 1 : 0)

         }

         return (quant, hrLevelAvg)

     }




    AV2 Specification                                                      Page 231 of 1169
```

<a id="s-5-20-7-29"></a>

##### § 5.20.7.29 Compute transform type function

```text
§   5.20.7.29. Compute transform type function

     compute_tx_type( plane, txSz, blockX, blockY ) {
         if ( Lossless && plane == 0 && fsc_mode ) {
             return IDTX
         }
         if ( Lossless ) {
             if ( !is_inter ) {
                  fscMode = PlaneStart == 0 ? fsc_mode :
                                              FscModes[ ChromaMiRow ][ ChromaMiCol ]
                  if ( fscMode ) {
                      return IDTX
                  } else {
                      return DCT_DCT
                  }
             }
             if ( is_inter && txSz != TX_4X4 ) {
                  return IDTX
             }
             if ( plane > 0 ) {
                  x4 = Max( MiCol, blockX << SubsamplingX )
                  y4 = Max( MiRow, blockY << SubsamplingY )
                  if ( is_inter && LumaTxSizes[ y4 ][ x4 ] != TX_4X4 ) {
                      return IDTX
                  }
                  if ( !FrameIsIntra &&
                       MiRow == ChromaMiRow && MiCol == ChromaMiCol ) {
                      return TxTypes[ y4 ][ x4 ]
                  }
                  return TxTypes[ MiRow ][ MiCol ]
             }
         }
         txSet = get_tx_set( txSz, plane )
         if ( plane == 0 ) {
             return TxTypes[ blockY ][ blockX ]
         }
         if ( enable_chroma_dctonly ) {
             return DCT_DCT
         }
         if ( is_inter ) {
             x4 = Max( MiCol, blockX << SubsamplingX )
             y4 = Max( MiRow, blockY << SubsamplingY )
             txType = TxTypes[ y4 ][ x4 ]
             if ( !is_tx_type_in_set( txSet, txType ) ) {
                  return DCT_DCT
             }
             return txType
         }
         if ( is_directional_mode( UVMode ) ) {
             pAngle = Mode_To_Angle[ UVMode ] + AngleDeltaUV * ANGLE_STEP
             (mode, unusedAngle) = wide_angle_mapping( UVMode, Tx_Width[ txSz ],
                                                         Tx_Height[ txSz ], pAngle )
             txType = Mode_To_Txfm[ mode ]
         } else {
             txType = Mode_To_Txfm[ UVMode ]
         }
         if ( !is_tx_type_in_set( txSet, txType ) ) {
             return DCT_DCT
         }
         return txType
     }

     is_tx_type_in_set( txSet, txType ) {
         return is_inter ? Tx_Type_In_Set_Inter[ txSet ][ txType ] :
                           Tx_Type_In_Set_Intra[ txSet ][ txType ]
     }




    AV2 Specification                                                                  Page 232 of 1169
where the tables Tx_Type_In_Set_Inter and Tx_Type_In_Set_Intra are specified as follows:

 Tx_Type_In_Set_Intra[ TX_SET_TYPES_INTRA ][ TX_TYPES ] = {
   {
      1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
   },
   {
      1, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0
   },
   {
      1, 0, 1, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0
   },
   {
      1, 1, 0, 0, 1, 0, 0, 0, 0, 1, 1, 1, 1, 0, 1, 0
   },
   {
      1, 0, 1, 0, 0, 1, 0, 0, 0, 1, 1, 1, 0, 1, 0, 1
   },
   {
      1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
   },
   {
      1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
   }
 }

 Tx_Type_In_Set_Inter[ TX_SET_TYPES_INTER ][ TX_TYPES ] = {
   {
      1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
   },
   {
      1, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0
   },
   {
      1, 0, 1, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0
   },
   {
      1, 1, 0, 0, 1, 0, 0, 0, 0, 1, 1, 1, 1, 0, 1, 0
   },
   {
      1, 0, 1, 0, 0, 1, 0, 0, 0, 1, 1, 1, 0, 1, 0, 1
   },
   {
      1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1
   },
   {
      1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0
   },
   {
      1, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0
   },
   {
      1, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 0, 0, 0, 0
   }
 }


The function wide_angle_mapping is defined as:

 wide_angle_mapping(mode, w, h, pAngle) {
     if ((h == 2 * w && pAngle < WAIP_WH_RATIO_2_THRES) ||
         (h == 4 * w && pAngle < WAIP_WH_RATIO_4_THRES) ||
         (h == 8 * w && pAngle < WAIP_WH_RATIO_8_THRES) ||
         (h == 16 * w && pAngle < WAIP_WH_RATIO_16_THRES)) {
         return (D203_PRED,180 + pAngle)
     } else if ((w == 2 * h && pAngle > 270 - WAIP_WH_RATIO_2_THRES) ||
             (w == 4 * h && pAngle > 270 - WAIP_WH_RATIO_4_THRES) ||
             (w == 8 * h && pAngle > 270 - WAIP_WH_RATIO_8_THRES) ||



AV2 Specification                                                                          Page 233 of 1169
                   (w == 16 * h &&
                   pAngle > 270 - WAIP_WH_RATIO_16_THRES)) {
               return (D45_PRED, pAngle - 180)
           }
           return (mode,pAngle)
     }


```

<a id="s-5-20-7-30"></a>

##### § 5.20.7.30 Get scan function

```text
§   5.20.7.30. Get scan function

     get_scan( txSz, txClass ) {
         w = Min( Tx_Width[ txSz ], 32)
         h = Min( Tx_Height[ txSz ], 32)
         if ( txClass == TX_CLASS_VERT ) {
             c = 0
             for ( y = 0; y < h; y++ ) {
                  for ( x = 0; x < w; x++ ) {
                      out[ c ] = y * w + x
                      c += 1
                  }
             }
         } else if ( txClass == TX_CLASS_HORIZ ) {
             c = 0
             for ( x = 0; x < w; x++ ) {
                  for ( y = 0; y < h; y++ ) {
                      out[ c ] = y * w + x
                      c += 1
                  }
             }
         } else {
             x = 0
             y = 0
             for ( c = 0; c < w*h; c++ ) {
                  out[ c ] = y * w + x
                  x += 1
                  y -= 1
                  if ( y < 0 || x >= w ) {
                      x += 1
                      s = Min(x,h - 1 - y)
                      x -= s
                      y += s
                  }
             }
         }
         return out
     }


```

<a id="s-5-20-7-31"></a>

##### § 5.20.7.31 Is directional mode function

```text
§   5.20.7.31. Is directional mode function

     is_directional_mode( mode ) {
         if ( ( mode >= V_PRED ) && ( mode <= D67_PRED ) ) {
             return 1
         }
         return 0
     }


```

<a id="s-5-20-7-32"></a>

##### § 5.20.7.32 Read CFL alphas syntax

```text
§   5.20.7.32. Read CFL alphas syntax

     read_cfl_alphas() {                                       Descriptor

         if ( !enable_cfl_intra ) {

          cfl_mhccp = 1

         } else if ( is_mhccp_allowed() ) {

          cfl_mhccp                                                S()




    AV2 Specification                                          Page 234 of 1169
         } else {

             cfl_mhccp = 0

         }

         if ( cfl_mhccp ) {

             cfl_index = CFL_MULTI

         } else {

             cfl_index                                      S()

         }

         if ( cfl_index == CFL_MULTI ) {

             cfl_mh_dir                                     S()

         }

         if ( cfl_index != CFL_EXPLICIT ) {

             return

         }

         cfl_alpha_signs                                    S()

         signU = (cfl_alpha_signs + 1 ) / 3

         signV = (cfl_alpha_signs + 1 ) % 3

         if ( signU != CFL_SIGN_ZERO ) {

             cfl_alpha_u                                    S()

             CflAlphaU = 1 + cfl_alpha_u

             if ( signU == CFL_SIGN_NEG ) {

                 CflAlphaU = -CflAlphaU

             }

         } else {

             CflAlphaU = 0

         }

         if ( signV != CFL_SIGN_ZERO ) {

             cfl_alpha_v                                    S()

             CflAlphaV = 1 + cfl_alpha_v

             if ( signV == CFL_SIGN_NEG ) {

                 CflAlphaV = -CflAlphaV

             }

         } else {

             CflAlphaV = 0

         }

     }


```

<a id="s-5-20-8"></a>

#### § 5.20.8 Coding tools structures

```text
§   5.20.8. Coding tools structures

```

<a id="s-5-20-8-1"></a>

##### § 5.20.8.1 Palette mode info syntax

```text
§   5.20.8.1. Palette mode info syntax

     palette_mode_info( ) {                             Descriptor

         if ( PlaneStart == 0 && YMode == DC_PRED ) {



    AV2 Specification                                   Page 235 of 1169
         has_palette_y                                                                            S()

         if ( has_palette_y ) {

             palette_size_y_minus_2                                                               S()

             PaletteSizeY = palette_size_y_minus_2 + 2

             cacheN = get_palette_cache( )

             idx = 0

             for ( i = 0; i < cacheN && idx < PaletteSizeY; i++ ) {

                 use_palette_color_cache_y                                                        L(1)

                 if ( use_palette_color_cache_y ) {

                     palette_colors_y[ idx ] = PaletteCache[ i ]

                     idx++

                 }

             }

             if ( idx < PaletteSizeY ) {

                 palette_colors_y[ idx ]                                                      L(BitDepth)

                 idx++

             }

             if ( idx < PaletteSizeY ) {

                 minBits = BitDepth - 3

                 palette_num_extra_bits_y                                                         L(2)

                 paletteBits = minBits + palette_num_extra_bits_y

             }

             while ( idx < PaletteSizeY ) {

                 palette_delta_y                                                             L(paletteBits)

                 palette_delta_y++

                 palette_colors_y[ idx ] =

                      Clip1( palette_colors_y[ idx - 1 ] +

                             palette_delta_y )

                 range = ( 1 << BitDepth ) - palette_colors_y[ idx ] - 1

                 paletteBits = Min( paletteBits, CeilLog2( range ) )

                 idx++

             }

             sort( palette_colors_y, 0, PaletteSizeY - 1 )

         }

     }

 }


The function sort( arr, i1, i2 ) sorts a subarray of the array arr in-place into ascending order. The
subarray to be sorted is between indices i1 and i2 inclusive.




AV2 Specification                                                                             Page 236 of 1169
    The function get_palette_cache, which merges the above and left palettes to form a cache, is specified as
    follows:

     get_palette_cache( ) {
         r = MiRow
         c = MiCol
         aboveN = 0
         if ( ( r * MI_SIZE ) % 64 ) {
             aboveN = PaletteSizes[ r - 1 ][ c ]
         }
         leftN = 0
         if ( AvailL ) {
             leftN = PaletteSizes[ r ][ c - 1 ]
         }
         aboveIdx = 0
         leftIdx = 0
         n = 0
         while ( aboveIdx < aboveN || leftIdx < leftN ) {
             if ( aboveIdx < aboveN ) {
                  val = PaletteColors[ r - 1 ][ c ][ aboveIdx ]
                  aboveIdx++
                  PaletteCache[ n ] = val
                  n++
             }
             if ( leftIdx < leftN ) {
                  val = PaletteColors[ r ][ c - 1 ][ leftIdx ]
                  leftIdx++
                  PaletteCache[ n ] = val
                  n++
             }
         }
         return n
     }



      NOTE:       get_palette_cache interleaves the available palette colors from above and left together.


```

<a id="s-5-20-8-2"></a>

##### § 5.20.8.2 Transform type syntax

```text
§   5.20.8.2. Transform type syntax

     transform_type( x4, y4, txSz, eob ) {                                                            Descriptor

       set = get_tx_set( txSz, 0 )

       if ( fsc_mode ) {

         TxType = IDTX

       } else if ( !is_inter && eob==1 ) {

         TxType = DCT_DCT

       } else if ( Lossless && is_inter ) {

         if ( txSz == TX_4X4 ) {

             lossless_inter_tx_type                                                                      S()

             TxType = lossless_inter_tx_type ? IDTX : DCT_DCT

         } else {

             TxType = IDTX

         }

       } else if ( set > 0 &&

          !Lossless &&

          !( reduced_tx_set == 2 && is_inter == 0 )




    AV2 Specification                                                                                Page 237 of 1169
     ) {

     if ( set == TX_SET_WIDE_32 || set == TX_SET_HIGH_32 ) {

         is_long_side_dct                                                     S()

     } else {

         is_long_side_dct = 1

     }

     if ( is_inter ) {

         inter_tx_type                                                        S()

         if ( set == TX_SET_WIDE_64 || set == TX_SET_WIDE_32 ) {

             TxType = Tx_Type_Inv_Long[ is_long_side_dct ][ 0 ]

                       [ inter_tx_type ]

         } else if ( set == TX_SET_HIGH_64 || set == TX_SET_HIGH_32 ) {

             TxType = Tx_Type_Inv_Long[ is_long_side_dct ][ 1 ]

                       [ inter_tx_type ]

         } else if ( set == TX_SET_INTER_1 ) {

             inter_tx_type_offset                                             S()

             TxType = Tx_Type_Inter_Inv_Set1[ inter_tx_type * 8 +

                          inter_tx_type_offset ]

         } else if ( set == TX_SET_INTER_2 ) {

             inter_tx_type_offset                                             S()

             TxType = Tx_Type_Inter_Inv_Set2[ inter_tx_type * 8 +

                          inter_tx_type_offset ]

         } else if ( set == TX_SET_DCT_IDTX ) {

             TxType = Tx_Type_Inter_Inv_Set3[ inter_tx_type ]

         } else {

             TxType = Tx_Type_Inter_Inv_Set4[ inter_tx_type ]

         }

     } else {

         intra_tx_type                                                        S()

         if ( set == TX_SET_WIDE_64 || set == TX_SET_WIDE_32 ) {

             TxType = Tx_Type_Inv_Long[ is_long_side_dct ][ 0 ]

                       [ intra_tx_type ]

         } else if ( set == TX_SET_HIGH_64 || set == TX_SET_HIGH_32 ) {

             TxType = Tx_Type_Inv_Long[ is_long_side_dct ][ 1 ]

                       [ intra_tx_type ]

         } else {

             sizeInfo = Size_Class[ txSz ]

             intraDir = YMode

             if ( is_directional_mode( intraDir ) ) {

              pAngle = Mode_To_Angle[ intraDir ] +

                  AngleDeltaY * ANGLE_STEP +




AV2 Specification                                                         Page 238 of 1169
                        Mrl_Index_To_Delta[ MrlIndex ]

                     (intraDir,unusedAngle) = wide_angle_mapping( intraDir,

                                 Tx_Width[ txSz ],

                                 Tx_Height[ txSz ], pAngle)

                 }

                 TxType = Md_Idx_To_Type[ sizeInfo ][ intraDir ][ intra_tx_type ]

             }

         }

     } else {

         TxType = DCT_DCT

     }

     large = Tx_Width[ txSz ] >= 8 && Tx_Height[ txSz ] >= 8

     if ( !large ) {

         eobLim = IST_4X4_HEIGHT

     } else if ( txSz == TX_8X8 || TxType == ADST_ADST ) {

         eobLim = IST_8X8_HEIGHT_RED

     } else {

         eobLim = IST_8X8_HEIGHT

     }

     if ( (is_inter ?

             enable_inter_ist && eob > 3 && TxType == DCT_DCT &&

                     Tx_Width[ txSz ] >= 16 && Tx_Height[ txSz ] >= 16 :

             enable_intra_ist && eob != 1 ) &&

         !Lossless &&

         (TxType == ADST_ADST || TxType == DCT_DCT) &&

         YMode != PAETH_PRED &&

         eob <= eobLim ) {

         sec_tx_type                                                                    S()

         if ( sec_tx_type != 0 && !is_inter ) {

             most_probable_stx_set                                                      S()

         }

     } else {

         sec_tx_type = 0

     }

     for ( i = 0; i < ( Tx_Width[ txSz ] >> 2 ); i++ ) {

         for ( j = 0; j < ( Tx_Height[ txSz ] >> 2 ); j++ ) {

             TxTypes[ y4 + j ][ x4 + i ] = TxType

         }

     }

 }




AV2 Specification                                                                   Page 239 of 1169
    where the inversion tables used in the function are specified as follows:

     Tx_Type_Inter_Inv_Set1[ 16 ] = {
         IDTX, V_DCT, H_DCT, V_ADST, H_ADST, V_FLIPADST, H_FLIPADST,
         DCT_DCT, ADST_DCT, DCT_ADST, FLIPADST_DCT, DCT_FLIPADST, ADST_ADST,
         FLIPADST_FLIPADST, ADST_FLIPADST, FLIPADST_ADST
     }

     Tx_Type_Inter_Inv_Set2[ 12 ] = {
         IDTX, V_DCT, H_DCT, DCT_DCT, ADST_DCT, DCT_ADST, FLIPADST_DCT,
         DCT_FLIPADST, ADST_ADST, FLIPADST_FLIPADST, ADST_FLIPADST,
         FLIPADST_ADST
     }

     Tx_Type_Inter_Inv_Set3[ 2 ]   = { IDTX, DCT_DCT }

     Tx_Type_Inter_Inv_Set4[ 4 ]   = { DCT_DCT, V_DCT, H_DCT, IDTX }

     Tx_Type_Inv_Long[ 2 ][ 2 ][ 4 ] = {
         {
             { V_DCT, V_ADST, V_FLIPADST, IDTX },
             { H_DCT, H_ADST, H_FLIPADST, IDTX },
         },
         {
             { DCT_DCT, ADST_DCT, FLIPADST_DCT, H_DCT },
             { DCT_DCT, DCT_ADST, DCT_FLIPADST, V_DCT },
         }
     }


```

<a id="s-5-20-8-3"></a>

##### § 5.20.8.3 Get transform set function

```text
§   5.20.8.3. Get transform set function

     get_tx_set( txSz, plane ) {
         txSzSqr = Tx_Size_Sqr[ txSz ]
         txSzSqrUp = Tx_Size_Sqr_Up[ txSz ]
         if ( txSzSqrUp > TX_32X32 ) {
             if ( txSzSqr >= TX_32X32 ) {
                 return TX_SET_DCTONLY
             }
             return (Tx_Width[ txSz ] > Tx_Height[ txSz ]) ? TX_SET_WIDE_64 :
                                                         TX_SET_HIGH_64
         }
         if ( txSzSqrUp == TX_32X32 && txSzSqr != TX_32X32 ) {
             return (Tx_Width[ txSz ] > Tx_Height[ txSz ]) ? TX_SET_WIDE_32 :
                                                         TX_SET_HIGH_32
         }
         if (!is_inter && txSzSqrUp == TX_32X32) {
             return TX_SET_DCTONLY
         }
         reducedTxSet = plane == 0 ? reduced_tx_set : enable_chroma_dctonly
         if ( txSzSqrUp == TX_32X32 || reducedTxSet == 1 ) {
             return is_inter ? TX_SET_DCT_IDTX : TX_SET_INTRA_2
         } else if ( reducedTxSet == 2 ) {
             return TX_SET_DCT_IDTX
         } else if ( reducedTxSet == 3 ) {
             return is_inter ? TX_SET_DCT_IDTX_IDDCT : TX_SET_INTRA_2
         }
         if ( is_inter ) {
             return ( txSzSqr == TX_16X16 ) ? TX_SET_INTER_2 : TX_SET_INTER_1
         }
         return TX_SET_INTRA_1
     }


```

<a id="s-5-20-8-4"></a>

##### § 5.20.8.4 Palette tokens syntax

```text
§   5.20.8.4. Palette tokens syntax

     palette_tokens( ) {                                                        Descriptor




    AV2 Specification                                                           Page 240 of 1169
   blockHeight = Block_Height[ MiSize ]

   blockWidth = Block_Width[ MiSize ]

   onscreenHeight = Min( blockHeight, (MiRows - MiRow) * MI_SIZE )

   onscreenWidth = Min( blockWidth, (MiCols - MiCol) * MI_SIZE )

   if ( PlaneStart == 0 && PaletteSizeY ) {

     palette_direction = 0

     if ( blockWidth < 64 && blockHeight < 64 ) {

         palette_direction                                                      L(1)

     }

     prevIdentityRow = PALETTE_ROW_FLAG_CONTEXTS - 1

     if ( palette_direction ) {

         outerLim = onscreenWidth

         innerLim = onscreenHeight

     } else {

         innerLim = onscreenWidth

         outerLim = onscreenHeight

     }

     for ( i = 0; i < outerLim; i++ ) {

         identity_row_y                                                          S()

         for ( j = 0; j < innerLim; j++ ) {

          if ( palette_direction ) {

              c = i

              r = j

          } else {

              r = i

              c = j

          }

          if ( identity_row_y == 2 ) {

              ColorMapY[ r ][ c ] = palette_direction ?

               ColorMapY[ r ][ c - 1 ] : ColorMapY[ r - 1 ][ c ]

          } else if ( identity_row_y == 1 && j > 0 ) {

              ColorMapY[ r ][ c ] = palette_direction ?

               ColorMapY[ r - 1 ][ c ] : ColorMapY[ r ][ c - 1 ]

          } else if ( r == 0 && c == 0 ) {

              color_index_map_y                                            NS(PaletteSizeY
                                                                                  )

              ColorMapY[ 0 ][ 0 ] = color_index_map_y

          } else {

              get_palette_color_context( ColorMapY, r, c, PaletteSizeY )

              palette_color_idx_y                                                S()

              ColorMapY[ r ][ c ] = ColorOrder[ palette_color_idx_y ]

          }



AV2 Specification                                                           Page 241 of 1169
                 }

                 prevIdentityRow = identity_row_y

             }

             for ( i = 0; i < onscreenHeight; i++ ) {

                 for ( j = onscreenWidth; j < blockWidth; j++ ) {

                     ColorMapY[ i ][ j ] = ColorMapY[ i ][ onscreenWidth - 1 ]

                 }

             }

             for ( i = onscreenHeight; i < blockHeight; i++ ) {

                 for ( j = 0; j < blockWidth; j++ ) {

                     ColorMapY[ i ][ j ] = ColorMapY[ onscreenHeight - 1 ][ j ]

                 }

             }

         }

     }


```

<a id="s-5-20-8-5"></a>

##### § 5.20.8.5 Palette color context function

```text
§   5.20.8.5. Palette color context function

     get_palette_color_context( colorMap, r, c, n ) {
         for ( i = 0; i < PALETTE_COLORS; i++ ) {
             scores[ i ] = 0
             ColorOrder[ i ] = i
         }
         if ( c > 0 ) {
             neighbor = colorMap[ r ][ c - 1 ]
             scores[ neighbor ] += 2
         }
         if ( ( r > 0 ) && ( c > 0 ) ) {
             neighbor = colorMap[ r - 1 ][ c - 1 ]
             scores[ neighbor ] += 1
         }
         if ( r > 0 ) {
             neighbor = colorMap[ r - 1 ][ c ]
             scores[ neighbor ] += 2
         }
         for ( i = 0; i < PALETTE_NUM_NEIGHBORS; i++ ) {
             maxScore = scores[ i ]
             maxIdx = i
             for ( j = i + 1; j < n; j++ ) {
                 if ( scores[ j ] > maxScore ) {
                     maxScore = scores[ j ]
                     maxIdx = j
                 } else if ( scores[ j ] > 0 && scores[ j ] == maxScore &&
                             c > 0 && j == colorMap[ r ][ c - 1 ] ) {
                     maxScore = scores[ j ]
                     maxIdx = j
                 }
             }
             if ( maxIdx != i ) {
                 maxScore = scores[ maxIdx ]
                 maxColorOrder = ColorOrder[ maxIdx ]
                 for ( k = maxIdx; k > i; k-- ) {
                     scores[ k ] = scores[ k - 1 ]
                     ColorOrder[ k ] = ColorOrder[ k - 1 ]
                 }
                 scores[ i ] = maxScore
                 ColorOrder[ i ] = maxColorOrder
             }
         }



    AV2 Specification                                                             Page 242 of 1169
          ColorContextHash = 0
          for ( i = 0; i < PALETTE_NUM_NEIGHBORS; i++ ) {
              ColorContextHash += scores[ i ] * Palette_Color_Hash_Multipliers[ i ]
          }
     }



      NOTE: The reference software has an alternative implementation that may be better suited for
      hardware implementations.

```

<a id="s-5-20-9"></a>

#### § 5.20.9 Helper functions

```text
§   5.20.9. Helper functions

```

<a id="s-5-20-9-1"></a>

##### § 5.20.9.1 Is inside function

```text
§   5.20.9.1. Is inside function

    is_inside determines whether a candidate position is inside the current tile.

     is_inside( candidateR, candidateC ) {
         return ( candidateC >= MiColStart &&
                  candidateC < MiColEnd &&
                  candidateR >= MiRowStart &&
                  candidateR < MiRowEnd )
     }


```

<a id="s-5-20-9-2"></a>

##### § 5.20.9.2 Is inside frame function

```text
§   5.20.9.2. Is inside frame function

    is_inside_frame determines whether a candidate position is inside the current frame.

     is_inside_frame( candidateR, candidateC ) {
         return ( candidateC >= 0 &&
                 candidateC < MiCols &&
                 candidateR >= 0 &&
                 candidateR < MiRows )
     }


```

<a id="s-5-20-9-3"></a>

##### § 5.20.9.3 Is inside filter region function

```text
§   5.20.9.3. Is inside filter region function

    is_inside_filter_region determines whether a candidate position is inside the region that is being used for
    CDEF and restoration filtering.

     is_inside_filter_region( candidateR, candidateC ) {
         if ( disable_loopfilters_across_tiles ) {
             return is_inside( candidateR, candidateC )
         } else {
             return is_inside_frame( candidateR, candidateC )
         }
     }


```

<a id="s-5-20-9-4"></a>

##### § 5.20.9.4 Clamp MV row function

```text
§   5.20.9.4. Clamp MV row function

     clamp_mv_row( mvec ) {
         bh4 = Num_4x4_Blocks_High[ MiSize ]
         low = -(MiRow + bh4) * MI_SIZE * 8 - MV_BORDER
         high = (MiRows - MiRow) * MI_SIZE * 8 + MV_BORDER
         return Clip3( low, high, mvec )
     }




    AV2 Specification                                                                             Page 243 of 1169
```

<a id="s-5-20-9-5"></a>

##### § 5.20.9.5 Clamp MV col function

```text
§   5.20.9.5. Clamp MV col function

     clamp_mv_col( mvec ) {
         bw4 = Num_4x4_Blocks_Wide[ MiSize ]
         low = -(MiCol + bw4) * MI_SIZE * 8 - MV_BORDER
         high = (MiCols - MiCol) * MI_SIZE * 8 + MV_BORDER
         return Clip3( low, high, mvec )
     }


```

<a id="s-5-20-9-6"></a>

##### § 5.20.9.6 Clear CDEF function

```text
§   5.20.9.6. Clear CDEF function

     clear_cdef( r, c ) {
         cdef_idx[ r ][ c ] = -1
         num4x4 = Num_4x4_Blocks_Wide[ SbSize ]
         cdefSize4 = Num_4x4_Blocks_Wide[ BLOCK_64X64 ]
         num64x64 = num4x4 / cdefSize4
         for ( i = 0; i < num64x64; i++ ) {
             for ( j = 0; j < num64x64; j++ ) {
                 cdef_idx[ r + i * cdefSize4 ][ c + j * cdefSize4 ] = -1
             }
         }
     }


```

<a id="s-5-20-10"></a>

#### § 5.20.10 Filtering structures

```text
§   5.20.10. Filtering structures

```

<a id="s-5-20-10-1"></a>

##### § 5.20.10.1 Read CDEF syntax

```text
§   5.20.10.1. Read CDEF syntax

     read_cdef( ) {                                                        Descriptor

       if ( (skip_flag && !cdef_on_skip_txfm_frame_enable) ||

           !cdef_frame_enable ) {

           return

       }

       cdefSize4 = Num_4x4_Blocks_Wide[ BLOCK_64X64 ]

       cdefMask4 = ~(cdefSize4 - 1)

       r = MiRow & cdefMask4

       c = MiCol & cdefMask4

       if ( cdef_idx[ r ][ c ] == -1 ) {

           if ( CdefStrengths == 1 ) {

               cdef_idx[ r ][ c ] = 0

           } else {

               cdef_index0                                                     S()

               if ( cdef_index0 ) {

                   cdef_idx[ r ][ c ] = 0

               } else if ( CdefStrengths == 2 ) {

                   cdef_idx[ r ][ c ] = 1

               } else {

                   cdef_index_minus_1                                          S()

                   cdef_idx[ r ][ c ] = cdef_index_minus_1 + 1

               }

           }




    AV2 Specification                                                      Page 244 of 1169
             w4 = Num_4x4_Blocks_Wide[ MiSize ]

             h4 = Num_4x4_Blocks_High[ MiSize ]

             for ( i = r; i < r + h4 ; i += cdefSize4 ) {

                 for ( j = c; j < c + w4 ; j += cdefSize4 ) {

                     cdef_idx[ i ][ j ] = cdef_idx[ r ][ c ]

                 }

             }

         }

     }


```

<a id="s-5-20-10-2"></a>

##### § 5.20.10.2 Read CCSO syntax

```text
§   5.20.10.2. Read CCSO syntax

     read_ccso( ) {                                                                  Descriptor

         if ( !enable_ccso ) {

             return

         }

         shiftRow = CcsoLumaSizeLog2 - MI_SIZE_LOG2

         shiftCol = CcsoLumaSizeLog2 - MI_SIZE_LOG2

         blkH4 = 1 << shiftRow

         blkW4 = 1 << shiftCol

         if ( (MiRow & (blkH4 - 1)) || (MiCol & (blkW4 - 1)) ) {

             return

         }

         for ( plane = 0; plane < NumPlanes; plane++ ) {

             if ( ccso_planes[ plane ] ) {

                 if ( !sb_reuse_ccso[ plane ] ) {

                     ccso_blk                                                            S()

                     CcsoBlks[ plane ][ MiRow >> shiftRow ][ MiCol >> shiftCol ] =

                      ccso_blk

                 }

             }

         }

     }


```

<a id="s-5-20-10-3"></a>

##### § 5.20.10.3 Read GDF syntax

```text
§   5.20.10.3. Read GDF syntax

     read_gdf( ) {                                                                   Descriptor

         if ( !gdf_frame_enable || !gdf_per_block ) {

             return

         }

         sbSize4 = Num_4x4_Blocks_Wide[ SbSize ]

         if ( MiRow % sbSize4 != 0 || MiCol % sbSize4 != 0 ) {

             return




    AV2 Specification                                                                Page 245 of 1169
         }

         sbRow = MiRow / sbSize4

         sbCol = MiCol / sbSize4

         sbPerGdf = GdfBlkSize / Block_Width[ SbSize ]

         if ( sbCol % sbPerGdf != 0 ) {

             return

         }

         if ( sbRow % sbPerGdf != 0 ) {

             return

         }

         use_gdf                                                                        S()

         GdfBlks[ sbRow / sbPerGdf ][ sbCol / sbPerGdf ] = use_gdf

     }


```

<a id="s-5-20-10-4"></a>

##### § 5.20.10.4 Read loop restoration syntax

```text
§   5.20.10.4. Read loop restoration syntax

     read_lr( row, col, bSize ) {                                                   Descriptor

         w = Num_4x4_Blocks_Wide[ bSize ]

         h = Num_4x4_Blocks_High[ bSize ]

         for ( plane = PlaneStart; plane < PlaneEnd; plane++ ) {

             if ( FrameRestorationType[ plane ] != RESTORE_NONE ) {

              subX = (plane == 0) ? 0 : SubsamplingX

              subY = (plane == 0) ? 0 : SubsamplingY

              unitSize = LoopRestorationSize[ plane ]

              miCols = MiColEnd - MiColStart

              miRows = MiRowEnd - MiRowStart

              lrRowOffset = (MiRowStart * MI_SIZE >> subY) / unitSize

              lrColOffset = (MiColStart * MI_SIZE >> subX) / unitSize

              c = col - MiColStart

              r = row - MiRowStart

              unitRows = count_units_in_frame(unitSize, miRows * MI_SIZE >> subY)

              unitCols = count_units_in_frame(unitSize, miCols * MI_SIZE >> subX)

              unitRowStart = ( r * ( MI_SIZE >> subY) +

                         unitSize - 1 ) / unitSize

              unitRowEnd = Min( unitRows, ( (r + h) * ( MI_SIZE >> subY) +

                         unitSize - 1 ) / unitSize)

              unitColStart = ( c * (MI_SIZE >> subX) + unitSize - 1 ) / unitSize

              unitColEnd = Min( unitCols,

                ( (c + w) * (MI_SIZE >> subX) + unitSize - 1 ) / unitSize)

              for ( unitRow = unitRowStart; unitRow < unitRowEnd; unitRow++ ) {

                for (unitCol = unitColStart; unitCol < unitColEnd; unitCol++) {

                  read_lr_unit(plane, unitRow + lrRowOffset,




    AV2 Specification                                                               Page 246 of 1169
                               unitCol + lrColOffset)

                     }

                 }

             }

         }

     }


    where count_units_in_frame is a function specified as:

     count_units_in_frame(unitSize, frameSize) {
         return Max((frameSize + (unitSize >> 1)) / unitSize, 1)
     }


```

<a id="s-5-20-10-5"></a>

##### § 5.20.10.5 Read loop restoration unit syntax

```text
§   5.20.10.5. Read loop restoration unit syntax

     read_lr_unit(plane, unitRow, unitCol) {                                          Descriptor

         if ( FrameRestorationType[ plane ] == RESTORE_WIENER_NONSEP ) {

             use_wiener_ns                                                                S()

             restorationType = use_wiener_ns ? RESTORE_WIENER_NONSEP : RESTORE_NONE

         } else if ( FrameRestorationType[ plane ] == RESTORE_PC_WIENER ) {

             use_pc_wiener                                                                S()

             restorationType = use_pc_wiener ? RESTORE_PC_WIENER : RESTORE_NONE

         } else {

             restorationType = RESTORE_SWITCHABLE_TYPES - 1

             for ( tool = 0; tool < RESTORE_SWITCHABLE_TYPES - 1; tool++ ) {

                 flex_restoration_type                                                    S()

                 if ( flex_restoration_type ) {

                     restorationType = tool

                     break

                 }

             }

         }

         LrType[ plane ][ unitRow ][ unitCol ] = restorationType

         if ( restorationType == RESTORE_WIENER_NONSEP ) {

             read_wienerns_filter( plane, unitRow, unitCol, 0 )

         }

     }


```

<a id="s-5-20-10-6"></a>

##### § 5.20.10.6 Read Wiener NS syntax

```text
§   5.20.10.6. Read Wiener NS syntax

     read_wienerns_filter( plane, unitRow, unitCol, readFrameFilters ) {              Descriptor

         numClasses = 1

         if ( frame_filters_on[ plane ] ) {

             if ( !readFrameFilters ) {




    AV2 Specification                                                                 Page 247 of 1169
         return

     }

     (numClasses, numRefFilters, _, _, _) = search_frame_filters( plane, -1 )

     nopcw = lr_tools_disable[ 0 ][ RESTORE_PC_WIENER ]

     groupCounts[ 0 ] = numClasses

     groupCounts[ 1 ] = numRefFilters

     groupCounts[ 2 ] = (plane > 0 || nopcw) ?

                       0 : 64 - numClasses - numRefFilters

     for ( i = 0; i < 3; i++ ) {

         groupHits[ i ] = 0

     }

     groupBase[ 0 ] = 0

     for ( i = 1; i < 3; i++ ) {

         groupBase[ i ] = groupBase[ i - 1 ]        + groupCounts[ i - 1 ]

     }

     for ( c = 0 ; c < numClasses; c++ ) {

         groupCounts[ 0 ] = c + 1

         if ( c == 0 ) {

             predGroup = (groupCounts[ 1 ] > 2) ?

                        1 : predict_group( groupCounts )

         } else {

             predGroup = predict_group( groupHits )

         }

         numZeros = 0

         altGroup = 0

         for ( i = 0; i < 3; i++ ) {

             if ( i != predGroup ) {

                 if ( groupCounts[ i ] == 0 ) {

                     numZeros += 1

                 } else {

                     altGroup = i

                 }

             }

         }

         if ( numZeros == 2 ) {

             use_alt_group = 0

         } else {

             use_alt_group                                                         f(1)

         }

         if ( use_alt_group ) {

             if ( numZeros == 1 ) {




AV2 Specification                                                               Page 248 of 1169
                   group = altGroup

               } else {

                   group_bit                                                       f(1)

                   group = predGroup <= group_bit ? group_bit + 1 : group_bit

               }

           } else {

               group = predGroup

           }

           n = groupCounts[ group ]

           ref = groupBase[ group ] + (n >> 1)

           if ( n == 1 ) {

               matchIndices[ c ] = groupBase[ group ]

           } else {

               matchIndices[ c ] = decode_signed_subexp_with_ref(

                           groupBase[ group ],

                           groupBase[ group ] + n, ref, 4)

           }

           groupHits[ group ]++

       }

   }

   for ( c = 0 ; c < numClasses ; c++ ) {

       if ( readFrameFilters ) {

           merged_param                                                            f(1)

       } else {

           merged_param                                                            L(1)

       }

       merged[ c ] = merged_param

       if ( readFrameFilters ) {

           refBank[ c ] = 0

       } else {

           for ( k = 0; k < WienerNsBankSize[ plane ][ c ] - 1; k++ ) {

               use_bank                                                            L(1)

               if (use_bank) {

                   break

               }

           }

           refBank[ c ] = (WienerNsPtr[ plane ][ c ] - k + LR_BANK_SIZE) %

                     LR_BANK_SIZE

       }

   }

   for ( c = 0 ; c < numClasses ; c++ ) {




AV2 Specification                                                               Page 249 of 1169
     if ( frame_filters_on[ plane ] ) {

         fill_first_slot_of_bank_with_filter_match( c, plane,

                               matchIndices[ c ] )

     }

     nCoeffs = plane > 0 ? WIENER_NS_CHROMA_COEFFS :

                     WIENER_NS_LUMA_COEFFS

     if ( merged[ c ] ) {

         if ( WienerNsBankSize[ plane ][ c ] == 0 ) {

             WienerNsBankSize[ plane ][ c ] = 1

         }

     } else {

         if ( WienerNsBankSize[ plane ][ c ] < LR_BANK_SIZE ) {

             WienerNsPtr[ plane ][ c ] = WienerNsBankSize[ plane ][ c ]

             WienerNsBankSize[ plane ][ c ] += 1

         } else {

             WienerNsPtr[ plane ][ c ] = (WienerNsPtr[ plane ][ c ] + 1) %

                           LR_BANK_SIZE

         }

         numSubsets = plane == 0 ? 4 : 3

         for ( subset = 0; subset < numSubsets - 1; subset++ ) {

             if ( readFrameFilters ) {

                 wiener_ns_length                                               f(1)

             } else {

                 wiener_ns_length                                                S()

             }

             if ( wiener_ns_length == 0 ) {

                 break

             }

         }

         if ( plane > 0 && subset > 0 ) {

             if ( readFrameFilters ) {

                 wiener_ns_uv_sym                                               f(1)

             } else {

                 wiener_ns_uv_sym                                                S()

             }

         } else {

             wiener_ns_uv_sym = 0

         }

     }

     for ( j = 0; j < nCoeffs; j++ ) {

         min = Wiener_Ns_Taps_Min[ plane!=0 ][ j ]




AV2 Specification                                                            Page 250 of 1169
             k = Wiener_Ns_Taps_K[ plane!=0 ][ j ]

             v = RefLrWienerNs[ plane ][ c ][ refBank[ c ] ][ j ]

             if ( !merged[ c ] ) {

                 if ( Wiener_Ns_Taps_Present[ plane!=0 ][ subset ][ j ] ) {

                     if ( readFrameFilters ) {

                         v = decode_signed_subexp_with_ref( min, min + (1 << k),

                                       v, k - 3 )

                     } else {

                         v = decode_signed_4part( min, k, v   )

                     }

                 } else {

                     v = 0

                 }

             }

             if ( readFrameFilters ) {

                 FrameLrWienerNs[ plane ][ c ][ j ] = v

                 if ( !merged[ c ]      && plane > 0 &&

                     j >= WIENER_NS_SHORT_COEFFS && wiener_ns_uv_sym ) {

                     FrameLrWienerNs[ plane ][ c ][ j + 1 ] = v

                     j++

                 }

             } else {

                 LrWienerNs[ plane ][ unitRow ][ unitCol ][ j ] = v

                 if ( !merged[ c ] ) {

                     RefLrWienerNs[ plane ][ c ]

                             [ WienerNsPtr[ plane ][ c ] ][ j ] = v

                 }

                 if ( !merged[ c ] && plane > 0 &&

                     j >= WIENER_NS_SHORT_COEFFS && wiener_ns_uv_sym ) {

                     LrWienerNs[ plane ][ unitRow ][ unitCol ][ j + 1 ] = v

                     RefLrWienerNs[ plane ][ c ]

                             [ WienerNsPtr[ plane ][ c ] ][ j + 1 ] = v

                     j++

                 }

             }

         }

     }

 }




AV2 Specification                                                                  Page 251 of 1169
where decode_signed_4part is a function defined as follows:

 decode_signed_4part(low, k, r) {
     rOffset = r - low
     xOffset = decode_unsigned_4part(k, rOffset)
     x = xOffset + low
     return x
 }

 decode_unsigned_4part(k, r) {
     mx = 1 << k
     v = decode_4part( 6 - k )
     if ((r << 1) <= mx) {
         offset = inverse_recenter(r, v)
     } else {
         offset = mx - 1 - inverse_recenter(mx - 1 - r, v)
     }
     return offset
 }

 decode_4part(num) {
     S() wiener_ns_base;
     bits = 2 - num + Max(1, wiener_ns_base)
     offset = wiener_ns_base == 0 ? 0 : (1 << bits)
     L(bits) wiener_ns_rem;
     return offset + wiener_ns_rem
 }


The function fill_first_slot_of_bank_with_filter_match is specified as:

 fill_first_slot_of_bank_with_filter_match( c, plane, m ) {
     WienerNsPtr[ plane ][ c ] = 0
     WienerNsBankSize[ plane ][ c ] = 1
     (numClasses, numRefFilters, matchIdx, matchCls, matchPlane) =
         search_frame_filters( plane, m )
     for( j = 0; j < ( (plane > 0) ? WIENER_NS_CHROMA_COEFFS :
                                     WIENER_NS_LUMA_COEFFS ); j++ ) {
         if ( m == 0 ) {
             v = 0
         } else if ( m < numClasses ) {
             oldCls = m - 1
             v = FrameLrWienerNs[ plane ][ oldCls ][ j ]
         } else if ( m < numClasses + numRefFilters ) {
             v = RefFrameLrWienerNs[ matchIdx ][ matchPlane ][ matchCls ][ j ]
         } else {
             v = get_translated_pc_wiener(m - NumFilterClasses - numRefFilters,j)
         }
         RefLrWienerNs[ plane ][ c ][ 0 ][ j ] = v
     }
 }


The function search_frame_filters is specified as:

 search_frame_filters( plane, target ) {
     nopcw = lr_tools_disable[ 0 ][ RESTORE_PC_WIENER ]
     minPcWiener = (plane > 0 || nopcw) ? 0 : 16
     numClasses = (plane == 0) ? NumFilterClasses : 1
     maxRefFilters = (nopcw ? 16 : 64) - numClasses - minPcWiener
     numRefFilters = 0
     numCheckPlanes = plane > 0 ? 2 : 1
     matchIdx = 0
     matchCls = 0
     matchPlane = plane
     for ( ref = 0; ref < NumTotalRefs; ref++ ) {
         if ( FrameType != SWITCH_FRAME && OrderHints[ref] != RESTRICTED_OH ) {



AV2 Specification                                                                   Page 252 of 1169
                idx = ref_frame_idx[ ref ]
                for ( check = 0; check < numCheckPlanes; check++ ) {
                    if ( check == 0 ) {
                        checkPlane = plane
                    } else {
                        checkPlane = plane == 1 ? 2 : 1
                    }
                    if ( RefFrameFiltersOn[ idx ][ checkPlane ] ) {
                        numRefClasses = (plane == 0) ?
                                             RefNumFilterClasses[ idx ] : 1
                        for ( i = 0; i < numRefClasses; i++ ) {
                             if ( numRefFilters < maxRefFilters ) {
                                 if ( numRefFilters + numClasses == target ) {
                                     matchIdx = idx
                                     matchCls = i
                                     matchPlane = checkPlane
                                 }
                                 numRefFilters += 1
                             }
                        }
                    }
                }
          }
      }
      return (numClasses, numRefFilters, matchIdx, matchCls, matchPlane)
 }


The function get_translated_pc_wiener (which converts a pixel classified filter into a Wiener filter) is
specified as:

 get_translated_pc_wiener( m, j ) {
     if ( j >= 12 ) {
             return 0
     }
     filt = Shuffled_Index[ m ]
     coeff = Round2Signed( Pc_Wiener_Filters[ 0 ][ filt ][ j ],
                           PC_WIENER_PREC_BITS - WIENER_NS_PREC_BITS )
     min = Wiener_Ns_Taps_Min[ 0 ][ j ]
     max = min + ( 1 << Wiener_Ns_Taps_K[ 0 ][ j ] ) - 1
     return Clip3(min, max, coeff)
 }


where Shuffled_Index is defined as:

 Shuffled_Index[ 64 ] = {
     16, 7, 58, 21, 12, 61, 26, 38, 18, 30, 50,
     45, 23, 49, 43, 62, 42, 54, 27, 36, 17, 44,
     32, 34, 4, 24, 52, 31, 37, 11, 33, 19, 35,
     6, 22, 53, 63, 25, 41, 47, 1, 59, 0, 28,
     40, 55, 48, 8, 5, 51, 9, 46, 56, 60, 15,
     2, 13, 14, 57, 29, 3, 20, 39, 10
 }


The function predict_group (which finds which group has the highest count) is specified as:


 predict_group( counts ) {
     pred = 0
     for ( i = 1; i <= 2; i++ ) {
         if ( counts[ i ] > counts[ pred ] ) {
              pred = i
         }




AV2 Specification                                                                              Page 253 of 1169
      }
      return pred
 }


The table Wiener_Ns_Taps_Present (which specifies which filter taps are present) is specified as:

 Wiener_Ns_Taps_Present[ 2 ][ 4 ][ WIENER_NS_CHROMA_COEFFS ] = {
     {
         { 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0},
         { 1, 1, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0},
         { 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0},
         { 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0}
     },
     {
         { 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0},
         { 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0},
         { 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1},
         { 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0}
     }
 }



                                                                                 ↑ Back to Table of Contents




AV2 Specification                                                                            Page 254 of 1169
```
