# AV2 v1.0.0 — § 7. Decoding process

<!-- Verbatim mirror of the AOM AV2 v1.0.0 specification (© Alliance for Open Media). The PDF is normative; this is a faithful `pdftotext -layout` copy. See [./README.md](./README.md) and [./index.md](./index.md). Do not hand-edit: regenerate via scripts/spec/regenerate-av2-spec.sh. -->

<a id="s-7"></a>

## § 7 Decoding process

```text
§   7. Decoding process
```

<a id="s-7-1"></a>

### § 7.1 General decoding process

```text
§   7.1. General decoding process
    When film_grain_params_present is equal to 0, decoders shall produce output frames that are identical in
    all respects and have the same output order as those produced by the decoding process specified herein.

    When film_grain_params_present is equal to 1, a decoder shall implement a film grain synthesis process
    that modifies the output arrays OutY, OutU, OutV. The reference film grain synthesis process is described
    in § 7.21.7 Film grain synthesis process.

    When film_grain_params_present is equal to 1, a conformant decoder shall satisfy at least one of the
    following two options:

     1. A conformant decoder shall produce output frames that are identical in all respects and have the
        same output order as those produced by the decoding process specified herein including applying the
        exact film grain synthesis process as specified in § 7.21.7 Film grain synthesis process.
     2. A conformant decoder shall produce intermediate frames that are identical in all respects and have
        the same order as the frames produced by the process specified in § 7.21.2 Intermediate output
        preparation process. In addition to that, a conformant decoder shall produce output frames that are
        in the same order and do not have perceptually significant differences with the frames produced by
        the reference film grain synthesis process specified in § 7.21.7 Film grain synthesis process when
        applied to the input frames of the film grain synthesis process with the film grain parameters
        signaled for these frames. The decoder may also include optional processing steps which are applied
        to the intermediate frames produced by the process specified in § 7.21.2 Intermediate output
        preparation process and before the film grain synthesis process, resulting in the input frames of the
        film grain synthesis process. Such optional processing steps are beyond the scope of this
        specification. Otherwise, the intermediate frames are the input frames of the film grain synthesis
        process. The definition of "perceptually significant differences" is beyond the scope of this
        specification and may be specified, for example, by a service provider as part of their accreditation
        program. The film grain synthesis process applied by a conformant decoder should be feature
        complete with regards to the reference film grain synthesis process of § 7.21.7 Film grain synthesis
        process including scaling strength of the film grain as a function of intensity according to the
        signaled parameters, same maximum AR lag, and similar modeling of correlation between luma and
        chroma and smoothing of transitions between blocks of grain when applicable.


      NOTE: To ensure conformance, decoder manufacturers are advised to implement the film grain
      synthesis process as specified in § 7.21.7 Film grain synthesis process. One reason to choose the
      second conformance option is implementation of optional processing steps between the output of
      § 7.21.2 Intermediate output preparation process and the film grain synthesis process, in which case
      there may be minor differences in the output with the reference film grain synthesis process of
      § 7.21.7 Film grain synthesis process. Examples of these optional processing steps are algorithms
      improving output frame quality, such as de-banding filtering and coding artefacts removal.


      NOTE: Some applications, such as transcoding from AV2 to AV2, may use intermediate output
      frames of § 7.21.2 Intermediate output preparation process for transcoding. In such cases, the
      original film grain synthesis information may be adapted and inserted in the transcoded bitstream.



    AV2 Specification                                                                           Page 375 of 1169
    The input to this process is a sequence of open bitstream units (OBUs).

    The output from this process is a sequence of decoded frames.

    For each OBU in turn the syntax elements are extracted as specified in § 5.2 OBU syntax.

    After all OBUs have been decoded, the flush implicit output frames process specified in § 7.21.5 Flush
    implicit output frames process is invoked with 0 as input (this outputs any remaining frames).

    The syntax tables include function calls indicating when the remaining decode processes are triggered.

    A singlestream can be decoded directly via this decoding process.

    Each stream within a multistream can be decoded by decoding the corresponding extracted OBUs.


      NOTE: Although the decoding process and semantics are defined for a single stream, a decoder
      implementation may choose to decode multiple extended layers at the same time as long as the
      output is equivalent.


    The corresponding OBUs can be extracted from a multistream for stream x by concatenating all OBUs
    that satisfy either of the following conditions:

      • obu_xlayer_id equal to GLOBAL_XLAYER_ID and obu_type is not equal to OBU_MSDO.
      • OBUs with obu_xlayer_id corresponding to the chosen stream.


      NOTE: In a coded video multistream sequence that contains an OBU with obu_type equal to
      OBU_MSDO, the obu_xlayer_id that corresponds to stream x is given by sub_xlayer_id[ x ]. Otherwise,
      a global LCR must be present and activated, and the obu_xlayer_id that corresponds to stream x is
      given by the x-th non-zero bit in lcr_xlayer_map. (For example, if lcr_xlayer_map is equal to 8, which
      is equal to 1 << 3, then stream 0 would correspond to choosing OBUs with obu_xlayer_id equal to 3.)


```

<a id="s-7-2"></a>

### § 7.2 Decode frame wrapup process

```text
§   7.2. Decode frame wrapup process
    This process is triggered by a call to decode_frame_wrapup from within the syntax tables.

    At this stage, all the tile level decode has been done, and this process performs any frame level decode
    that is required.

    The frame level filters are applied as follows:

      • If TipFrameMode is equal to TIP_FRAME_AS_OUTPUT, the deblocking filter is applied by the
        following ordered steps:

          1. If apply_deblocking_filter_tip is equal to 1, the deblocking filter for TIP process specified in § 7.16
             Deblocking filter for TIP process is invoked.
          2. LrFrame is set equal to CurrFrame.
      • Otherwise, if bru_inactive is equal to 1, the frame is updated by the following ordered steps:

          1. LrFrame is set equal to a copy of FrameStore[ref_frame_idx[bru_ref]].




    AV2 Specification                                                                                 Page 376 of 1169
      2. MfRefFrames[ y8 ][ x8 ][ list ] is set equal to bru_ref for y8 = 0..(MiRows>>1)-1, x8 = 0..
         (MiCols>>1)-1, for list=0..1.
      3. MfMvs[ y8 ][ x8 ][ list ][ comp ] is set equal to 0 for y8 = 0..(MiRows>>1)-1, x8 = 0..
         (MiCols>>1)-1, for list=0..1, for comp=0..1.
  • Otherwise, if ShowExistingFrame is equal to 0, the process first performs any post processing
    filtering by the following ordered steps:

      1. If apply_deblocking_filter[ 0 ] is not equal to 0 or apply_deblocking_filter[ 1 ] is not equal to 0, the
         deblocking filter process specified in § 7.17 Deblocking filter process is invoked (this process
         modifies the contents of CurrFrame).
      2. The CDEF process specified in § 7.18 CDEF process is invoked (this process takes CurrFrame and
         produces CdefFrame).
      3. The CCSO process specified in § 7.19 CCSO process is invoked (this process takes CurrFrame and
         modifies CdefFrame).
      4. The loop restoration process specified in § 7.20 Loop restoration process is invoked (this process
         takes CurrFrame and CdefFrame and produces LrFrame).
      5. If segmentation_enabled is equal to 1 and segmentation_update_map is equal to 0,
         SegmentIds[ row ][ col ] is set equal to PrevSegmentIds[ row ][ col ] for row = 0..MiRows-1, for
         col = 0..MiCols-1.
      6. If use_bru is equal to 1, it is a requirement of bitstream conformance that bru_region_valid() is
         equal to 1.

All the syntax elements that can be read in film_grain_model and film_grain_config should be saved into
an area of memory indexed by NUM_REF_FRAMES (this is the same as calling the save_grain_params
function specified in section § 7.23 Reference frame update process with an input of
NUM_REF_FRAMES). (This saving is needed because the reference frame update process can cause
previous frames to be reloaded and film grain applied.)

The reference frame update process as specified in § 7.23 Reference frame update process is invoked
(this process saves the current frame state into the reference frames and can cause frames to be output).

The frames to output are decided as follows:

  • If ShowExistingFrame is equal to 1, the output frame buffers process specified in § 7.21.6 Output
    frame buffers process is invoked with (derive_sef_order_hint ? frame_to_show_map_idx : -1) as input.
  • Otherwise, if immediate_output_frame is equal to 1, if the current frame has not already been output,
    the output frame buffers process specified in § 7.21.6 Output frame buffers process is invoked with -1
    as input.


  NOTE: When immediate_output_frame is equal to 1, the current frame is stored into the frame
  buffers by the reference frame update process. However, this process can trigger the output of
  frames which can themselves trigger the output of the current frame.




AV2 Specification                                                                                 Page 377 of 1169
The function bru_region_valid is used to check that BruModes has a valid pattern of blocks.

 bru_region_valid() {
     sbSize4 = Num_4x4_Blocks_Wide[ SbSize ]
     num = 0
     sbRows = (MiRows + sbSize4 - 1) / sbSize4
     sbCols = (MiCols + sbSize4 - 1) / sbSize4
     for( r = 0; r < sbRows; r++ ) {
         for( c = 0; c < sbCols; c++ ) {
              if ( BruModes[ r * sbSize4 ][ c * sbSize4 ] == BRU_ACTIVE ) {
                  left[num] = c - 1
                  right[num] = c + 1
                  top[num] = r - 1
                  bottom[num] = r + 1
                  active[num] = 1
                  num = num + 1
              }
         }
     }
     changed = 1
     while( changed ) {
         changed = 0
         for( a = 0; a < num; a++ ) {
              for( b = a + 1; b < num; b++) {
                  if ( active[a] && active[b] &&
                          !( right[a] < left[b] ||
                          right[b] < left[a] ||
                          bottom[a] < top[b] ||
                          bottom[b] < top[a] ) ) {
                      left[a] = Min( left[a], left[b] )
                      right[a] = Max( right[a], right[b] )
                      top[a] = Min( top[a], top[b] )
                      bottom[a] = Max( bottom[a], bottom[b] )
                      active[b] = 0
                      changed = 1
                  }
              }
         }
     }
     for( a = 0; a < num; a++ ) {
         if ( active[a] ) {
              for( r = top[ a ]; r <= bottom[ a ]; r++ ) {
                  for( c = left[ a ]; c <= right[ a ]; c++ ) {
                      row = r * sbSize4
                      col = c * sbSize4
                      if (row >= 0 && row < MiRows && col >= 0 && col < MiCols) {
                          if ( BruModes[ row ][ col ] == BRU_INACTIVE ) {
                              return 0
                          }
                          if ( r == top[ a ] ||
                                r == bottom[ a ] ||
                                c == left[ a ] ||
                                c == right[ a ] ) {
                              if ( BruModes[ row ][ col ] != BRU_SUPPORT) {
                                   return 0
                              }
                          }
                      }
                  }
              }
         }
     }
     return 1
 }




AV2 Specification                                                                             Page 378 of 1169
      NOTE: bru_region_valid merges rectangles of BRU_ACTIVE blocks together if the rectangles
      (including a one block wide boundary) overlap, and then checks that there are no inactive blocks
      inside each merged rectangle and that the edge of each merged rectangle is either off-screen or
      marked as support.


```

<a id="s-7-3"></a>

### § 7.3 Ordering of OBUs

```text
§   7.3. Ordering of OBUs
```

<a id="s-7-3-1"></a>

#### § 7.3.1 General

```text
§   7.3.1. General
    A bitstream conforming to this specification consists of one or more coded video sequences.

    A coded video sequence consists of one or more temporal units. A temporal unit consists of at least one
    coded output frame unit belonging to one coded extended layer unit. The definition of a coded output
    frame unit, coded non-output frame unit and coded extended layer unit are provided in sub-sections
    § 7.3.3 Coded output frame unit, § 7.3.4 Coded non-output frame unit, and § 7.3.6 Coded extended layer
    unit, respectively. The temporal unit is further specified in sub-section § 7.3.7 Temporal unit.

    A coded multistream video sequence is a set of coded video sequences across two or more extended
    layers that satisfies the following requirements:

     1. The temporal units of the coded video sequences collectively contain OBUs with two or more distinct
        non-global values of obu_xlayer_id.
     2. An OBU with obu_type equal to OBU_MSDO or an activated global layer configuration record OBU is
        present as specified in Annex A.2 Profiles.
     3. When an OBU with obu_type equal to OBU_MSDO is present, it is present in each temporal unit that
        contains a random access point.
     4. For each OBU in a coded multistream video sequence with obu_xlayer_id not equal to
        GLOBAL_XLAYER_ID, obu_xlayer_id must be equal to some value of sub_xlayer_id in the preceding
        OBU_MSDO or to some value of LcrXLayerID in the activated global LCR.
     5. All extended layers within a temporal unit share the same output time.
     6. The coded extended layer units from different extended layers within a temporal unit shall appear in
        ascending order of obu_xlayer_id.
     7. The extracted bitstream for each individual stream forms a valid bitstream.


      NOTE: Not all extended layers are required to be present in every temporal unit. For example, in a
      multistream bitstream where extended layers operate at different frame rates, a temporal unit may
      contain coded extended layer units for only a subset of the extended layers. When multiple extended
      layers are present in a temporal unit, they are required to share the same output time. An encoder
      may use the show existing frame mechanism to satisfy this requirement when extended layers use
      different coding structures.


      NOTE: The coded video sequences and random access points do not have to be aligned across
      different extended layers unless the OrderHint matching constraint is enabled via
      multistream_doh_constraint_flag or lcr_doh_constraint_flag (see § 7.3.7 Temporal unit and § 7.4.6
      Multistream Random Access).




    AV2 Specification                                                                             Page 379 of 1169
```

<a id="s-7-3-2"></a>

#### § 7.3.2 Coded multistream video sequence boundaries

```text
§   7.3.2. Coded multistream video sequence boundaries

    A coded multistream video sequence begins at a temporal unit that contains an OBU with obu_type equal
    to OBU_CLOSED_LOOP_KEY for at least one extended layer and satisfies one of the following conditions:

     1. No coded multistream video sequence is currently active and an OBU with obu_type equal to
        OBU_MSDO is present.
     2. A coded multistream video sequence is currently active, an OBU with obu_type equal to OBU_MSDO
        is present, and the value of multistream_profile_idc, multistream_level_idx, multistream_tier,
        num_streams_minus_2, multistream_even_allocation_flag, or multistream_large_picture_idc differs
        from the corresponding value in the previous OBU_MSDO.
     3. No coded multistream video sequence is currently active and a global layer configuration record is
        activated.

    A coded multistream video sequence ends at the earliest of:

     1. A temporal unit that begins a new coded multistream video sequence as defined above.
     2. A temporal unit that begins a new coded video sequence for at least one extended layer but does not
        contain an OBU with obu_type equal to OBU_MSDO and does not have an activated global layer
        configuration record.
     3. The end of the bitstream.

    At the end of a coded multistream video sequence, all remaining frames from all extended layers shall be
    output and all reference frame buffers for all extended layers shall be invalidated.


      NOTE: The values of sub_xlayer_id may change at a random access point without starting a new
      coded multistream video sequence.


    It is a requirement of bitstream conformance that, in a coded multistream video sequence in which both
    an OBU with obu_type equal to OBU_MSDO and an activated global layer configuration record are
    present, the set of coded multistream video sequence boundaries obtained by applying the rules of this
    section using both the MSDO and the activated global layer configuration record shall be identical to the
    set of boundaries obtained by applying those rules using the MSDO alone.


      NOTE: In a bitstream conforming to interoperability point 0 or interoperability point 1, an OBU
      with obu_type equal to OBU_MSDO is required whenever a coded multistream video sequence is
      present (see Annex A.2 Profiles, Table A.4). Together with the requirement above, this means that an
      implementation decoding such a bitstream may determine coded multistream video sequence
      boundaries from the MSDO alone, regardless of whether a global layer configuration record is also
      activated.

```

<a id="s-7-3-3"></a>

#### § 7.3.3 Coded output frame unit

```text
§   7.3.3. Coded output frame unit
    A coded output frame unit is a collection of consecutive OBUs in a bitstream, all having the same
    obu_xlayer_id, obu_mlayer_id, and obu_tlayer_id, according to the following rules and presence order:

      • Zero or one OBU with obu_type equal to OBU_CONTENT_INTERPRETATION,
      • Zero or more OBUs with obu_type equal to OBU_MULTI_FRAME_HEADER,



    AV2 Specification                                                                           Page 380 of 1169
  • Zero or more OBUs, which may be present in any order, with an obu_type equal to any of:

       ◦ OBU_BUFFER_REMOVAL_TIMING
       ◦ OBU_QUANTIZATION_MATRIX.
       ◦ OBU_FILM_GRAIN.
       ◦ OBU_METADATA_SHORT having metadata_is_suffix equal to 0
       ◦ OBU_METADATA_GROUP having metadata_is_suffix equal to 0
  • Either:

       ◦ One or more OBUs that contain a single coded frame with immediate_output_frame equal to 1 or
         implicit_output_frame equal to 1, where the OBUs of the coded frame have the same obu_type
         and the obu_type can be equal to any of:

            ▪ OBU_CLOSED_LOOP_KEY,
            ▪ OBU_OPEN_LOOP_KEY,
            ▪ OBU_LEADING_TILE_GROUP,
            ▪ OBU_REGULAR_TILE_GROUP,
            ▪ OBU_SWITCH,
            ▪ OBU_LEADING_TIP,
            ▪ OBU_REGULAR_TIP, and
            ▪ OBU_RAS_FRAME.

          If the OBUs of the coded frame have an obu_type equal to any of

            ▪ OBU_CLOSED_LOOP_KEY,
            ▪ OBU_OPEN_LOOP_KEY,
            ▪ OBU_LEADING_TILE_GROUP,
            ▪ OBU_REGULAR_TILE_GROUP,
            ▪ OBU_SWITCH, or
            ▪ OBU_RAS_FRAME,

          then the first encountered OBU shall have is_first_tile_group equal to 1, and all remaining OBUs
          of the same type, if present, shall have is_first_tile_group equal to 0.
  • Or:

       ◦ One OBU of either type OBU_LEADING_SEF or OBU_REGULAR_SEF.

    Such a frame is associated with a decoded display order hint value, OrderHint.
  • Zero or more OBUs that may be present in any order, with different types also allowed to be
    interleaved, as follows:

       ◦ Zero or more OBUs with obu_type equal to OBU_METADATA_SHORT having metadata_is_suffix
         equal to 1,




AV2 Specification                                                                             Page 381 of 1169
           ◦ Zero or more OBUs with obu_type equal to OBU_METADATA_GROUP having metadata_is_suffix
             equal to 1.

    OBUs with obu_type equal to OBU_PADDING may appear at any position within a coded output frame
    unit.

```

<a id="s-7-3-4"></a>

#### § 7.3.4 Coded non-output frame unit

```text
§   7.3.4. Coded non-output frame unit
    A coded non-output frame unit is a collection of OBUs, all having the same obu_xlayer_id, obu_mlayer_id,
    and obu_tlayer_id, according to the following rules and presence order:

      • Zero or one OBU with obu_type equal to OBU_CONTENT_INTERPRETATION,
      • Zero or more OBUs with obu_type equal to OBU_MULTI_FRAME_HEADER,
      • A sequence of different OBUs, which may be present in any order, with different types also allowed to
        be interleaved, as follows:

           ◦ Zero or one OBU with obu_type equal to OBU_BUFFER_REMOVAL_TIMING
           ◦ Zero or more OBUs with obu_type equal to OBU_QUANTIZATION_MATRIX.
           ◦ Zero or more OBUs with obu_type equal to OBU_FILM_GRAIN.
           ◦ Zero or more OBUs with obu_type equal to OBU_METADATA_SHORT having metadata_is_suffix
             equal to 0
           ◦ Zero or more OBUs with obu_type equal to OBU_METADATA_GROUP having metadata_is_suffix
             equal to 0
      • One or more OBUs that contain a single coded frame with immediate_output_frame equal to 0 and
        implicit_output_frame equal to 0, where the OBUs of the coded frame have the same obu_type and
        the obu_type can be equal to any of:

           ◦ OBU_CLOSED_LOOP_KEY,
           ◦ OBU_OPEN_LOOP_KEY,
           ◦ OBU_LEADING_TILE_GROUP,
           ◦ OBU_REGULAR_TILE_GROUP,
           ◦ OBU_SWITCH,
           ◦ OBU_LEADING_TIP,
           ◦ OBU_REGULAR_TIP,
           ◦ OBU_BRIDGE_FRAME, and
           ◦ OBU_RAS_FRAME.

        If the OBUs of the coded frame have an obu_type equal to any of the following values:

           ◦ OBU_CLOSED_LOOP_KEY,
           ◦ OBU_OPEN_LOOP_KEY,
           ◦ OBU_LEADING_TILE_GROUP,
           ◦ OBU_REGULAR_TILE_GROUP,
           ◦ OBU_SWITCH, or



    AV2 Specification                                                                           Page 382 of 1169
           ◦ OBU_RAS_FRAME,

        then the first encountered OBU shall have is_first_tile_group equal to 1, and all remaining OBUs of
        the same type, if present, shall have is_first_tile_group equal to 0.
      • A sequence of different OBUs, that may be present in any order, with different types also allowed to
        be interleaved, as follows:

           ◦ Zero or more OBUs with obu_type equal to OBU_METADATA_SHORT having metadata_is_suffix
             equal to 1,
           ◦ Zero or more OBUs with obu_type equal to OBU_METADATA_GROUP having metadata_is_suffix
             equal to 1.

    OBUs with obu_type equal to OBU_PADDING may appear at any position within a coded non-output
    frame unit.

```

<a id="s-7-3-5"></a>

#### § 7.3.5 Coded frame unit

```text
§   7.3.5. Coded frame unit

    A coded frame unit is either a coded output frame unit or a coded non-output frame unit.

```

<a id="s-7-3-6"></a>

#### § 7.3.6 Coded extended layer unit

```text
§   7.3.6. Coded extended layer unit
    A coded extended layer unit is a collection of OBUs that share the same obu_xlayer_id and are
    constrained to be present in the following order:

      • Zero or more OBUs with obu_type equal to OBU_LAYER_CONFIGURATION_RECORD,
      • Zero or more OBUs with obu_type equal to OBU_OPERATING_POINT_SET,
      • Zero or more OBUs with obu_type equal to OBU_ATLAS_SEGMENT,
      • Zero or more OBUs with obu_type equal to OBU_SEQUENCE_HEADER,
      • For each embedded layer present in the bitstream, in ascending order of obu_mlayer_id the following
        can be present in the following order:

           ◦ Zero or more coded non-output frame units in this layer,
           ◦ Zero or one coded output frame unit in this layer

    OBUs with obu_type equal to OBU_PADDING may appear at any position within a coded extended layer
    unit.

    The following constraints apply to every coded extended layer unit:

      • At least one coded output frame unit shall be present in the coded extended layer unit.
      • If at least one coded non-output frame unit in a particular embedded layer is present, then one coded
        output frame unit shall also be present in this same embedded layer.
      • All coded output frame units in this coded extended layer unit shall have the same value of
        OrderHint.
      • If a coded extended layer unit contains a CLK OBU, then the following shall apply:

           ◦ Only the first coded frame unit in each embedded layer of the coded extended layer unit can
             consist of CLK OBUs, while the first coded frame unit of the lowest embedded layer present in the
             coded extended layer unit shall be a CLK OBU.


    AV2 Specification                                                                             Page 383 of 1169
  • If a coded extended layer unit contains an OLK OBU, then the following shall apply:

       ◦ Only the first coded frame unit in each embedded layer of the coded extended layer unit can
         consist of OLK OBUs, while the first coded frame unit of the lowest embedded layer present in
         the coded extended layer unit shall be an OLK OBU.
  • A coded extended layer unit cannot contain both OLK and CLK OBUs.
  • If a coded extended layer unit contains a leading frame, then all coded frame units in that coded
    extended layer unit shall be leading frames.
  • If an OBU with obu_type equal to OBU_CONTENT_INTERPRETATION is present in a coded extended
    layer unit, it shall only be present in the first frame unit of each embedded layer within this coded
    extended layer unit.
  • If an OBU with obu_type equal to OBU_CONTENT_INTERPRETATION is present in any coded
    extended layer unit, this OBU shall also be present in the first coded extended layer unit of the
    sequence and shall contain the same contents in all its repetitions for a given embedded layer.


  NOTE: When performing random access at an OBU_RAS_FRAME, OBU_CLOSED_LOOP_KEY or
  OBU_OPEN_LOOP_KEY OBUs that are required as long-term reference frames may appear in the
  same coded extended layer unit as the random access frame. See § 7.3.9 Availability of long-term
  reference frames for the requirements on this case.


Each coded extended layer unit has an associated order hint that is given by the value of OrderHint in the
coded output frame units.


  NOTE: This is well defined because all coded output frame units are required to share the same
  value of OrderHint.


If monotonic_output_order_flag is equal to 0, it is a requirement of bitstream conformance that within a
coded video sequence, for a given value of obu_xlayer_id and obu_mlayer_id, if a coded output frame unit
X has an associated OrderHint value equal to ohX, there shall not be a coded output frame unit Y in the
same extended layer and embedded layer that appears later than X in output order and has an associated
OrderHint value less than or equal to ohX, unless a switch frame with restricted_prediction_switch equal
to 1 appears between X and Y in coding order.


  NOTE: The value of OrderHint is reset at the start of a new coded video sequence and at a switch
  frame with restricted_prediction_switch equal to 1. In both cases, the OrderHint counter is effectively
  restarted, allowing OrderHint values to be reused in subsequent coded output frame units.


For each coded extended layer unit that contains an OBU with obu_type equal to
OBU_CLOSED_LOOP_KEY or OBU_OPEN_LOOP_KEY, the OBUs within the coded extended layer unit for
each operating point satisfy two conditions:

  • The OBUs contain one or more coded frame units.
  • The first coded frame unit has obu_type equal to OBU_CLOSED_LOOP_KEY or
    OBU_OPEN_LOOP_KEY.




AV2 Specification                                                                             Page 384 of 1169
    A new coded video sequence for an extended layer is defined to start at each temporal unit that contains
    an OBU with obu_type equal to OBU_CLOSED_LOOP_KEY in the coded extended layer unit
    corresponding to the extended layer.

    Within a particular coded video sequence of an extended layer, it is allowed to send redundant copies of
    the activated sequence_header_obu, but the contents must be bit-identical each time the activated
    sequence header appears. A new coded video sequence is required if the activated sequence header
    parameters change.

    Within each extended layer, only one sequence header shall remain active for the duration of a coded
    video sequence, i.e., until a CLK is encountered for that extended layer. Additional sequence header
    OBUs with a different seq_header_id can be present in the bitstream but are not activated and have no
    effect on the decoding process until referenced by a subsequent CLK frame header.

    OBU types that are not defined in this specification can be ignored by a decoder.

```

<a id="s-7-3-7"></a>

#### § 7.3.7 Temporal unit

```text
§   7.3.7. Temporal unit

    A temporal unit consists of a series of OBUs constrained to be present in the following order:

      • One OBU with obu_type equal to OBU_TEMPORAL_DELIMITER associated with obu_xlayer_id equal
        to GLOBAL_XLAYER_ID,
      • Zero or one OBU with obu_type equal to OBU_MSDO,
      • Zero or more OBUs with obu_type equal to OBU_LAYER_CONFIGURATION_RECORD associated with
        obu_xlayer_id equal to GLOBAL_XLAYER_ID,
      • Zero or more OBUs with obu_type equal to OBU_OPERATING_POINT_SET associated with
        obu_xlayer_id equal to GLOBAL_XLAYER_ID,
      • Zero or more OBUs with obu_type equal to OBU_ATLAS_SEGMENT associated with obu_xlayer_id
        equal to GLOBAL_XLAYER_ID,
      • Zero or more OBUs with obu_type equal to OBU_METADATA_SHORT or OBU_METADATA_GROUP
        associated with obu_xlayer_id equal to GLOBAL_XLAYER_ID and having metadata_is_suffix equal to
        0,
      • For each extended layer present in this temporal unit, in ascending order of obu_xlayer_id, a coded
        extended layer unit as defined in § 7.3.6 Coded extended layer unit.

    Additionally, OBUs with obu_type equal to OBU_PADDING may also appear at any position within a
    temporal unit. When present outside of a coded extended layer unit, they shall have obu_xlayer_id equal
    to GLOBAL_XLAYER_ID.

    Furthermore, it is a requirement of bitstream conformance that when lcr_doh_constraint_flag in the
    activated global LCR is equal to 1, or multistream_doh_constraint_flag in the preceding MSDO is equal to
    1, the following conditions are additionally satisfied for each temporal unit in the coded multistream
    video sequence:

      • All frame units within this temporal unit shall use the same value of OrderHintBits.
      • Coded output frame units present in multiple coded extended layer units within this temporal unit
        shall have the same value of OrderHint.




    AV2 Specification                                                                            Page 385 of 1169
```

<a id="s-7-3-8"></a>

#### § 7.3.8 Availability of high level syntax OBUs

```text
§   7.3.8. Availability of high level syntax OBUs

```

<a id="s-7-3-8-1"></a>

##### § 7.3.8.1 General

```text
§   7.3.8.1. General

    High level syntax (HLS) OBUs carry configuration and parameter information that is referenced by other
    OBUs during the decoding process. Each HLS OBU shall be available to the decoding process prior to
    being referenced, by inclusion in the bitstream or by provision through external means.

    This shall also be true if decoding process starts at any random access point and drops any temporal
    units containing leading frames.


      NOTE: This means that HLS OBUs used at a random access point need to be resent in the same
      temporal unit (or be provided through external means). As a result, HLS OBUs such as sequence
      headers, multi-frame headers and film grain models that were only available from earlier positions in
      the bitstream cannot be assumed to be available at a random access point. When HLS OBUs are
      provided through external means, they remain available to the decoding process until superseded.


    The semantics of syntax elements within an HLS OBU apply only when that OBU is activated for the
    current decoding context. An HLS OBU that is present in the bitstream but not activated has no effect on
    the decoding process.

    The following subsections specify the availability requirements for each HLS OBU type.

```

<a id="s-7-3-8-2"></a>

##### § 7.3.8.2 MSDO availability

```text
§   7.3.8.2. MSDO availability

    When an OBU with obu_type equal to OBU_MSDO is present in a multistream bitstream, it shall be
    available to the decoding process at each random access point, by inclusion in the bitstream or by
    provision through external means. The requirements on the presence of MSDO OBUs depend on the
    interoperability point, as specified in Annex A.2 Profiles.

    It is a requirement of bitstream conformance that an OBU with obu_type equal to OBU_MSDO that is not
    at a random access point shall be identical to the previous OBU_MSDO.

```

<a id="s-7-3-8-3"></a>

##### § 7.3.8.3 LCR availability

```text
§   7.3.8.3. LCR availability

    A layer configuration record OBU with obu_xlayer_id equal to GLOBAL_XLAYER_ID and
    lcr_global_config_record_id equal to id shall be available to the decoding process prior to being
    referenced by a local layer configuration record OBU with lcr_global_id equal to id, or by a sequence
    header with seq_lcr_id equal to id, by inclusion in the bitstream or by provision through external means.

    A layer configuration record OBU with obu_xlayer_id not equal to GLOBAL_XLAYER_ID shall be available
    to the decoding process prior to being referenced by a sequence header with seq_lcr_id that resolves to
    this local layer configuration record, by inclusion in the bitstream or by provision through external
    means.

```

<a id="s-7-3-8-4"></a>

##### § 7.3.8.4 Atlas segment OBU availability

```text
§   7.3.8.4. Atlas segment OBU availability

    An atlas segment OBU with obu_xlayer_id equal to GLOBAL_XLAYER_ID and atlas_segment_id equal to id
    can be available to the decoding process prior to being referenced by a layer configuration record with
    lcr_global_atlas_id equal to id, by inclusion in the bitstream or by provision through external means.




    AV2 Specification                                                                           Page 386 of 1169
    An atlas segment OBU with obu_xlayer_id not equal to GLOBAL_XLAYER_ID and atlas_segment_id equal
    to id shall be available to the decoding process prior to being referenced by a layer configuration record
    with lcr_local_atlas_id equal to id, by inclusion in the bitstream or by provision through external means.

```

<a id="s-7-3-8-5"></a>

##### § 7.3.8.5 OPS availability

```text
§   7.3.8.5. OPS availability

    An operating point set OBU with obu_xlayer_id equal to GLOBAL_XLAYER_ID and ops_id equal to id shall
    be available to the decoding process prior to being referenced, by inclusion in the bitstream or by
    provision through external means.

    An operating point set OBU with obu_xlayer_id not equal to GLOBAL_XLAYER_ID and ops_id equal to id
    shall be available to the decoding process prior to being referenced, by inclusion in the bitstream or by
    provision through external means.


      NOTE:       The use of operating point set OBUs is optional for decoders.

```

<a id="s-7-3-8-6"></a>

##### § 7.3.8.6 Sequence header availability

```text
§   7.3.8.6. Sequence header availability

    A sequence header OBU with seq_header_id equal to id shall be available to the decoding process prior to
    being referenced by a frame header with seq_header_id_in_frame_header equal to id, or by a multi-frame
    header OBU with mfh_seq_header_id equal to id, by inclusion in the bitstream or by provision through
    external means.

    When seq_lcr_id is not equal to 0, the layer configuration record referenced by seq_lcr_id shall be
    available per § 7.3.8.3 LCR availability.

    See § 7.3.6 Coded extended layer unit for additional constraints on sequence header lifetime within a
    coded video sequence.

```

<a id="s-7-3-8-7"></a>

##### § 7.3.8.7 Multi-frame header availability

```text
§   7.3.8.7. Multi-frame header availability

    A multi-frame header OBU with mfh_id_minus_1 equal to id minus 1 shall be available to the decoding
    process prior to being referenced by a frame header with cur_mfh_id equal to id, by inclusion in the
    bitstream or by provision through external means.

    It is a requirement of bitstream conformance that the layer dependency constraints
    TLayerDependencyMap and MLayerDependencyMap are satisfied for the referenced multi-frame header
    OBU.

    The sequence header referenced by mfh_seq_header_id shall be available per § 7.3.8.6 Sequence header
    availability.

```

<a id="s-7-3-8-8"></a>

##### § 7.3.8.8 Film grain OBU availability

```text
§   7.3.8.8. Film grain OBU availability

    When apply_grain is equal to 1 in a frame header, a film grain OBU that has set
    FilmGrainPresent[ fgm_id ] equal to 1 for the referenced fgm_id shall be available to the decoding
    process, by inclusion in the bitstream or by provision through external means.

    It is a requirement of bitstream conformance that the layer dependency constraints
    TLayerDependencyMap and MLayerDependencyMap are satisfied for the referenced film grain model, as
    specified in § 6.17.10.1 Film grain config semantics.




    AV2 Specification                                                                            Page 387 of 1169
```

<a id="s-7-3-8-9"></a>

##### § 7.3.8.9 Quantization matrix OBU availability

```text
§   7.3.8.9. Quantization matrix OBU availability

    When using_qmatrix is equal to 1 in a frame header, the quantization matrix levels referenced by qm_y,
    qm_u, and qm_v shall be available to the decoding process, by inclusion of a quantization matrix OBU in
    the bitstream or by provision through external means.

    Quantization matrix levels from previous temporal units are reset at the first OBU in a temporal unit with
    obu_type equal to OBU_CLOSED_LOOP_KEY or OBU_OPEN_LOOP_KEY or OBU_SWITCH or
    OBU_RAS_FRAME (the QmProtected array is used to avoid the reset of levels sent in the current
    temporal unit). When initiating decoding at a random access point, a decoder shall ensure that any
    required quantization matrix levels are available. If obu_type is equal to OBU_SWITCH, the reset only
    applies if restricted_prediction_switch is equal to 1.

    It is a requirement of bitstream conformance that the layer dependency constraints
    TLayerDependencyMap and MLayerDependencyMap are satisfied for the referenced quantization matrix
    levels, as specified in § 6.17.6.2 Setup QM params semantics.

```

<a id="s-7-3-8-10"></a>

##### § 7.3.8.10 Content interpretation OBU availability

```text
§   7.3.8.10. Content interpretation OBU availability

    When present, a content interpretation OBU shall be available to the decoding process from the first
    coded extended layer unit of the embedded layer in the coded video sequence in which it is present, by
    inclusion in the bitstream or by provision through external means.

    All instances of a content interpretation OBU for a given embedded layer within a coded video sequence
    shall contain the same information, as specified in § 6.14 Content interpretation OBU semantics.

    CI OBUs shall only appear in the first coded frame unit of each embedded layer within a temporal unit.

    If a CI OBU is present in any temporal unit for a given embedded layer, a CI OBU shall also be present in
    the first temporal unit of the coded video sequence for that embedded layer and shall contain the same
    contents.

```

<a id="s-7-3-8-11"></a>

##### § 7.3.8.11 Content interpretation parameters initialization

```text
§   7.3.8.11. Content interpretation parameters initialization

    The content interpretation parameters for each embedded layer in an extended layer are initialized to
    default values at the start of the decoder and at each random access point of the extended layer (i.e., at
    each temporal unit containing an OBU in the extended layer with obu_type equal to
    OBU_CLOSED_LOOP_KEY or OBU_OPEN_LOOP_KEY).

    The default values for the content interpretation parameters are:

      • ci_scan_type_idc = 0 (unspecified)
      • ci_color_description_present_flag = 0
      • ci_color_primaries = CP_UNSPECIFIED
      • ci_transfer_characteristics = TC_UNSPECIFIED
      • ci_matrix_coefficients = MC_UNSPECIFIED
      • ci_full_range_flag = 0
      • ci_chroma_sample_position_top = CSP_UNSPECIFIED
      • ci_chroma_sample_position_bottom = CSP_UNSPECIFIED



    AV2 Specification                                                                             Page 388 of 1169
      • ci_aspect_ratio_info_present_flag = 0
      • ci_timing_info_present_flag = 0
      • ci_extension_present_flag = 0

    If the decoding process starts at a random access point, the content interpretation parameters for each
    embedded layer m are determined as follows:

     1. The content interpretation parameters for embedded layer m are first reset to the default values
        listed above.
     2. If a content interpretation OBU is present in the same temporal unit for embedded layer m, the
        content interpretation parameters are set to the values specified in that OBU.
     3. Otherwise, if no content interpretation OBU is present for embedded layer m and there exists an
        embedded layer k such that MLayerPresenceMap[m][k] is equal to 1 and content interpretation
        parameters have been established for embedded layer k, the content interpretation parameters for
        embedded layer m are inherited from embedded layer k, where k is the highest such embedded layer
        less than m.

    It is a requirement of bitstream conformance that when a content interpretation OBU is present in a
    temporal unit that does not contain a CLK or OLK for the same embedded layer, and does not contain a
    CLK or OLK for any embedded layer k where MLayerPresenceMap[m][k] is equal to 1, the contents of
    that content interpretation OBU shall be identical to the content interpretation parameters that were
    established at the most recent random access point for that embedded layer.

```

<a id="s-7-3-9"></a>

#### § 7.3.9 Availability of long-term reference frames

```text
§   7.3.9. Availability of long-term reference frames

```

<a id="s-7-3-9-1"></a>

##### § 7.3.9.1 General

```text
§   7.3.9.1. General

    Long-term reference frames carry frame data that is referenced by other OBUs during the decoding
    process. Each long-term reference frame shall be available to the decoding process prior to being
    referenced, by inclusion in the bitstream or by provision through external means, and shall be held in the
    same reference frame buffer slot that it would occupy under sequential decoding.

    When initiating decoding at a random access point containing an OBU_RAS_FRAME, or an
    OBU_OPEN_LOOP_KEY when long_term_frame_id_bits is not equal to 0, inclusion of long-term reference
    frames in the bitstream may result in coded extended layer units that do not follow the constraints in
    § 7.3.6 Coded extended layer unit. It is a requirement of bitstream conformance that in this case, any
    OBU_CLOSED_LOOP_KEY OBUs that are required as long-term reference frames appear as the first
    coded frame units in the coded extended layer unit containing the random access frame, followed by any
    OBU_OPEN_LOOP_KEY OBUs that are required as long-term reference frames. These long-term
    reference frame OBUs shall have immediate_output_frame equal to 0 and implicit_output_frame equal to
    0.


      NOTE: The definition of a coded extended layer unit requires that long-term reference frames with
      immediate_output_frame equal to 0 and implicit_output_frame equal to 0 are included in the same
      coded extended layer unit as the random access frame. Since the long-term reference frames are one
      or more OBU_CLOSED_LOOP_KEY and OBU_OPEN_LOOP_KEY OBUs, the above allows these frames
      to be in the same coded extended layer unit as the OBU_RAS_FRAME or OBU_OPEN_LOOP_KEY for
      the purpose of performing a random access operation.




    AV2 Specification                                                                           Page 389 of 1169
```

<a id="s-7-4"></a>

### § 7.4 Random access decoding

```text
§   7.4. Random access decoding
    This section specifies how decoding is initiated at a random access point. Three random access processes
    are described: a closed random access process (§ 7.4.3 Closed Random Access) for
    OBU_CLOSED_LOOP_KEY, an open random access process (§ 7.4.4 Open Random Access) for
    OBU_OPEN_LOOP_KEY, and a random access switch process (§ 7.4.5 Random Access Switch) for
    OBU_RAS_FRAME. These processes apply anytime decoding is initiated at one of these OBUs, which
    includes at the start of a new coded video sequence or at the start of a new coded multistream video
    sequence, both of which always begin at a closed random access point. In a multistream bitstream, a
    temporal unit may be a random access point for some extended layers but not for others, which is also
    described in this section and specified in § 7.4.6 Multistream Random Access.

```

<a id="s-7-4-1"></a>

#### § 7.4.1 General

```text
§   7.4.1. General

    A temporal unit containing one or more OBUs with obu_type equal to OBU_CLOSED_LOOP_KEY,
    OBU_OPEN_LOOP_KEY, or OBU_RAS_FRAME is defined to be a random access point. Decoding can be
    correctly initiated at such a temporal unit. The availability requirements for initiating decoding at a
    random access point are specified in § 7.3.8 Availability of high level syntax OBUs and § 7.3.9 Availability
    of long-term reference frames.

    The process of initiating decoding at a random access point follows the ordered steps:

     1. If the temporal unit contains one or more OBUs with an obu_type equal to OBU_CLOSED_LOOP_KEY,
        OBU_OPEN_LOOP_KEY or OBU_RAS_FRAME, the variable isRandomAccessPoint is set equal to 1.
        Otherwise, isRandomAccessPoint is set equal to 0.
     2. If isRandomAccessPoint is equal to 1, the variable MultiStreamDecoderMode is determined as
        follows:

          1. If the temporal unit contains one or more OBUs with an obu_type equal to OBU_MSDO then
             MultiStreamDecoderMode is set equal to 1.
          2. Otherwise, MultiStreamDecoderMode is set equal to 0.
     3. For each coded extended layer unit in the temporal unit, the random access process for that extended
        layer is determined by the OBU type present in the coded extended layer unit:

          1. If the first coded frame unit in a coded extended layer unit contains an OBU with obu_type equal
             to OBU_CLOSED_LOOP_KEY, then the closed loop key frame random access process in § 7.4.3
             Closed Random Access applies to that extended layer.
          2. Otherwise, if the first coded frame unit in the coded extended layer unit contains an OBU with
             obu_type equal to OBU_OPEN_LOOP_KEY, then the open loop key frame random access process
             in § 7.4.4 Open Random Access applies to that extended layer.
          3. Otherwise, if the coded extended layer unit contains an OBU with obu_type equal to
             OBU_RAS_FRAME, then the random access switch process in § 7.4.5 Random Access Switch
             applies to that extended layer.


      NOTE: The value for MultiStreamDecoderMode can only be updated at a random access point. The
      value for MultiStreamDecoderMode then persists for subsequent temporal units that are not random
      access points.




    AV2 Specification                                                                              Page 390 of 1169
      NOTE: MultiStreamDecoderMode is set to 1 only when an MSDO OBU is present. A multistream
      bitstream that does not contain an MSDO OBU will have MultiStreamDecoderMode equal to 0.


      NOTE: For multistream bitstreams, additional random access requirements are specified in § 7.4.6
      Multistream Random Access.

```

<a id="s-7-4-2"></a>

#### § 7.4.2 Random access and use of long-term reference frames

```text
§   7.4.2. Random access and use of long-term reference frames

```

<a id="s-7-4-2-1"></a>

##### § 7.4.2.1 Random access with long-term reference frames

```text
§   7.4.2.1. Random access with long-term reference frames

    A coded video sequence may use random access with long-term reference frames when
    long_term_frame_id_bits is set to a value not equal to 0 in the sequence header associated with this coded
    video sequence. In such a coded video sequence, the random access described in § 7.4.4 Open Random
    Access and § 7.4.5 Random Access Switch may rely on previous OBU_CLOSED_LOOP_KEY and
    OBU_OPEN_LOOP_KEY frame data for the decoding of the video sequence. When the decoding starts
    with § 7.4.4 Open Random Access and § 7.4.5 Random Access Switch, this frame data may need to be
    provided.

```

<a id="s-7-4-2-2"></a>

##### § 7.4.2.2 Random access without long-term reference frames

```text
§   7.4.2.2. Random access without long-term reference frames

    A coded video sequence uses random access without long-term reference frames when
    long_term_frame_id_bits is set to 0 in the sequence header associated with this coded video sequence. In
    such a coded video sequence, random access described in § 7.4.4 Open Random Access and § 7.4.5
    Random Access Switch does not use any previous OBU_CLOSED_LOOP_KEY and OBU_OPEN_LOOP_KEY
    frame data for the decoding of the video sequence.

```

<a id="s-7-4-3"></a>

#### § 7.4.3 Closed Random Access

```text
§   7.4.3. Closed Random Access

    The closed random access process applies to an extended layer when the first coded frame unit in the
    coded extended layer unit has obu_type equal to OBU_CLOSED_LOOP_KEY. The process starts a new
    coded video sequence for the extended layer (see § 7.3.6 Coded extended layer unit).

    When the closed random access process is invoked for an extended layer, the following apply:

      • All reference frame buffers for the extended layer are invalidated, and any pending implicit output
        frames are flushed. See § 5.18.2 Frame header info syntax for the associated variable assignments.
      • The sequence header referenced by the CLK frame header becomes the active sequence header and
        remains active for the remainder of the new coded video sequence, as specified in § 7.3.6 Coded
        extended layer unit.
      • Content interpretation parameters for each embedded layer in the extended layer are re-established,
        as specified in § 7.3.8.11 Content interpretation parameters initialization.
      • Quantization matrix levels from previous temporal units are reset, as specified in § 7.3.8.9
        Quantization matrix OBU availability.

```

<a id="s-7-4-4"></a>

#### § 7.4.4 Open Random Access

```text
§   7.4.4. Open Random Access

    The open random access process applies to an extended layer when the first coded frame unit in the
    coded extended layer unit has obu_type equal to OBU_OPEN_LOOP_KEY. During sequential decoding, the
    process does not start a new coded video sequence for the extended layer. However, when a decoder



    AV2 Specification                                                                             Page 391 of 1169
initiates decoding at the open random access point, the process is treated as if it were the start of a new
coded video sequence for the extended layer (see § 7.3.6 Coded extended layer unit). For the purposes of
the decoding process, all reference frame buffers not refreshed by the OLK are invalidated except for the
long term reference frames listed in ref_long_term_id, leading frames are discarded, and the sequence
header referenced by the OLK frame header is activated.


  NOTE: During sequential decoding, the OLK does not start a new coded video sequence. Leading
  frames that follow the OLK can be decoded using reference frames from the preceding frames.


Provided the following HLS OBUs are available to the decoding process, by inclusion in the bitstream or
by provision through external means, and that the long term reference condition defined below is
satisfied, decoding can be correctly initiated at such a temporal unit, and all subsequent non-leading
frames in decoding order can be correctly decoded, without performing the decoding process of any
frames that precede the temporal unit in decoding order (with exception of long term reference frames
listed in ref_long_term_id of this OLK):

  • A sequence header OBU with seq_header_id equal to the value referenced by the OLK frame header.
  • If seq_lcr_id in the sequence header is not equal to 0, the layer configuration record referenced by
    seq_lcr_id.
  • If the referenced layer configuration record references an atlas segment OBU via lcr_global_atlas_id
    or lcr_local_atlas_id, that atlas segment OBU.
  • If cur_mfh_id is greater than 0, the multi-frame header OBU with mfh_id_minus_1 equal to cur_mfh_id
    minus 1.
  • If apply_grain is equal to 1, the film grain OBU for the referenced fgm_id.
  • If using_qmatrix is equal to 1 and the referenced quantization matrix levels differ from the default
    levels established by the sequence header, the quantization matrix OBU providing those levels.
  • In a multistream bitstream, an OBU with obu_type equal to OBU_MSDO or a global layer
    configuration record OBU, when present.

The long term reference condition is defined such that one or more of the following shall be satisfied:

 1. long_term_frame_id_bits is equal to 0 for this sequence (where ref_long_term_id is inferred as empty),
    or
 2. num_key_ref_frames is equal to 0 in this OLK frame header (where ref_long_term_id is inferred as
    empty), or
 3. The decoded reference frames identified by the ref_long_term_id values signaled in the OLK frame
    header are available. These reference frames are retained from the previous coded video sequence
    and are required for reference in future inter frames.

It is a requirement of bitstream conformance that any regular frames (IsRegular equal to 1) after an OLK
shall not reference any frames (or other information stored by the reference frame update process § 7.23
Reference frame update process ) that precede the OLK temporal unit, other than information made
available through the reference frame buffers refreshed by the OLK temporal unit, or the long term
references included in ref_long_term_id.

Regular frames that follow leading frames after the OLK temporal unit shall also not reference leading
frames or HLS OBUs that are indicated in temporal units containing leading frames.


AV2 Specification                                                                            Page 392 of 1169
    The constraint to not reference leading frames is enforced by the reference frame invalidation process in
    § 5.18.1 General frame header syntax, which sets RefValid[ i ] equal to 0 for reference frame slots not
    refreshed by the OLK when the first Regular frame is encountered.

    See § 7.3.8 Availability of high level syntax OBUs for the general availability requirements for each HLS
    OBU type.

    See § 7.3.9 Availability of long-term reference frames for the availability requirements for long-term
    reference frames.

    A long term reference frame shall be included in the ref_long_term_id list of an OLK, if and only if:

     1. when using sequential decoding, this long term reference frame is held in a reference frame buffer
        when the OLK is encountered, and
     2. when using sequential decoding, this long term reference frame is held in a reference frame buffer
        when the first Regular frame (in a different temporal unit than the OLK) is encountered after the
        OLK, and
     3. The long term reference frame is in the same embedded layer as the OLK, or is in an embedded layer
        that is dependent on the embedded layer of the OLK


      NOTE: the constraints on the ref_long_term_id list above ensure that the reference frame buffers
      are the same whether randomly accessed from an OLK, or sequentially decoded. For example,
      consider the case when a leading frame updates a reference frame buffer that was originally taken by
      a long term reference. If randomly accessed, then the long term reference would still be available
      (given it is incorrectly included in the OLK ref_long_term_id list), but if sequentially decoded, the long
      term reference would not be held in a reference frame buffer. This is avoided by the constraints.


    It is a requirement of bitstream conformance that if long_term_frame_id_bits is greater than 0, the
    OrderHint of an OLK shall be less than (1 << OrderHintBits).


      NOTE: This constraint ensures that the OrderHint of an OLK is equal to the value of order_hint in
      the bitstream (i.e., no modular wrap-around has occurred) when long-term reference frames are in
      use. This guarantees that the relative distance and ordering between an OLK and its long-term
      reference frames are the same whether decoding is sequential or initiated at the OLK as a random
      access point. Encoders may select an appropriate value for order_hint_bits_minus_1 when addressing
      this constraint.

```

<a id="s-7-4-5"></a>

#### § 7.4.5 Random Access Switch

```text
§   7.4.5. Random Access Switch

    The random access switch process applies to an extended layer when the coded extended layer unit
    contains an OBU with obu_type equal to OBU_RAS_FRAME. The process does not start a new coded
    video sequence for the extended layer (see § 7.3.6 Coded extended layer unit).


      NOTE: The RAS frame is an inter-predicted frame. Although it is inter-predicted, it may only
      reference long-term reference frames whose RefLongTermId appears in the ref_long_term_id list, as
      specified in § 6.17 Frame header OBU semantics. This restriction is what enables random access at an
      inter-predicted frame.




    AV2 Specification                                                                              Page 393 of 1169
    For decoding to be correctly initiated at a RAS frame, one of the following shall be satisfied:

     1. num_key_ref_frames is equal to 0 in this RAS frame header (where ref_long_term_id is inferred as
        empty), or
     2. The decoded reference frames identified by the ref_long_term_id values signaled in the RAS frame
        header are available, as specified in § 7.3.9 Availability of long-term reference frames.

    When the random access switch process is invoked for an extended layer, the following apply:

      • Reference frame buffers that do not hold long-term reference frames listed in ref_long_term_id are
        refreshed by the RAS frame, as specified in § 6.17 Frame header OBU semantics.
      • The active sequence header for the extended layer remains in effect, as specified in § 7.3.6 Coded
        extended layer unit.
      • Any active layer configuration record remains in effect as part of the active sequence header (see
        § 6.4 Sequence header OBU semantics for LCR activation).
      • Quantization matrix levels from previous temporal units are reset, as specified in § 7.3.8.9
        Quantization matrix OBU availability.


      NOTE: After the reference frame update process, only the first refreshed reference frame buffer
      (containing the decoded RAS frame) and the long-term reference frame buffers identified by
      ref_long_term_id are valid. See § 7.23 Reference frame update process.


    The following bitstream conformance requirements apply to RAS frames:

    It is a requirement of bitstream conformance that if a long term reference frame is included in the
    ref_long_term_id list of a RAS frame, then, when using sequential decoding, this long term reference
    frame is held in a reference frame buffer when the RAS frame is encountered.


      NOTE: The constraint on the ref_long_term_id list above prevents the list from declaring long-term
      reference frames that are not present in a reference frame buffer under sequential decoding.


    It is a requirement of bitstream conformance that if long_term_frame_id_bits is greater than 0, the
    OrderHint of a RAS frame with restricted_prediction_switch equal to 0 shall be less than (1 <<
    OrderHintBits).


      NOTE: This constraint ensures that the OrderHint of a RAS frame is equal to the value of
      order_hint in the bitstream (i.e., no modular wrap-around has occurred) when long-term reference
      frames are in use. This guarantees that the relative distance and ordering between a RAS frame and
      its long-term reference frames are the same whether decoding is sequential or initiated at the RAS
      frame as a random access point. Encoders may select an appropriate value for
      order_hint_bits_minus_1 when addressing this constraint.

```

<a id="s-7-4-6"></a>

#### § 7.4.6 Multistream Random Access

```text
§   7.4.6. Multistream Random Access

    In a multistream bitstream, different coded extended layer units within the same temporal unit may
    contain different types of random access OBUs (e.g., OBU_CLOSED_LOOP_KEY in one extended layer
    and OBU_OPEN_LOOP_KEY in another). As specified in § 7.4.1 General, the corresponding random access
    process applies independently to each extended layer.


    AV2 Specification                                                                                 Page 394 of 1169
    Random access points are not required to be aligned across extended layers, and a temporal unit may be
    a random access point for some extended layers but not for others. However, when
    MultiStreamDecoderMode is equal to 1 and multistream_doh_constraint_flag is equal to 1, or when a
    global layer configuration record is activated and lcr_doh_constraint_flag is equal to 1, all coded output
    frame units present together in a temporal unit are required to share the same OrderHintBits and
    OrderHint, as specified in § 7.3.7 Temporal unit.

    When a decoder initiates decoding at a temporal unit that is a random access point for a subset of the
    extended layers in the multistream, the decoder shall not decode coded extended layer units for an
    extended layer until a random access point for that extended layer is encountered.

    When an OBU with obu_type equal to OBU_MSDO is present, it is parsed before any coded extended
    layer units in the temporal unit, as specified in § 7.3.7 Temporal unit. The variable
    MultiStreamDecoderMode and the sub_xlayer_id array are therefore established before the per-extended-
    layer random access processes are invoked.

```

<a id="s-7-5"></a>

### § 7.5 Frame end update CDF process

```text
§   7.5. Frame end update CDF process
    This process is triggered when the function frame_end_update_cdf is called from the tile group syntax
    table.

    The frame CDF arrays are set based on the saved CDF arrays as follows.

    A copy is made of the saved CDF values for each of the CDF arrays mentioned in the semantics for
    init_coeff_cdfs and init_non_coeff_cdfs. The name of the destination for the copy is the name of the CDF
    array with no prefix. The name of the source for the copy is the name of the CDF array prefixed with
    "Saved".

    Once the CDF arrays have been copied, the last entry in each destination array, representing the symbol
    count for that context, is set equal to (3 * count) >> 2 where count is equal to the value of the last entry in
    each source array.

    For example, the array IdentityRowYCdf will be created as follows:

     for( i = 0; i < PALETTE_ROW_FLAG_CONTEXTS; i++ ) {
         for ( j = 0; j < 4; j++ ) {
             IdentityRowYCdf[ i ][ j ] = SavedIdentityRowYCdf[ i ][ j ]
         }
         IdentityRowYCdf[ i ][ 3 ] = ( 3 * SavedIdentityRowYCdf[ i ][ 3 ] ) >> 2
     }


```

<a id="s-7-6"></a>

### § 7.6 Extended layer context management

```text
§   7.6. Extended layer context management
    The function save_xlayer_context is used to save information corresponding to the decoder state when
    obu_xlayer_id was last processed.

     save_xlayer_context( obu_xlayer_id ) {

          if( obu_xlayer_id == GLOBAL_XLAYER_ID )
              return

          if( MultiStreamDecoderMode ) {
              for( i = 0; i < num_streams_minus_2 + 2; i++ ) {
                  if( sub_xlayer_id[i] == obu_xlayer_id ) {




    AV2 Specification                                                                                Page 395 of 1169
                    streamID = i
                    break
               }
          }
      } else {
          streamID = obu_xlayer_id
      }

      save_context( streamID )

 }


where save_context( streamID ) stores all decoder state information for the current obu_xlayer_id in a
memory location denoted by the streamID value.

The function load_xlayer_context is used to load information corresponding to the decoder state when
obu_xlayer_id was last processed.

 load_xlayer_context( obu_xlayer_id ) {

      if( obu_xlayer_id == GLOBAL_XLAYER_ID )
          return

      if( MultiStreamDecoderMode ) {
          for( i = 0; i < num_streams_minus_2 + 2; i++ ) {
               if( sub_xlayer_id[i] == obu_xlayer_id ) {
                   streamID = i
                   break
               }
          }
      } else {
          streamID = obu_xlayer_id
      }

      load_context( streamID )

 }


where load_context( streamID ) loads all decoder state information for the current obu_xlayer_id from the
memory location denoted by the streamID value.


  NOTE: This specification defines decoding as the sequential processing of OBUs. The
  load_xlayer_context() and save_xlayer_context() realize the separate processing of extended layers in
  this context. Some implementations may use separate instances or other methods to separate the
  processing of individual streamIDs. These implementations may not need to implement the
  load_xlayer_context() and save_xlayer_context() functions.


  NOTE: When MultiStreamDecoderMode is equal to 0, the streamID is set directly to obu_xlayer_id.
  This applies both to singlestream bitstreams and to multistream bitstreams that do not contain an
  MSDO OBU.


  NOTE: When MultiStreamDecoderMode is equal to 1, the sub_xlayer_id lookup in
  save_xlayer_context and load_xlayer_context is guaranteed to find a match for any conformant
  bitstream, as a coded multistream video sequence requires that every obu_xlayer_id value (excluding
  GLOBAL_XLAYER_ID) corresponds to a value in sub_xlayer_id.




AV2 Specification                                                                           Page 396 of 1169
```

<a id="s-7-7"></a>

### § 7.7 Get ref frames process

```text
§   7.7. Get ref frames process
    This process is triggered if the function get_ref_frames is called while reading the frame header info.

    The input to this process is the variable checkRes specifying if the resolution of reference frames is used.

    The syntax elements in the ref_frame_idx array are computed based on the quantizer and display order
    hints saved for the reference frames.

    Variables indicating the quantizer and display order hint for distinct reference frames are prepared as
    follows:

     maxDisp = 0
     for ( i = 0; i < NumRefFrames; i++ ) {
         mapOrderHint[i] = -1
         if ( first_slot_with_ref(i) && RefOrderHint[i] != RESTRICTED_OH &&
               ( !IsBridge || i == bridge_frame_ref_idx ) &&
               (AllowedFrames & (1 << i)) &&
               TLayerDependencyMap[obu_mlayer_id][obu_tlayer_id][RefTLayerId[i]] &&
               MLayerDependencyMap[obu_mlayer_id][RefMLayerId[i]] ) {
             if ( valid_ref_frame_size( checkRes, i ) ) {
                  mapOrderHint[i] = RefOrderHint[i]
             }
             mapBaseQIdx[i] = RefBaseQIdx[i]
             maxDisp = Max( maxDisp, RefOrderHint[i])
         }
     }


    where first_slot_with_ref detects distinct reference frames as follows:

     first_slot_with_ref( i ) {
         if ( !RefValid[i] ) {
             return 0
         }
         for ( j = 0; j < i; j++) {
             if ( RefValid[j] && RefCounter[j] == RefCounter[i] ) {
                  return 0
             }
         }
         return 1
     }


    and valid_ref_frame_size checks resolution based validity as follows:

     valid_ref_frame_size( checkRes, slot ) {
         if ( !checkRes )
             return 1
         return ( 2 * FrameWidth >= RefFrameWidth[ slot ] &&
                  2 * FrameHeight >= RefFrameHeight[ slot ] &&
                  FrameWidth <= 16 * RefFrameWidth[ slot ] &&
                  FrameHeight <= 16 * RefFrameHeight[ slot ] )
     }


    The distinct reference frames are given a score as follows:

     NRanked = 0
     maxQ = 0
     minQ = 0
     for ( i = 0; i < NumRefFrames; i++ ) {
         d = mapOrderHint[i]



    AV2 Specification                                                                             Page 397 of 1169
      if (d != -1) {
          q = mapBaseQIdx[i]
          dispDiff = get_relative_dist( OrderHint, d )
          tDist = Abs(dispDiff) + obu_mlayer_id - RefMLayerId[i]
          if (maxDisp > OrderHint) {
              score = (tDist << DIST_WEIGHT_BITS) + q
          } else {
              score = Dist_Score_Lookup[Min(tDist, DECAY_DIST_CAP)] +
                       Max(tDist - DECAY_DIST_CAP, 0) + q
          }
          refRatio = FloorLog2( RefFrameWidth[ i ] * RefFrameHeight[ i ] )
          score -= refRatio << 5
          if (new_score_or_dist(d,score,RefMLayerId[i])) {
              ScoresIndex[NRanked] = i
              ScoresScore[NRanked] = score
              ScoresOrderHint[NRanked] = d
              ScoresDistance[NRanked] = dispDiff
              ScoresBaseQIdx[NRanked] = q
              ScoresLayer[NRanked] = RefMLayerId[i]
              if (NRanked == 0) {
                   minQ = q
                   maxQ = q
              } else {
                   minQ = Min(q,minQ)
                   maxQ = Max(q,maxQ)
              }
              NRanked += 1
          }
      }
 }


where Dist_Score_Lookup is defined as:

 Dist_Score_Lookup[ DECAY_DIST_CAP + 1 ] = {
     0, 64, 96, 112, 120, 124, 126,
 }


and the function new_score_or_dist (which returns 1 if we have found a new score or a new display order
hint) is given by:

 new_score_or_dist(d,score,mLayer) {
     for ( i = 0; i < NRanked; i++ ) {
         if ( ScoresOrderHint[i] == d &&
               ScoresScore[i] == score &&
               mLayer == ScoresLayer[i] ) {
              return 0
         }
     }
     return 1
 }


If too many references have been selected, a reference is dropped as follows:

 if (NRanked > REFS_PER_FRAME) {
     qThresh = (maxQ + minQ + 1) / 2
     unmappedIdx = get_unmapped_ref(qThresh)
     if (unmappedIdx >= 0) {
         ScoresScore[unmappedIdx] = 0x7fffffff
     }
 }




AV2 Specification                                                                         Page 398 of 1169
where get_unmapped_ref chooses the reference to drop as follows:

 get_unmapped_ref(qThresh) {
     nPast = 0
     nFuture = 0
     maxPastDistance = 0
     maxFutureDistance = 0
     pastIdx = 0
     futureIdx = 0
     for ( i = 0; i < NRanked; i++ ) {
         if (ScoresBaseQIdx[i] >= qThresh) {
             d = ScoresDistance[i]
             if (d > 0) {
                 if (d > maxPastDistance) {
                      maxPastDistance = d
                      pastIdx = i
                 }
                 nPast++
             } else if (d < 0) {
                 if (-d > maxFutureDistance) {
                      maxFutureDistance = -d
                      futureIdx = i
                 }
                 nFuture++
             }
         }
     }
     if (nPast > nFuture) {
         return pastIdx
     }
     if (nPast < nFuture) {
         return futureIdx
     }
     if (nPast > 0) {
         return maxPastDistance >= maxFutureDistance ? pastIdx : futureIdx
     }
     return -1
 }


The references are ranked and the values for ref_frame_idx are computed as follows:

 bubble_sort_ref_scores()
 NumTotalRefs = Min(NRanked,ActiveNumRefFrames)
 for (i = 0; i < NumTotalRefs; i++) {
     ref_frame_idx[ i ] = ScoresIndex[ i ]
 }


where the function bubble_sort_ref_scores (which sorts the references based on their score) is specified
as:

 bubble_sort_ref_scores( ) {
     for (i = NRanked - 1; i > 0; i--) {
         for (j = 0; j < i; j++) {
             if (ScoresScore[j] > ScoresScore[j + 1]) {
                 index = ScoresIndex[j]
                 score = ScoresScore[j]
                 displayOrder = ScoresOrderHint[j]
                 distance = ScoresDistance[j]
                 baseQIdx = ScoresBaseQIdx[j]

                    ScoresIndex[j] = ScoresIndex[j+1]
                    ScoresScore[j] = ScoresScore[j+1]
                    ScoresOrderHint[j] = ScoresOrderHint[j+1]
                    ScoresDistance[j] = ScoresDistance[j+1]
                    ScoresBaseQIdx[j] = ScoresBaseQIdx[j+1]



AV2 Specification                                                                           Page 399 of 1169
                        ScoresIndex[j+1] = index
                        ScoresScore[j+1] = score
                        ScoresOrderHint[j+1] = displayOrder
                        ScoresDistance[j+1] = distance
                        ScoresBaseQIdx[j+1] = baseQIdx
                    }
               }
          }
     }


    Finally, any remaining restricted frames are added at the end as follows:

     if ( checkRes && !IsBridge ) {
         for ( i = 0; i < NumRefFrames; i++ ) {
             if ( RefValid[ i ] && RefOrderHint[ i ] == RESTRICTED_OH &&
                  TLayerDependencyMap[obu_mlayer_id][obu_tlayer_id][RefTLayerId[i]] &&
                  MLayerDependencyMap[obu_mlayer_id][RefMLayerId[i]] &&
                  (AllowedFrames & (1 << i)) &&
                  NumTotalRefs < ActiveNumRefFrames ) {
                 ref_frame_idx[ NumTotalRefs ] = i
                 NumTotalRefs++
             }
         }
     }


```

<a id="s-7-8"></a>

### § 7.8 Get past future cur ref lists process

```text
§   7.8. Get past future cur ref lists process
    This process is triggered by a call to get_past_future_cur_ref_lists while reading the frame header info.

    The process chooses references to be used as follows:

     NumPastRefs = 0
     NumFutureRefs = 0
     numCurRefs = 0
     FurthestFuture = NONE
     ClosestPast = NONE
     ClosestFuture = NONE
     for (i = 0; i < NumTotalRefs; i++) {
         if ( RefOrderHint[ref_frame_idx[i]] != RESTRICTED_OH ) {
             if ( ScoresDistance[i] > 0 ) {
                 NumPastRefs++
                 if ( ClosestPast == NONE ||
                      ScoresDistance[i] < ScoresDistance[ClosestPast] ) {
                      ClosestPast = i
                 }
             } else if ( ScoresDistance[i] < 0 ) {
                 NumFutureRefs++
                 if ( FurthestFuture == NONE ||
                      RefOrderHint[ref_frame_idx[FurthestFuture]] <
                          RefOrderHint[ref_frame_idx[i]] ) {
                      FurthestFuture = i
                 }
                 if ( ClosestFuture == NONE ||
                      RefOrderHint[ref_frame_idx[i]] <
                          RefOrderHint[ref_frame_idx[ClosestFuture]] ) {
                      ClosestFuture = i
                 }
             } else {
                 curRefs[numCurRefs] = i
                 numCurRefs++
             }
         }
     }
     SkipSegFrame = numCurRefs > 0 ? curRefs[0] : ClosestPast



    AV2 Specification                                                                             Page 400 of 1169
     if ( SkipSegFrame == NONE ) {
         SkipSegFrame = 0
     }
     OrigClosestFuture = ClosestFuture
     OrigClosestPast = ClosestPast


```

<a id="s-7-9"></a>

### § 7.9 Motion field estimation process

```text
§   7.9. Motion field estimation process
```

<a id="s-7-9-1"></a>

#### § 7.9.1 General

```text
§   7.9.1. General

    This process is triggered by a call to motion_field_estimation while reading the frame header info.

    A linear projection model is employed to create a motion field estimation that is able to capture high
    velocity temporal motion trajectories.

    The motion field is estimated based on the saved motion vectors from the reference frames and the
    relative frame distances.

    As the frame distances depend on the frame being referenced, a separate motion field is estimated for
    each reference frame used by the current frame.

    A motion vector (for each reference frame type) is prepared at each location on an 8x8 luma sample grid.

    The variable w8 (representing the width of the motion field in units of 8x8 luma samples) is set equal to
    MiCols >> 1.


    The variable h8 (representing the height of the motion field in units of 8x8 luma samples) is set equal to
    MiRows >> 1.


    As the linear projection can create a field with holes, the motion fields are initialized to an invalid motion
    vector as follows:

     for ( y = 0; y < h8 ; y++ )
         for ( x = 0; x < w8; x++ ) {
             MotionFieldValid[ y ][ x ] = 0
             MotionFieldOffset[ y ][ x ] = 0
             for( src = 0; src < NumTotalRefs; src++ ) {
                 TrajValid[ src ][ y ][ x ] = 0
                 for(k=0;k<3;k++) {
                     TrajPosValid[ k ][ src ][ y ][ x ] = 0
                 }
             }
         }


    An array sortRef that gives the reference frames in sorted order (sorted by order hint) is computed as
    follows:

     for( i = 0 ; i < NumTotalRefs ; i++) {
         sortRef[i] = i
     }
     for( i = 0; i < NumTotalRefs ; i++ ) {
         for( j = i + 1 ; j < NumTotalRefs ; j++ ) {
             if ( get_relative_dist( OrderHints[ sortRef[ j ] ],
                                     OrderHints[ sortRef[ i ] ] ) < 0 ) {
                 tmp = sortRef[i]
                 sortRef[i] = sortRef[j]
                 sortRef[j] = tmp




    AV2 Specification                                                                               Page 401 of 1169
           }
      }
 }


A variable curIdx that specifies the index of the reference just before the current order hint is computed
as follows:

 curIdx = -1
 for( i = 0 ; i < NumTotalRefs ; i++ ) {
     if ( get_relative_dist( OrderHints[ sortRef[ i ] ], OrderHint ) < 0 ) {
         curIdx = i
     } else {
         break
     }
 }


The references are topologically sorted as follows:

 for ( rf = 0; rf < NumTotalRefs; rf++) {
     MotionFieldVisited[ rf ] = 0
     MotionFieldDepth[ rf ] = -1
     MotionFieldChecked[ rf ][ 0 ] = 0
     MotionFieldChecked[ rf ][ 1 ] = 0
 }
 MotionFieldStackCount = 0
 for ( rf = 0; rf < NumTotalRefs; rf++) {
     if ( OrderHints[ rf ] != RESTRICTED_OH ) {
         topo_sort_refs( rf )
     }
 }


Where topo_sort_refs is a recursive function specified as:

 topo_sort_refs( rf ) {
     if ( MotionFieldVisited[ rf ] ) {
         return
     }
     MotionFieldVisited[ rf ] = 1
     refIdx = ref_frame_idx[ rf ]
     if (RefFrameType[ refIdx ] == INTER_FRAME) {
         for( i = 0; i < RefNumTotalRefs[ refIdx ]; i++ ) {
             if ( SavedOrderHints[ refIdx ][ i ] != RESTRICTED_OH ) {
                 for( j = 0 ; j < NumTotalRefs ; j++) {
                     if ( OrderHints[ j ] == SavedOrderHints[ refIdx ][ i ] &&
                         !is_ref_overlay( j ) ) {
                         topo_sort_refs( j )
                         break
                     }
                 }
             }
         }
     }
     MotionFieldDepth[ rf ] = MotionFieldStackCount
     MotionFieldStack[ MotionFieldStackCount ] = rf
     MotionFieldStackCount++
 }

 is_ref_overlay( ref ) {
     refIdx = ref_frame_idx[ ref ]
     for (i = 0; i < RefNumTotalRefs[ refIdx ]; i++) {
         if (SavedOrderHints[ refIdx ][ i ] == RefOrderHint[ refIdx ]) {
             return 1
         }




AV2 Specification                                                                            Page 402 of 1169
      }
      return 0
 }


If MotionFieldStackCount is less than 2, the process immediately terminates.

The variable processCount (representing how many motion fields have to be projected) is set equal to 0.

The projections to do are recorded as follows:

 if ( enable_tip &&
       ( (NumFutureRefs > 0 && NumPastRefs > 0) || NumPastRefs >= 2 ) ) {
     past = sortRef[curIdx]
     if ( NumFutureRefs > 0 && NumPastRefs > 0) {
          future = sortRef[curIdx + 1]
     } else {
          future = sortRef[curIdx - 1]
     }
     if ( MotionFieldDepth[past] > MotionFieldDepth[future] ) {
          processCount = record_tip_projection( past, 1, future, processCount )
     } else {
          processCount = record_tip_projection( future, 0, past, processCount )
     }
     ClosestPast = past
     ClosestFuture = future
 } else {
     ClosestPast = NONE
     ClosestFuture = NONE
 }
 for( groupIdx = 0; groupIdx < 2 ; groupIdx++ ) {
     pastIdx = curIdx >= groupIdx ? curIdx - groupIdx : -1
     if (pastIdx >= 0 && !has_future_ref( sortRef[ pastIdx ] ))
          pastIdx = -1
     pastDist = pastIdx >= 0 ?
                     get_dist_to_closest_interp_ref(sortRef[pastIdx], 0) : -1
     futureIdx = curIdx < NumTotalRefs - groupIdx - 1 ?
                     curIdx + 1 + groupIdx : -1
     if (futureIdx >= 0 && !has_past_ref( sortRef[ futureIdx ] ))
          futureIdx = -1
     futureDist = futureIdx >= 0 ?
                       get_dist_to_closest_interp_ref(sortRef[futureIdx], 1) : -1
     if (futureDist < pastDist) {
          if (futureIdx >= 0) {
              processCount = record_projection( sortRef[futureIdx], 0,
                                                processCount )
          }
          if (pastIdx >= 0) {
              processCount = record_projection( sortRef[pastIdx], 1, processCount)
          }
     } else {
          if (pastIdx >= 0) {
              processCount = record_projection( sortRef[pastIdx], 1, processCount)
          }
          if (futureIdx >= 0) {
              processCount = record_projection( sortRef[futureIdx], 0,
                                                processCount )
          }
     }
 }

 if (curIdx >= 0) {
     processCount = record_projection( sortRef[curIdx], 0, processCount )
 }
 if (curIdx >= 1) {
     processCount = record_projection( sortRef[curIdx - 1], 0, processCount )
 }
 for ( ri = MotionFieldStackCount - 1; ri > 0 ; ri-- ) {
     ref = MotionFieldStack[ ri ]



AV2 Specification                                                                          Page 403 of 1169
      isBwd = OrderHints[ ref ] < OrderHint
      for( j = 0 ; j < 2 ; j++ ) {
          if (!MotionFieldChecked[ ref ][ isBwd ]) {
              processCount =
                  record_projection_with_type( 0, ref, isBwd, -1, MFMV_STACK_SIZE,
                                               processCount)
          }
          isBwd = !isBwd;
      }
 }


where the functions has_future_ref, has_past_ref, get_dist_to_closest_interp_ref,
is_ref_motion_field_eligible, is_ref_motion_field_eligible_by_frame_size,
is_ref_motion_field_eligible_by_frame_type, record_tip_projection, record_projection, and
record_projection_with_type are specified as:

 has_future_ref( ref ) {
     if ( OrderHints[ ref ] == RESTRICTED_OH ) {
         return 0
     }
     refIdx = ref_frame_idx[ ref ]
     for (i = 0; i < RefNumTotalRefs[ refIdx ]; i++) {
         if ( SavedOrderHints[ refIdx ][ i ] != RESTRICTED_OH &&
               SavedOrderHints[ refIdx ][ i ] > RefOrderHint[ refIdx ] ) {
              return 1
         }
     }
     return 0
 }

 has_past_ref( ref ) {
     if ( OrderHints[ ref ] == RESTRICTED_OH ) {
         return 0
     }
     refIdx = ref_frame_idx[ ref ]
     for (i = 0; i < RefNumTotalRefs[ refIdx ]; i++) {
         if ( SavedOrderHints[ refIdx ][ i ] != RESTRICTED_OH &&
               SavedOrderHints[ refIdx ][ i ] < RefOrderHint[ refIdx ]) {
              return 1
         }
     }
     return 0
 }

 get_dist_to_closest_interp_ref(startFrame, findForwardRef) {
     absClosestRefOffset = 0x7fffffff
     startIdx = ref_frame_idx[ startFrame ]
     if ( !is_ref_motion_field_eligible( startIdx ) ) {
         return 0x7fffffff
     }
     for (ref = 0; ref < RefNumTotalRefs[ startIdx ]; ref++) {
         if ( SavedOrderHints[ startIdx ][ ref ] != RESTRICTED_OH ) {
             refOffset = SavedOrderHints[ startIdx ][ ref ]
             startToRefOffset = get_relative_dist( OrderHints[startFrame],
                                                   refOffset)
             curToRefOffset = get_relative_dist( OrderHint, refOffset)
             absStartToRefOffset = Abs(startToRefOffset)
             isTwoSides = findForwardRef ?
                             (startToRefOffset > 0 && curToRefOffset > 0) :
                             (startToRefOffset < 0 && curToRefOffset < 0)
             if (isTwoSides) {
                 absClosestRefOffset = Min( absClosestRefOffset,
                                            absStartToRefOffset )
             }
         }
     }
     return absClosestRefOffset



AV2 Specification                                                                           Page 404 of 1169
 }

 is_ref_motion_field_eligible_by_frame_size( srcIdx ) {
     return RefFrameWidth[ srcIdx ] == FrameWidth &&
            RefFrameHeight[ srcIdx ] == FrameHeight
 }

 is_ref_motion_field_eligible_by_frame_type( srcIdx ) {
     return RefFrameType[ srcIdx ] != INTRA_ONLY_FRAME &&
            RefFrameType[ srcIdx ] != KEY_FRAME
 }

 is_ref_motion_field_eligible( srcIdx ) {
     return is_ref_motion_field_eligible_by_frame_type( srcIdx ) &&
            is_ref_motion_field_eligible_by_frame_size( srcIdx )
 }

 record_tip_projection(ref, isBwd, targetFrame, processCount) {
     return record_projection_with_type( 1, ref, isBwd, targetFrame,
                                         TIP_MFMV_STACK_SIZE, processCount )
 }

 record_projection(ref, isBwd, processCount) {
     return record_projection_with_type( 0, ref, isBwd, -1, TIP_MFMV_STACK_SIZE,
                                         processCount)
 }

 record_projection_with_type( doingTip, ref, isBwd, targetFrame, maxCheck,
                               processCount ) {
     refIdx = ref_frame_idx[ ref ]
     if ( !is_ref_motion_field_eligible( refIdx ) ) {
         return processCount
     }
     refToCur = get_relative_dist( OrderHints[ ref ], OrderHint )
     if ( Abs(refToCur) > MAX_FRAME_DISTANCE ) {
         return processCount
     }
     if ( use_bru ) {
         if ( bru_ref == ref || (doingTip && bru_ref == targetFrame) ) {
              return processCount
         }
     }
     if ( doingTip ) {
         isBwd = OrderHints[ref] < OrderHints[targetFrame]
     }
     if ( processCount >= maxCheck ||
           MotionFieldChecked[ ref ][ isBwd ] ) {
         return processCount
     }
     MotionFieldChecked[ ref ][ isBwd ] = 1
     c = processCount
     MotionFieldRef[ c ] = ref
     MotionFieldBwd[ c ] = isBwd
     MotionFieldTargetFrame[ c ] = targetFrame
     processCount++
     return processCount
 }


The recorded projections are processed as follows:


 if ( reduced_ref_frame_mvs_mode ) {
     processCount = Min( 1, processCount )
 }
 for( i = 0; i < processCount; i++) {
     ref = MotionFieldRef[ i ]
     isBwd = MotionFieldBwd[ i ]




AV2 Specification                                                                  Page 405 of 1169
          targetFrame = MotionFieldTargetFrame[ i ]
          projection(ref,isBwd ? -1 : 1, isBwd , targetFrame)
     }


    The function calls to projection indicate that the projection process specified in § 7.9.3 Projection process
    is invoked.

    If enable_mv_traj is equal to 1, the fill trajectory motion vector gap process specified in § 7.9.2 Fill
    trajectory motion vector gap process is invoked.

```

<a id="s-7-9-2"></a>

#### § 7.9.2 Fill trajectory motion vector gap process

```text
§   7.9.2. Fill trajectory motion vector gap process

    If ProjStep is not equal to 2, this process immediately terminates.

    Otherwise (ProjStep is equal to 2), this process fills in the gaps as follows:

     w8 = MiCols >> 1
     h8 = MiRows >> 1
     for( rf = 0; rf < NumTotalRefs ; rf++ ) {
         for ( y8 = 0; y8 < h8 ; y8 += 2 ) {
             for ( x8 = 0; x8 < w8; x8 += 2 ) {
                 fill_traj_mv(rf, y8, x8, 0, 1)
                 fill_traj_mv(rf, y8, x8, 1, 0)
                 fill_traj_mv(rf, y8, x8, 1, 1)
             }
         }
     }


    where the function fill_traj_mv (which fills a specific position) is defined as:

     fill_traj_mv( rf, y8, x8, dy, dx) {
         w8 = MiCols >> 1
         h8 = MiRows >> 1
         if ( !TrajValid[ rf ][ y8 ][ x8 ] || y8 + dy == h8 || x8 + dx == w8 ) {
             return
         }
         count = 1
         avgMv = TrajMv[ rf ][ y8 ][ x8 ]
         rAvail = dx > 0 && tmvp_avail(x8, x8 + 2, w8) &&
                   TrajValid[ rf ][ y8 ][ x8 + 2 ]
         bAvail = dy > 0 && tmvp_avail(y8, y8 + 2, h8) &&
                   TrajValid[ rf ][ y8 + 2 ][ x8 ]
         brAvail = dy > 0 && dx > 0 && tmvp_avail(x8, x8 + 2, w8) &&
                    tmvp_avail(y8, y8 + 2, h8) && TrajValid[ rf ][ y8 + 2 ][ x8 + 2 ]
         if (rAvail) {
             count++
             for( c = 0 ; c < 2; c++ ) {
                 avgMv[ c ] += TrajMv[ rf ][ y8 ][ x8 + 2 ][ c ]
             }
         }
         if (bAvail) {
             count++
             for( c = 0 ; c < 2; c++ ) {
                 avgMv[ c ] += TrajMv[ rf ][ y8 + 2 ][ x8 ][ c ]
             }
         }
         if (brAvail) {
             count++
             for( c = 0 ; c < 2; c++ ) {
                 avgMv[ c ] += TrajMv[ rf ][ y8 + 2 ][ x8 + 2 ][ c ]
             }
         }
         for( c = 0 ; c < 2; c++ ) {




    AV2 Specification                                                                                Page 406 of 1169
              TrajMv[ rf ][ y8 + dy ][ x8 + dx ][ c ] = calc_avg(avgMv[ c ], count)
          }
          TrajValid[ rf ][ y8 + dy ][ x8 + dx ] = 1
     }


    The get_tmvp_shift function (which specifies the right shift required to convert from a position in terms of
    multiples of 8 pixels to a position in terms of TMVP units) is specified as:

     get_tmvp_shift() {
         if ( SbSize == BLOCK_64X64 || ProjStep == 1 ) {
             return 3
         } else {
             return 4
         }
     }



      NOTE:       TMVP units are either 64 by 64 (a shift of 3), or 128 by 128 pixels in size (a shift of 4).


    The get_tmvp_unit function (which converts the position from a multiple of 8 pixels to the TMVP unit) is
    specified as:

     get_tmvp_unit( x8 ) {
         return x8 >> get_tmvp_shift()
     }


    The get_tmvp_phase function (which specifies the phase of the given TMVP unit) is specified as:

     get_tmvp_phase( x8 ) {
         return get_tmvp_unit( x8 ) % 3
     }



      NOTE: The TMVP is designed so that all the computation for a TMVP unit depends only on the
      TMVP unit and its left and right neighbors, and that the computation can happen in parallel. The
      phase is used to ensure that the computations are kept separate.


    The tmvp_avail function (which checks that two positions are in the same TMVP unit) is specified as:

     tmvp_avail( base8, loc8, max8 ) {
         if ( loc8 >= max8 ) {
             return 0
         }
         return get_tmvp_unit( base8 ) == get_tmvp_unit( loc8 )
     }


```

<a id="s-7-9-3"></a>

#### § 7.9.3 Projection process

```text
§   7.9.3. Projection process

    The inputs to this process are:

      • a variable src specifying which reference frame’s motion vectors are projected,
      • a variable dstSign specifying a negation multiplier for the motion vector direction,
      • a variable isBwd specifying if the reference frame has a higher order hint,
      • a variable targetFrame specifying the target frame (or -1 in some cases).


    AV2 Specification                                                                                  Page 407 of 1169
The process projects the motion vectors from a whole reference frame and stores the results in
MotionFieldMvs.

The variable srcIdx (representing which reference frame is used) is set equal to ref_frame_idx[ src ].

The variable refToCur is set equal to get_relative_dist( OrderHints[ src ], OrderHint ).

The array startRefMap (that will be used during the tracking of motion vector trajectories) is computed
as follows:

 for( k = 0 ; k < RefNumTotalRefs[ srcIdx ] ; k++ ) {
     startRefMap[ k ] = NONE
     for( rf = 0; rf < NumTotalRefs; rf++ ) {
         if ( SavedOrderHints[ srcIdx ][ k ] == OrderHints[ rf ] &&
              OrderHints[ rf ] != OrderHints[ src ] &&
              !( SavedOrderHints[ srcIdx ][ k ] == RESTRICTED_OH ||
                 OrderHints[ rf ] == RESTRICTED_OH ) ) {
             startRefMap[ k ] = rf
             break
         }
     }
 }


The variable w8 (representing the width of the motion field in units of 8x8 luma samples) is set equal to
MiCols >> 1.


The variable h8 (representing the height of the motion field in units of 8x8 luma samples) is set equal to
MiRows >> 1.


The process is specified as follows:

 for ( y8 = 0; y8 < h8; y8 += ProjStep ) {
     for ( x8 = 0; x8 < w8; x8 += ProjStep ) {
         list = isBwd
         srcRef = SavedRefFrames[ srcIdx ][ y8 ][ x8 ][ list ]
         if ( is_inter_ref_frame( srcRef ) ) {
             if ( enable_mv_traj ) {
                 mv2[ 0 ] = uncompression_mv(
                                  SavedMvs[ srcIdx ][ y8 ][ x8 ][ list ][ 0 ] )
                 mv2[ 1 ] = uncompression_mv(
                                  SavedMvs[ srcIdx ][ y8 ][ x8 ][ list ][ 1 ] )
                 check_traj_intersect(src, x8, y8, startRefMap[srcRef], mv2)
             }
             refOffset = get_relative_dist( OrderHints[ src ],
                                             SavedOrderHints[ srcIdx ][ srcRef ] )
             if ( SavedOrderHints[ srcIdx ][ srcRef ] == RESTRICTED_OH ) {
                 refOffset = 0
             }
             posValid = Abs( refOffset ) <= MAX_FRAME_DISTANCE
             if (isBwd) {
                 refOffset = -refOffset
             }
             if ( posValid && refOffset >= 0 ) {
                 mv = SavedMvs[ srcIdx ][ y8 ][ x8 ][ list ]
                 mv[ 0 ] = uncompression_mv( mv[ 0 ] )
                 mv[ 1 ] = uncompression_mv( mv[ 1 ] )
                 projMv = get_mv_projection( mv, refToCur * dstSign, refOffset )
                 if (isBwd) {
                      (posValid,posX8,posY8) = get_sampled_position( x8, y8, 1,
                                                                     projMv )
                 } else {
                      (posValid,posX8,posY8) = get_sampled_position( x8, y8,
                                                                     dstSign,



AV2 Specification                                                                             Page 408 of 1169
                                                                         projMv )
                    }
                    posValid = check_block_position(posValid, x8, y8, posX8, posY8)
                    if ( posValid && ( !MotionFieldValid[ posY8 ][ posX8 ] ||
                          ( targetFrame != -1 &&
                             targetFrame == startRefMap[srcRef] &&
                             MotionFieldOffset[ posY8 ][ posX8 ] != refOffset )
                       ) ) {
                        if ( enable_mv_traj ) {
                             k = get_tmvp_phase( posX8 )
                             TrajPos[k][src][y8][x8][0] = posY8
                             TrajPos[k][src][y8][x8][1] = posX8
                             TrajPosValid[k][src][y8][x8] = 1
                             for(c=0;c<2;c++) {
                                 TrajMv[src][posY8][posX8][c] =
                                     Clip3( -REFMVS_LIMIT, REFMVS_LIMIT, -projMv[c] )
                             }
                             TrajValid[src][posY8][posX8] = 1
                             endFrame = startRefMap[srcRef]
                             if (endFrame != NONE) {
                                 projMv = get_mv_projection( mv,
                                               refOffset - refToCur * dstSign,
                                               refOffset )
                                 for(c=0;c<2;c++) {
                                     TrajMv[endFrame][posY8][posX8][c] =
                                         Clip3( -REFMVS_LIMIT, REFMVS_LIMIT,
                                                 projMv[c] )
                                 }
                                 TrajValid[endFrame][posY8][posX8] = 1
                                 (targetValid, targetX8, targetY8) =
                                     get_sampled_position( x8, y8, 1, mv)
                                 targetValid = check_block_position( targetValid,
                                                                      targetX8,
                                                                      targetY8,
                                                                      posX8, posY8 )
                                 if (targetValid) {
                                     TrajPos[k][endFrame][targetY8][targetX8][0] =
                                         posY8
                                     TrajPos[k][endFrame][targetY8][targetX8][1] =
                                         posX8
                                     TrajPosValid[k][endFrame][targetY8][targetX8]=1
                                 }
                             }
                        }
                        if (isBwd) {
                             mv[ 0 ] = -mv[ 0 ]
                             mv[ 1 ] = -mv[ 1 ]
                        }
                        MotionFieldValid[ posY8 ][ posX8 ] = 1
                        MotionFieldMvs[ posY8 ][ posX8 ] = mv
                        MotionFieldOffset[ posY8 ][ posX8 ] = refOffset
                    }
                }
           }
      }
 }


When the function get_mv_projection is called, the get mv projection process specified in § 7.9.4 Get MV
projection process is invoked and the output assigned to projMv.

When the function get_sampled_position is called, the get sampled position process specified in § 7.9.6
Get sampled position process is invoked and the outputs are assigned to posValid, posX8, and posY8.




AV2 Specification                                                                           Page 409 of 1169
The function check_traj_intersect (which tries to extend a motion vector trajectory) is specified as:

 check_traj_intersect(srcFrame, x8, y8, endFrame, mv) {
     if (endFrame == NONE) {
         return
     }
     for( k = 0; k < 3; k++ ) {
         if ( TrajPosValid[ k ][ srcFrame ][ y8 ][ x8 ] != 0 ) {
             trajY8 = TrajPos[ k ][ srcFrame ][ y8 ][ x8 ][ 0 ]
             trajX8 = TrajPos[ k ][ srcFrame ][ y8 ][ x8 ][ 1 ]
             if ( !TrajValid[ endFrame ][ trajY8 ][ trajX8 ] ) {
                 for( c = 0; c < 2; c++ ) {
                      v = TrajMv[ srcFrame ][ trajY8 ][ trajX8 ][ c ] + mv[ c ]
                      TrajMv[ endFrame ][ trajY8 ][ trajX8 ][ c ] =
                          Clip3(-REFMVS_LIMIT, REFMVS_LIMIT, v)
                 }
                 TrajValid[ endFrame ][ trajY8 ][ trajX8 ] = 1
                 (posValid,posX8,posY8) = get_sampled_position(
                      trajX8, trajY8, 1, TrajMv[ endFrame ][ trajY8 ][ trajX8 ] )
                 posValid = check_block_position( posValid, posX8, posY8, trajX8,
                                                   trajY8)
                 if (posValid) {
                      TrajPos[ k ][ endFrame ][ posY8 ][ posX8 ][ 0 ] = trajY8
                      TrajPos[ k ][ endFrame ][ posY8 ][ posX8 ][ 1 ] = trajX8
                      TrajPosValid[ k ][ endFrame ][ posY8 ][ posX8 ] = 1
                 }
             }
         }
     }
     (posValid,endX8,endY8) = get_sampled_position( x8, y8, 1, mv )
     if (!posValid) {
         return
     }
     for( k = 0; k < 3; k++ ) {
         if ( TrajPosValid[ k ][ endFrame ][ endY8 ][ endX8 ] != 0 ) {
             trajY8 = TrajPos[ k ][ endFrame ][ endY8 ][ endX8 ][ 0 ]
             trajX8 = TrajPos[ k ][ endFrame ][ endY8 ][ endX8 ][ 1 ]
             if ( check_block_position(1, x8, y8, trajX8, trajY8) &&
                   !TrajValid[ srcFrame ][ trajY8 ][ trajX8 ] ) {
                 for( c = 0; c < 2; c++ ) {
                      v = TrajMv[ endFrame ][ trajY8 ][ trajX8 ][ c ] - mv[ c ]
                      TrajMv[ srcFrame ][ trajY8 ][ trajX8 ][ c ] =
                          Clip3(-REFMVS_LIMIT, REFMVS_LIMIT, v)
                 }
                 TrajValid[ srcFrame ][ trajY8 ][ trajX8 ] = 1
                 (posValid,posX8,posY8) = get_sampled_position(
                      trajX8, trajY8, 1, TrajMv[ srcFrame ][ trajY8 ][ trajX8 ] )
                 posValid = check_block_position( posValid, posX8, posY8, trajX8,
                                                   trajY8)
                 if (posValid) {
                      TrajPos[ k ][ srcFrame ][ posY8 ][ posX8 ][ 0 ] = trajY8
                      TrajPos[ k ][ srcFrame ][ posY8 ][ posX8 ][ 1 ] = trajX8
                      TrajPosValid[ k ][ srcFrame ][ posY8 ][ posX8 ] = 1
                 }
             }
         }
     }
 }


The function calls to check_block_position indicate that the check block position process specified in
§ 7.9.8 Check block position process is invoked.




AV2 Specification                                                                             Page 410 of 1169
```

<a id="s-7-9-4"></a>

#### § 7.9.4 Get MV projection process

```text
§   7.9.4. Get MV projection process

    The inputs to this process are:

      • a length 2 array mv specifying a motion vector,
      • a variable numerator specifying the number of frames to be covered by the projected motion vector,
      • a variable denominator specifying the number of frames covered by the original motion vector.

    The outputs of this process are:

      • a length 2 array projMv containing the projected motion vector.

    This process starts with a motion vector mv. This motion vector gives the displacement expected when
    moving a certain number of frames (given by the variable denominator). In order to use the motion vector
    for predictions using a different reference frame, the length of the motion vector must be scaled.

    The variable clippedDenominator is set equal to Min( MAX_FRAME_DISTANCE, denominator ).

    The variable clippedNumerator is set equal to Clip3( -MAX_FRAME_DISTANCE,
    MAX_FRAME_DISTANCE, numerator ).

    The projected motion vector is specified as follows:

     for ( i = 0; i < 2; i++ ) {
         scaled = Round2Signed( mv[ i ] * clippedNumerator *
                                 Div_Mult[ clippedDenominator ], 14 )
         projMv[ i ] = Clip3( MV_LOW + 1, MV_UPP - 1, scaled )
     }


    where Div_Mult is a constant lookup table specified as:

     Div_Mult[32] = {
       0,    16384, 8192, 5461, 4096, 3276, 2730, 2340, 2048, 1820, 1638,
       1489, 1365, 1260, 1170, 1092, 1024, 963, 910, 862, 819, 780,
       744, 712,    682, 655, 630, 606, 585, 564, 546, 528
     }


```

<a id="s-7-9-5"></a>

#### § 7.9.5 Get MV projection clamp process

```text
§   7.9.5. Get MV projection clamp process

    The inputs to this process are:

      • a length 2 array mv specifying a motion vector,
      • a variable numerator specifying the number of frames to be covered by the projected motion vector,
      • a variable denominator specifying the number of frames covered by the original motion vector.

    The outputs of this process are:

      • a length 2 array projMv containing the projected motion vector.

    The get mv projection process specified in § 7.9.4 Get MV projection process is invoked with mv,
    numerator, and denominator as inputs, and the output is assigned to projMv.




    AV2 Specification                                                                           Page 411 of 1169
    The projected motion vector is clamped to a tighter range as follows:

     for ( i = 0; i < 2; i++ ) {
         projMv[ i ] = Clip3( -REFMVS_LIMIT, REFMVS_LIMIT, projMv[ i ] )
     }


```

<a id="s-7-9-6"></a>

#### § 7.9.6 Get sampled position process

```text
§   7.9.6. Get sampled position process

    The inputs to this process are:

      • variables x8 and y8 specifying a location in units of 8x8 luma samples,
      • a variable dstSign specifying a negation multiplier for the motion vector direction,
      • a length 2 array projMv specifying a projected motion vector.

    The get block position no constraint process specified in § 7.9.7 Get block position no constraint process
    is invoked with x8, y8, dstSign, and projMv as inputs, and the outputs are assigned to posValid, posX8,
    and posY8.

    If ProjStep is equal to 2, the position is changed to an even location as follows:

     posX8 -= posX8 & 1
     posY8 -= posY8 & 1


    The outputs of this process are the variables posValid, posX8, and posY8.

```

<a id="s-7-9-7"></a>

#### § 7.9.7 Get block position no constraint process

```text
§   7.9.7. Get block position no constraint process

    The inputs to this process are:

      • variables x8 and y8 specifying a location in units of 8x8 luma samples,
      • a variable dstSign specifying a negation multiplier for the motion vector direction,
      • a length 2 array projMv specifying a projected motion vector.

    The process returns a flag posValid that indicates if the position is to be used and variables posX8 and
    posY8 representing the projected location in units of 8x8 luma samples.


      NOTE:       This function does not check the constraints of being close to the current TMVP unit.


    The variable posValid is set equal to 1.

    The variable posY8 is set equal to project_no_constraint(y8, projMv[ 0 ], dstSign, MiRows >> 1).

    The variable posX8 is set equal to project_no_constraint(x8, projMv[ 1 ], dstSign, MiCols >> 1).

    where the function project_no_constraint is specified as follows:

     project_no_constraint( v8, delta, dstSign, max8 ) {
         if ( delta >= 0 ) {
             offset8 = delta >> ( 3 + 1 + MI_SIZE_LOG2 )
         } else {
             offset8 = -( ( -delta ) >> ( 3 + 1 + MI_SIZE_LOG2 ) )
         }



    AV2 Specification                                                                                  Page 412 of 1169
          v8 += dstSign * offset8
          if ( v8 < 0 ||
               v8 >= max8 ) {
              posValid = 0
          }
          return v8
     }


    The project_no_constraint function clears posValid if the resulting position is offset too far.

    The outputs of this process are the variables posValid, posX8, and posY8.

```

<a id="s-7-9-8"></a>

#### § 7.9.8 Check block position process

```text
§   7.9.8. Check block position process

    The inputs to this process are:

      • a variable posValid specifying if the location is valid according to previous checks,
      • variables posX8 and posY8 specifying a location to be checked in units of 8x8 luma samples,
      • variables baseX8 and baseY8 specifying a base location in units of 8x8 luma samples.

    The output of this process is the variable posValid that indicates if the checked position is sufficiently
    close to the base position.

    If posValid is equal to 0, the process terminates immediately with 0 as output.

    Otherwise, the position is checked as follows:

     shift = get_tmvp_shift()
     for (ord = 0; ord < 2; ord++) {
         v8 = ord ? baseX8 : baseY8
         sbOff8 = 1 << shift
         if ( ProjStep > 1 ) {
              maxOff8 = ord ? sbOff8 : 0
         } else {
              maxOff8 = ord ? sbOff8 >> 1 : 0
         }
         base8 = (v8 >> shift) << shift
         pos8 = ord ? posX8 : posY8
         if (pos8 < base8 - maxOff8 ||
              pos8 >= base8 + sbOff8 + maxOff8 ) {
              return 0
         }
     }
     return 1


```

<a id="s-7-10"></a>

### § 7.10 Setup TIP motion field process

```text
§   7.10. Setup TIP motion field process
```

<a id="s-7-10-1"></a>

#### § 7.10.1 General

```text
§   7.10.1. General

    This process is triggered by a call to setup_tip_motion_field while reading the frame header info.

    The estimated motion field is temporally scaled based on the frames chosen for TIP, and the TIP frame is
    constructed if TipFrameMode is equal to TIP_FRAME_AS_OUTPUT.

    It is a requirement of bitstream conformance that all the following conditions are true whenever this
    process is triggered:

      • The FrameType is not equal to SWITCH_FRAME (indicating that the frame is not a switch frame),


    AV2 Specification                                                                                 Page 413 of 1169
      • ClosestPast is not equal to NONE,
      • ClosestFuture is not equal to NONE,
      • TipFrameMode is not equal to TIP_FRAME_DISABLED,
      • use_ref_frame_mvs is equal to 1,
      • HasBothRefs is equal to 1 or NumPastRefs is greater than or equal to 2,
      • is_ref_motion_field_eligible_by_frame_type(ref_frame_idx[ClosestPast]) is true or
        is_ref_motion_field_eligible_by_frame_type(ref_frame_idx[ClosestFuture]) is true,
      • is_ref_motion_field_eligible_by_frame_size(ref_frame_idx[ClosestPast]) is true and
        is_ref_motion_field_eligible_by_frame_size(ref_frame_idx[ClosestFuture]) is true.

    The following ordered steps now apply:

     1. The TIP temporal scale motion field process specified in § 7.10.2 TIP temporal scale motion field
        process is invoked.
     2. If allow_tip_hole_fill is equal to 1, the following ordered steps apply:

          1. The TIP fill motion field holes process specified in § 7.10.3 TIP fill motion field holes process is
             invoked.
          2. The TIP block average filter motion vector process specified in § 7.10.4 TIP block average filter
             motion vector process is invoked.
     3. The fill temporal motion vectors sample gap process specified in § 7.10.5 Fill temporal motion vectors
        sample gap process is invoked.
     4. If TipFrameMode is equal to TIP_FRAME_AS_OUTPUT, the build TIP process specified in § 7.10.6
        Build TIP process is invoked.

```

<a id="s-7-10-2"></a>

#### § 7.10.2 TIP temporal scale motion field process

```text
§   7.10.2. TIP temporal scale motion field process

    The variable refOffset is set as follows:

     (refOffset, _, _) = get_tip_offsets()


    The variable w8 is set equal to MiCols >> 1.

    The variable h8 is set equal to MiRows >> 1.

    The motion field is scaled as follows:

     for ( y8 = 0; y8 < h8 ; y8 += ProjStep ) {
         for ( x8 = 0; x8 < w8; x8 += ProjStep ) {
             mv = MotionFieldMvs[ y8 ][ x8 ]
             if ( MotionFieldValid[ y8 ][ x8 ] ) {
                 startOffset = MotionFieldOffset[ y8 ][ x8 ]
                 MotionFieldMvs[ y8 ][ x8 ] = get_mv_projection_clamp( mv, refOffset,
                                                                       startOffset )
                 MotionFieldValid[ y8 ][ x8 ] = 1
             }
             MotionFieldOffset[ y8 ][ x8 ] = refOffset
         }
     }




    AV2 Specification                                                                                 Page 414 of 1169
    When the function get_mv_projection_clamp is called, the get mv projection clamp process specified in
    § 7.9.5 Get MV projection clamp process is invoked.

```

<a id="s-7-10-3"></a>

#### § 7.10.3 TIP fill motion field holes process

```text
§   7.10.3. TIP fill motion field holes process

    This process fills in holes in the motion field.

    The filling is constrained to only look at locations within the same superblock (or within the same 128 by
    128 block if superblocks are 256 by 256).

    The motion vector filling is applied as follows:

     step = ProjStep
     sbSize8 = 1 << get_tmvp_shift()
     w8 = MiCols >> 1
     h8 = MiRows >> 1
     for ( y8 = 0; y8 < h8 ; y8 += sbSize8) {
         for ( x8 = 0; x8 < w8; x8 += sbSize8 ) {
             endRow8 = Min(y8 + sbSize8, h8)
             endCol8 = Min(x8 + sbSize8, w8)
             for( row8 = y8; row8 < endRow8; row8 += step ) {
                 for( col8 = x8; col8 < endCol8; col8 += step ) {
                      for( dir = 0; dir < 4; dir++) {
                          dstRow8 = row8 + Tip_Dirs[ dir ][ 0 ] * step
                          dstCol8 = col8 + Tip_Dirs[ dir ][ 1 ] * step
                          if ( dstRow8 >= y8 && dstRow8 < endRow8 &&
                                  dstCol8 >= x8 && dstCol8 < endCol8 &&
                                  !MotionFieldValid[dstRow8][dstCol8] ) {
                              MotionFieldValid[ dstRow8 ][ dstCol8 ] =
                                  MotionFieldValid[ row8 ][ col8 ]
                              for ( j = 0; j < 2; j++ )
                                  MotionFieldMvs[ dstRow8 ][ dstCol8 ][ j ] =
                                      MotionFieldMvs[ row8 ][ col8 ][ j ]
                              MotionFieldOffset[ dstRow8 ][ dstCol8 ] =
                                  MotionFieldOffset[ row8 ][ col8 ]
                          }
                      }
                 }
             }
         }
     }


    where the constant table Tip_Dirs is specified as:

     Tip_Dirs[ 5 ][ 2 ] = {
         { -1, 0 }, { 0, -1 }, { 1, 0 }, { 0, 1 }, { 0, 0 }
     }


```

<a id="s-7-10-4"></a>

#### § 7.10.4 TIP block average filter motion vector process

```text
§   7.10.4. TIP block average filter motion vector process

    This process smooths the motion field by averaging motion vectors.

    The averaging is constrained to only look at locations within the same superblock (or within the same 128
    by 128 block if superblocks are 256 by 256).

    The motion vectors are averaged and applied as follows:

     step = ProjStep
     sbSize8 = 1 << get_tmvp_shift()
     w8 = MiCols >> 1



    AV2 Specification                                                                            Page 415 of 1169
     h8 = MiRows >> 1
     for ( y8 = 0; y8 < h8 ; y8 += sbSize8) {
         for ( x8 = 0; x8 < w8; x8 += sbSize8 ) {
             endRow8 = Min(y8 + sbSize8, h8)
             endCol8 = Min(x8 + sbSize8, w8)
             for( row8 = y8; row8 < endRow8; row8 += step ) {
                 for( col8 = x8; col8 < endCol8; col8 += step ) {
                      mv[0] = 0
                      mv[1] = 0
                      count = 0
                      for( dir = 0; dir < 5; dir++) {
                          dstRow8 = row8 + Tip_Dirs[ dir ][ 0 ] * step
                          dstCol8 = col8 + Tip_Dirs[ dir ][ 1 ] * step
                          if ( dstRow8 >= y8 && dstRow8 < endRow8 &&
                                   dstCol8 >= x8 && dstCol8 < endCol8 &&
                                   MotionFieldValid[dstRow8][dstCol8] ) {
                               for ( j = 0; j < 2; j++ )
                                   mv[j] += MotionFieldMvs[ dstRow8 ][ dstCol8 ][ j ]
                               count += 1
                          }
                      }
                      if (count == 0) {
                          avgValid[ row8 ][ col8 ] = 0
                          avgMotionFieldMvs[ row8 ][ col8 ][ 0 ] = -(1<<15)
                          avgMotionFieldMvs[ row8 ][ col8 ][ 1 ] = -(1<<15)
                      } else {
                          avgValid[ row8 ][ col8 ] = 1
                          for(j=0;j<2;j++) {
                               avgMotionFieldMvs[ row8 ][ col8 ][ j ] =
                                   Round2Signed( mv[j] * Weight_Div_Mult[count], 16 )
                          }
                      }
                 }
             }
         }
     }
     for ( y8 = 0; y8 < h8 ; y8 += step ) {
         for ( x8 = 0; x8 < w8; x8 += step ) {
             for (comp = 0; comp < 2; comp++) {
                 MotionFieldMvs[ y8 ][ x8 ][ comp ] =
                      avgMotionFieldMvs[ y8 ][ x8 ][ comp ]
             }
             MotionFieldValid[ y8 ][ x8 ] = avgValid[ y8 ][ x8 ]
         }
     }


    where the constant table Weight_Div_Mult is specified as:

     Weight_Div_Mult[6] = {
         0, 65536, 32768, 21845, 16384, 13107
     }



      NOTE:       Multiplication by an entry in Weight_Div_Mult approximates a division by the value of the
      index.

```

<a id="s-7-10-5"></a>

#### § 7.10.5 Fill temporal motion vectors sample gap process

```text
§   7.10.5. Fill temporal motion vectors sample gap process

    At this stage the motion field is defined with a sampling step of ProjStep 8x8s.

    This process fills in gaps so that the motion field is defined at every location.

    If ProjStep is not equal to 2, this process terminates immediately.




    AV2 Specification                                                                             Page 416 of 1169
Otherwise, the gaps are filled in as follows:

 w8 = MiCols >> 1
 h8 = MiRows >> 1
 for ( y8 = 0; y8 < h8 ; y8 += 2 ) {
     for ( x8 = 0; x8 < w8; x8 += 2 ) {
         fill_tpl(y8, x8, 0, 1)
         fill_tpl(y8, x8, 1, 0)
         fill_tpl(y8, x8, 1, 1)
     }
 }


where the fill_tpl function fills in a single gap as follows:

 fill_tpl( y8, x8, dy, dx) {
     w8 = MiCols >> 1
     h8 = MiRows >> 1
     if ( !MotionFieldValid[ y8 ][ x8 ] || y8 + dy == h8 || x8 + dx == w8) {
         return
     }
     curOffset = MotionFieldOffset[ y8 ][ x8 ]
     count = 0
     for (c = 0; c < 2; c++) {
         avgMv[ c ] = 0
     }
     for (i = 0; i < 4; i++) {
         candX = i & 1
         candY = i >> 1
         available = ( dy >= candY &&
                        dx >= candX &&
                        tmvp_avail( x8, x8 + 2 * candX, w8 ) &&
                        tmvp_avail( y8, y8 + 2 * candY, h8 ) &&
                        MotionFieldValid[ y8 + 2 * candY ][ x8 + 2 * candX ] )
         if (available) {
             count++
             if ( i == 0 ) {
                 projMv = MotionFieldMvs[ y8 + 2 * candY ][ x8 + 2 * candX ]
             } else {
                 projMv = get_mv_projection_clamp(
                      MotionFieldMvs[ y8 + 2 * candY ][ x8 + 2 * candX ],
                      curOffset,
                      MotionFieldOffset[ y8 + 2 * candY ][ x8 + 2 * candX ] )
             }
             for (c = 0; c < 2; c++) {
                 avgMv[ c ] += projMv[ c ]
             }
         }
     }
     MotionFieldOffset[ y8 + dy ][ x8 + dx ] = curOffset
     for( c = 0 ; c < 2; c++ ) {
         MotionFieldMvs[ y8 + dy ][ x8 + dx ][ c ] = calc_avg (avgMv[ c ], count)
     }
     MotionFieldValid[ y8 + dy ][ x8 + dx ] = 1
 }


The function calc_avg performs approximate division with rounding as follows:

 calc_avg(n, d) {
     if ( d == 1 ) {
         return n
     } else if ( d == 2 ) {
         return Round2Signed( n, 1 )
     } else if ( d == 3 ) {
         return Round2Signed( n * 85, 8 )
     } else {



AV2 Specification                                                                   Page 417 of 1169
               return Round2Signed( n, 2 )
          }
     }


    When the function get_mv_projection_clamp is called, the get mv projection clamp process specified in
    § 7.9.5 Get MV projection clamp process is invoked.

```

<a id="s-7-10-6"></a>

#### § 7.10.6 Build TIP process

```text
§   7.10.6. Build TIP process

    This process builds samples in the current frame out of 8 by 8 blocks coded in TIP mode as follows:

     RefFrame[0] = ClosestPast
     RefFrame[1] = ClosestFuture
     motion_mode = SIMPLE
     use_bawp = 0
     compound_type = COMPOUND_AVERAGE
     CwpIdx = Tip_Weighting_Factor[ tip_global_wtd_index ]
     YMode = NEWMV
     use_intrabc = 0
     use_optflow = opfl_refine_type != REFINE_NONE &&
                    TipInterpFilter == EIGHTTAP_SHARP && enable_tip_refinemv
     DecidedAgainstRefinemv = 0
     (refOffset, pastOffset, futureOffset) = get_tip_offsets()
     tipSize = ( enable_tip_refinemv &&
                  TipInterpFilter == EIGHTTAP_SHARP ) ? BLOCK_8X8 : BLOCK_16X16
     storeRefinedMvs = store_refined_mvs()
     use_refinemv = 0
     for( row = 0; row < MiRows; row += Num_4x4_Blocks_High[tipSize] ) {
         for( col = 0; col < MiCols; col += Num_4x4_Blocks_Wide[tipSize] ) {
             for (i = 0; i < Num_4x4_Blocks_High[tipSize]; i++) {
                  for (j = 0; j < Num_4x4_Blocks_Wide[tipSize]; j++) {
                      RefFrames[row+i][col+j][0] = ClosestPast
                      RefFrames[row+i][col+j][1] = ClosestFuture
                  }
             }
             y8 = row >> 1
             x8 = col >> 1
             if ( !MotionFieldValid[ y8 ][ x8 ] ) {
                  localMvs[ 0 ][ 0 ] = 0
                  localMvs[ 0 ][ 1 ] = 0
                  localMvs[ 1 ][ 0 ] = 0
                  localMvs[ 1 ][ 1 ] = 0
             } else {
                  localMvs[ 0 ] = get_mv_projection( MotionFieldMvs[ y8 ][ x8 ],
                                                      pastOffset, refOffset )
                  localMvs[ 1 ] = get_mv_projection( MotionFieldMvs[ y8 ][ x8 ],
                                                      futureOffset, refOffset )
             }
             for (comp = 0; comp < 2; comp++) {
                  BlockMvs[ 0 ][ comp ] = localMvs[ 0 ][ comp ] + TipGlobalMv[ comp ]
                  BlockMvs[ 1 ][ comp ] = localMvs[ 1 ][ comp ] + TipGlobalMv[ comp ]
             }
             for (i = 0; i < Num_4x4_Blocks_High[tipSize]; i++) {
                  for (j = 0; j < Num_4x4_Blocks_Wide[tipSize]; j++) {
                      for (list = 0; list < 2; list++) {
                           Mvs[ row + i ][ col + j ][ list ] = BlockMvs[ list ]
                      }
                  }
             }
             for( plane = 0; plane < NumPlanes; plane++ ) {
                  if (plane == 0) {
                      subX = 0
                      subY = 0
                  } else {
                      subX = SubsamplingX
                      subY = SubsamplingY
                  }



    AV2 Specification                                                                          Page 418 of 1169
                    bw = Block_Width[ tipSize ] >> subX
                    bh = Block_Height[ tipSize ] >> subY
                    x = (col * MI_SIZE) >> subX
                    y = (row * MI_SIZE) >> subY
                    predict_inter(plane, x, y, bw, bh, row, col, 1, 0)
                    if ( plane == 0 ) {
                        if ( storeRefinedMvs ) {
                            motion_field_motion_vector_storage(row, col, tipSize,
                                 LumaUseOptflowRefinement ? 1 : 2)
                        } else {
                            motion_field_motion_vector_storage(row, col, tipSize, 0 )
                        }
                    }
               }
          }
     }


    The function call to motion_field_motion_vector_storage indicates that the motion field motion vector
    storage process specified in § 7.22 Motion field motion vector storage process is invoked.

```

<a id="s-7-11"></a>

### § 7.11 Motion vector context processes

```text
§   7.11. Motion vector context processes
```

<a id="s-7-11-1"></a>

#### § 7.11.1 General

```text
§   7.11.1. General

    The following sections define the processes used for getting the context needed for reading motion
    vectors.

    The entry point to these processes is triggered by a function call to find_mode_ctx.

    This function call invokes the find mode context process specified in § 7.11.2 Find mode context process.

```

<a id="s-7-11-2"></a>

#### § 7.11.2 Find mode context process

```text
§   7.11.2. Find mode context process

    This process is triggered by a function call to find_mode_ctx.

    The input to this process is a variable isCompound containing 0 for single prediction, or 1 to signal
    compound prediction.

    The variable NewMvCount is set equal to 0.

    The variable WarpMvCount is set equal to 0.

    The variable WarpSampleFound[ 0 ] is set equal to 0.

    The variable WarpSampleFound[ 1 ] is set equal to 0.

    Locations around the block are scanned as follows:

     bw4 = Num_4x4_Blocks_Wide[ MiSize ]
     bh4 = Num_4x4_Blocks_High[ MiSize ]
     isSbBorder = ( MiRow & (Num_4x4_Blocks_High[ SbSize ] - 1) ) == 0 ? 1 : 0
     scan_point_warp_ctx(bh4 - 1, -1)
     leftA = scan_point_ctx( bh4 - 1, -1, isCompound )
     scan_point_warp_ctx( -1, isSbBorder ? Max(0, bw4 - 2) : bw4 - 1 )
     aboveA = scan_point_ctx( -1, bw4 - 1, isCompound )
     scan_point_warp_ctx( 0, -1 )
     leftB = scan_point_ctx( 0, -1, isCompound )
     if ( bw4 >= (isSbBorder ? 4 : 2) ) {




    AV2 Specification                                                                             Page 419 of 1169
         scan_point_warp_ctx( -1, 0)
     }
     aboveB = scan_point_ctx( -1, 0, isCompound )


    where a call of scan_point_ctx indicates that the scan point context process specified in § 7.11.3 Scan
    point context process is invoked and a call of scan_point_warp_ctx indicates that the scan point warp
    context process § 7.11.4 Scan point warp context process is invoked.

    The variable NewMvContext is set as follows:

     nearestMatch = ((aboveA || aboveB) ? 1 : 0) + ((leftA || leftB) ? 1 : 0)
     NewMvContext = nearestMatch + ((NewMvCount > 0) ? 2 : 0)


```

<a id="s-7-11-3"></a>

#### § 7.11.3 Scan point context process

```text
§   7.11.3. Scan point context process

    The inputs to this process are:

      • a variable deltaRow specifying (in units of 4x4 luma samples) how far above to look for a motion
        vector,
      • a variable deltaCol specifying (in units of 4x4 luma samples) how far left to look for a motion vector,
      • a variable isCompound containing 0 for single prediction, or 1 to signal compound prediction.

    This process updates the variable NewMvCount.

    The variable mvRow is set equal to MiRow + deltaRow.

    The variable mvCol is set equal to MiCol + deltaCol.

    The variable found (specifying if a block with matching references has been found) is computed as
    follows:

     found = 0
     if ( is_inside( mvRow, mvCol ) ) {
         if ( IsInters[ mvRow ][ mvCol ] ) {
             candMode = YModes[ mvRow ][ mvCol ]
             if ( isCompound == 0 ) {
                 for ( candList = 0; candList < 2 - use_intrabc; candList++ ) {
                      if ( RefFrames[ mvRow ][ mvCol ][ candList ] == RefFrame[0] ) {
                           if ( has_newmv_for_list( candMode, candList ) ) {
                               NewMvCount = Min(3, NewMvCount + 1)
                           }
                           found = 1
                           break
                      }
                 }
             } else {
                 if ( RefFrames[ mvRow ][ mvCol ][ 0 ] == TIP_FRAME &&
                        ClosestPast == RefFrame[ 0 ] &&
                        ClosestFuture == RefFrame[ 1 ]) {
                           found = 1
                 }
                 if ( RefFrames[ mvRow ][ mvCol ][ 0 ] == RefFrame[ 0 ] &&
                      RefFrames[ mvRow ][ mvCol ][ 1 ] == RefFrame[ 1 ] ) {
                           found = 1
                 }
                 if ( found > 0 ) {
                      if ( has_newmv( candMode ) ) {
                           NewMvCount = Min( 3, NewMvCount + found )
                      }



    AV2 Specification                                                                              Page 420 of 1169
                    }
               }
          }
     }


    where has_newmv_for_list is specified as:

     has_newmv_for_list( candMode, refList ) {
         if ( candMode == NEW_NEWMV || candMode == NEWMV ) {
             return 1
         }
         if ( refList == 0 ) {
             return candMode == NEW_NEARMV || candMode == JOINT_NEWMV
         } else {
             return candMode == NEAR_NEWMV
         }
     }


    If found is greater than 0, the output of this process is 1.

    Otherwise, the output of this process is 0.

```

<a id="s-7-11-4"></a>

#### § 7.11.4 Scan point warp context process

```text
§   7.11.4. Scan point warp context process

    The inputs to this process are:

      • a variable deltaRow specifying (in units of 4x4 luma samples) how far above to look for a motion
        vector,
      • a variable deltaCol specifying (in units of 4x4 luma samples) how far left to look for a motion vector.

    This process updates the variable WarpMvCount (counting the number of matching warp blocks) and the
    array WarpSampleFound (specifying if there are blocks with matching reference frames that may be used
    for warp).

    ExtendDeltaRow and ExtendDeltaCol record the first place where a potential block for extended warp
    was found.

    The position is adjusted to an aligned location on a superblock border as follows:

     isSbBorder = ( MiRow & (Num_4x4_Blocks_High[ SbSize ] - 1) ) == 0
     if ( deltaRow < 0 && isSbBorder ) {
         deltaCol -= MiCol & 1
     }



      NOTE: The intention is for the memory requirement for warp parameters to be reduced by only
      using even mode info locations.


    The variable mvRow is set equal to MiRow + deltaRow.

    The variable mvCol is set equal to MiCol + deltaCol.




    AV2 Specification                                                                              Page 421 of 1169
    If is_inside( mvRow, mvCol ) is equal to 1 and RefFrames[ mvRow ][ mvCol ][ 0 ] has been written for this
    frame (this checks that the candidate location has been decoded) and IsInters[ mvRow ][ mvCol ] is equal
    to 1, the variables are updated as follows:

     if ( RefFrames[ mvRow ][ mvCol ][ 0 ] == RefFrame[ 0 ] ||
           RefFrames[ mvRow ][ mvCol ][ 1 ] == RefFrame[ 0 ]) {
         if ( !WarpSampleFound[ 0 ] ) {
              ExtendDeltaRow = deltaRow
              ExtendDeltaCol = deltaCol
         }
         WarpSampleFound[ 0 ] = 1
         if ( MotionModes[ mvRow ][ mvCol ] >= LOCALWARP ) {
              WarpMvCount++
         }
     }
     if ( RefFrames[ mvRow ][ mvCol ][ 0 ] == RefFrame[ 1 ] ||
           RefFrames[ mvRow ][ mvCol ][ 1 ] == RefFrame[ 1 ]) {
         WarpSampleFound[ 1 ] = 1
     }


```

<a id="s-7-12"></a>

### § 7.12 Motion vector prediction processes

```text
§   7.12. Motion vector prediction processes
```

<a id="s-7-12-1"></a>

#### § 7.12.1 General

```text
§   7.12.1. General

    The following sections define the processes used for predicting the motion vectors.

    The entry point to these processes is triggered by the function call to find_mv_stack in the inter block
    mode info syntax described in § 5.20.7.6 Inter block mode info syntax. This function call invokes the Find
    MV Stack Process specified in § 7.12.2 Find MV stack process.

```

<a id="s-7-12-2"></a>

#### § 7.12.2 Find MV stack process

```text
§   7.12.2. Find MV stack process

    This process is triggered by a function call to find_mv_stack.

    The input to this process is a variable isCompound containing 0 for single prediction, or 1 to signal
    compound prediction.

    This process constructs an array RefStackMv containing motion vector candidates.

    If DeriveWrl is equal to 1, array WarpParamStack will also be constructed and NumWarpFound set to
    indicate the number of candidates in these arrays.

    The process also prepares the value of the contexts used when decoding inter prediction syntax elements.

    The array RefStackMv will be constructed during this process. RefStackMv[ idx ][ list ][ comp ]
    represents component comp (0 for y or 1 for x) of a motion vector for a particular list (0 or 1) at position
    idx (0 to MAX_REF_MV_STACK_SIZE - 1) in the stack.

    The variable SingleMvCount is set equal to 0.

    The variable DerivedMvCount is set equal to 0.

    The variable PruneCount is set equal to 0.

    The variable SinglePruneCount is set equal to 0.

    The variable DerivedPruneCount is set equal to 0.


    AV2 Specification                                                                              Page 422 of 1169
The variable NumWarpFound is set equal to 0.

The motion vector and warp parameter stacks are initialized as follows:

 for( i = 0; i < MAX_REF_MV_STACK_SIZE; i++ ) {
     RefStackRowOffset[ i ] = 0
     RefStackColOffset[ i ] = 0
     for( list = 0; list < 2; list++ ) {
         for ( comp = 0; comp < 2; comp++ ) {
             RefStackMv[ i ][ list ][ comp ] = 0
         }
     }
     RefStackCwp[ i ] = CWP_EQUAL
     if ( i < MAX_WARP_REF_CANDIDATES ) {
         for( j = 0; j < 6; j++ ) {
             WarpParamStack[ i ][ j ] = Default_Warp_Params[ j ]
         }
     }
 }


The variable bw4 specifying the width of the block in 4x4 luma samples is set equal to
Num_4x4_Blocks_Wide[ MiSize ].

The variable bh4 specifying the height of the block in 4x4 luma samples is set equal to
Num_4x4_Blocks_High[ MiSize ].

The variables useTemporal (specifying if the temporal scan process is used) and useTemporalFirst
(specifying if the temporal scan is done before other prediction steps) and isSbBorder (specifying if the
block is at the top edge of a superblock) are specified as:

 useTemporal = ( use_ref_frame_mvs == 1 && !use_intrabc &&
                 RefFrame[ 0 ] != TIP_FRAME &&
                 ( skip_mode || RefFrame[ 0 ] != RefFrame[ 1 ] ) )
 useTemporalFirst = ( DrlReorder != DRL_REORDER_ALWAYS &&
                      use_ref_frame_mvs &&
                      RefFrame[ 1 ] == NONE &&
                      is_inter_ref_frame( RefFrame[ 0 ] ) &&
                      RefFrame[ 0 ] != TIP_FRAME &&
                      (OrigClosestFuture == NONE || OrigClosestPast == NONE) &&
                      Abs(get_relative_dist( OrderHint,
                                             OrderHints[ RefFrame[ 0 ] ] )) <= 2
                    )
 isSbBorder = ( MiRow & (Num_4x4_Blocks_High[ SbSize ] - 1) ) == 0 ? 1 : 0


The following ordered steps apply:

 1. The variable NumMvFound (representing the number of motion vector candidates in RefStackMv) is
    set equal to 0.
 2. The setup global mv process specified in § 7.12.2.1 Setup global MV process is invoked with the input
    0 and the output is assigned to GlobalMvs[ 0 ].
 3. If isCompound is equal to 1, the setup global mv process specified in § 7.12.2.1 Setup global MV
    process is invoked with the input 1 and the output is assigned to GlobalMvs[ 1 ].
 4. If DeriveWrl is equal to 1, the generate points from corners process specified in § 7.12.2.3 Generate
    points from corners process is invoked with the input 0.




AV2 Specification                                                                            Page 423 of 1169
     5. If DeriveWrl is equal to 1 and NumWarpFound is equal to 0 and Num_4x4_Blocks_Wide[ MiSize ] is
        less than or equal to 16, the generate points from corners process specified in § 7.12.2.3 Generate
        points from corners process is invoked with the input 1.
     6. If useTemporal is equal to 1 and useTemporalFirst is equal to 1, the temporal scan process in
        § 7.12.2.7 Temporal scan process is invoked with isCompound as input.
     7. The scan point process in § 7.12.2.6 Scan point process is invoked with deltaRow equal to bh4 - 1,
        deltaCol equal to -1, and isCompound as inputs.
     8. The scan point process in § 7.12.2.6 Scan point process is invoked with deltaRow equal to -1, deltaCol
        equal to Max(0, bw4 - 1 - isSbBorder), and isCompound as inputs.
     9. If bh4 is greater than or equal to 2, the scan point process in § 7.12.2.6 Scan point process is invoked
        with deltaRow equal to 0, deltaCol equal to -1, and isCompound as inputs.
    10. If bw4 is greater than or equal to (isSbBorder ? 4 : 2), the scan point process in § 7.12.2.6 Scan point
        process is invoked with deltaRow equal to -1, deltaCol equal to 0, and isCompound as inputs.
    11. If bh4 is less than or equal to 16, the scan point process in § 7.12.2.6 Scan point process is invoked
        with deltaRow equal to bh4, deltaCol equal to -1, and isCompound as inputs.
    12. If bw4 is less than or equal to 16, the scan point process in § 7.12.2.6 Scan point process is invoked
        with deltaRow equal to -1, deltaCol equal to isSbBorder ? Max(2,bw4) : bw4, and isCompound as inputs.
    13. If useTemporal is equal to 1 and useTemporalFirst is equal to 0, the temporal scan process in
        § 7.12.2.7 Temporal scan process is invoked with isCompound as input.
    14. The scan point process in § 7.12.2.6 Scan point process is invoked with deltaRow equal to -1, deltaCol
        equal to -1 - isSbBorder, and isCompound as inputs.
    15. The variable numNearest (representing the number of motion vectors found in the immediate
        neighborhood) is set equal to NumMvFound.
    16. The scan col process in § 7.12.2.5 Scan col process is invoked with deltaCol equal to -3 and
        isCompound as inputs.
    17. The variable useSort is set equal to DrlReorder == DRL_REORDER_ALWAYS || (DrlReorder ==
        DRL_REORDER_CONSTRAINT && !useTemporalFirst && numNearest >= 4).

    18. If useSort is equal to 1, the sorting process in § 7.12.2.19 Sorting process is invoked with start equal
        to 0, end equal to numNearest, and isCompound as input.
    19. If isCompound is equal to 1, the fill mvp from derived smvp process in § 7.12.2.22 Fill mvp from
        derived smvp process is invoked with isCompound as input.
    20. If enable_refmvbank is equal to 1, the fill mvp from ref mv bank process in § 7.12.2.21 Fill mvp from
        ref mv bank process is invoked with isCompound as input.
    21. If isCompound is equal to 0, the fill mvp from derived smvp process in § 7.12.2.22 Fill mvp from
        derived smvp process is invoked with isCompound as input.
    22. The extra search process in § 7.12.2.20 Extra search process is invoked with isCompound as input.
    23. The clamping process in § 7.12.2.23 Clamping process is invoked with isCompound as input.

```

<a id="s-7-12-2-1"></a>

##### § 7.12.2.1 Setup global MV process

```text
§   7.12.2.1. Setup global MV process

    The input to this process is a variable refList specifying which set of motion vectors to predict.

    The output of this process is the motion vector mv representing global motion for this block.


    AV2 Specification                                                                              Page 424 of 1169
    The motion vector mv is initialized to (0, 0).

    The variable ref (specifying the reference frame) is set equal to RefFrame[ refList ].

    If ref is not equal to INTRA_FRAME and ref is not equal to TIP_FRAME, the get warp motion vector
    process specified in § 7.12.2.2 Get warp motion vector process is invoked with gm_params[ref],
    FrameMvPrecision as inputs, and the output is assigned to mv.

```

<a id="s-7-12-2-2"></a>

##### § 7.12.2.2 Get warp motion vector process

```text
§   7.12.2.2. Get warp motion vector process

    The inputs to this process are:

      • an array params containing the warp parameters,
      • a variable precision specifying the precision required for the motion vector.

    The output of this process is the motion vector mv of the requested precision derived from the warp
    parameters.

    The variable bw (representing the width of the block in units of luma samples) is set equal to
    Block_Width[ MiSize ].

    The variable bh (representing the height of the block in units of luma samples) is set equal to
    Block_Height[ MiSize ].

    The output motion vector mv is specified by projecting the central luma sample of the block as follows:

     x = MiCol * MI_SIZE + bw / 2 - 1
     y = MiRow * MI_SIZE + bh / 2 - 1
     xc = (params[ 2 ] - (1 << WARPEDMODEL_PREC_BITS)) * x +
              params[ 3 ] * y +
              params[ 0 ]
     yc = params[ 4 ] * x +
              (params[ 5 ] - (1 << WARPEDMODEL_PREC_BITS)) * y +
              params[ 1 ]
     if ( precision == MV_PRECISION_EIGHTH_PEL) {
         mv[ 0 ] = Round2Signed( yc, WARPEDMODEL_PREC_BITS - 3 )
         mv[ 1 ] = Round2Signed( xc, WARPEDMODEL_PREC_BITS - 3 )
     } else {
         mv[ 0 ] = Round2Signed( yc, WARPEDMODEL_PREC_BITS - 2 ) * 2
         mv[ 1 ] = Round2Signed( xc, WARPEDMODEL_PREC_BITS - 2 ) * 2
     }
     mv[ 0 ] = Clip3(MV_LOW + 1, MV_UPP - 1, mv[ 0 ] )
     mv[ 1 ] = Clip3(MV_LOW + 1, MV_UPP - 1, mv[ 1 ] )
     mv[ 0 ] = clamp_mv_row( mv[ 0 ] )
     mv[ 1 ] = clamp_mv_col( mv[ 1 ] )
     if ( precision < MV_PRECISION_HALF_PEL ) {
         lower_mv_precision( precision, mv )
     }


```

<a id="s-7-12-2-3"></a>

##### § 7.12.2.3 Generate points from corners process

```text
§   7.12.2.3. Generate points from corners process

    The input to this process is a variable iter specifying how many times the process has been invoked for
    the current block.

    This process creates a warp model from motion vectors found around the current block.




    AV2 Specification                                                                             Page 425 of 1169
The arrays CornerPts, CornerMvs and the variable CornersFound are created from the blocks at three of
the corners of the current block as follows:

 bw4 = Num_4x4_Blocks_Wide[ MiSize ]
 bh4 = Num_4x4_Blocks_High[ MiSize ]
 CornersFound = 0
 warp_corner( -1, -1, iter )
 warp_corner( -1, bw4 - 1, iter )
 warp_corner( bh4 - 1, -1, 0 )


where the call to warp_corner invokes the warp corner process specified in section § 7.12.2.4 Warp
corner process.

If CornersFound is not equal to 3, this process immediately terminates.

Otherwise, the motion vectors are examined to check they are not all the same as follows:

 allMvsSame = 1
 for (n = 0; n < CornersFound; n++) {
     for(c = 0; c < 2; c++) {
         refPts[n][c] = (CornerPts[n][c] << WARPEDMODEL_PREC_BITS) +
                         (CornerMvs[n][c] << GM_TRANS_ONLY_PREC_DIFF)
         if (CornerMvs[n][c] != CornerMvs[0][c]) {
             allMvsSame = 0
         }
     }
 }


If allMvsSame is equal to 1, the process immediately terminates.

If any of the values written into refPts are negative, the process immediately terminates.

The warp model is created and inserted into the candidate list as follows:

 widthLog2 = Mi_Width_Log2[MiSize] + MI_SIZE_LOG2
 heightLog2 = Mi_Height_Log2[MiSize] + MI_SIZE_LOG2
 y0 = CornerPts[0][0]
 x0 = CornerPts[0][1]
 wmmat = zeros[6]
 wmmat[ 2 ] = (refPts[ 1 ][ 1 ] - refPts[ 0 ][ 1 ]) >> widthLog2
 wmmat[ 4 ] = (refPts[ 1 ][ 0 ] - refPts[ 0 ][ 0 ]) >> widthLog2
 wmmat[ 3 ] = (refPts[ 2 ][ 1 ] - refPts[ 0 ][ 1 ]) >> heightLog2
 wmmat[ 5 ] = (refPts[ 2 ][ 0 ] - refPts[ 0 ][ 0 ]) >> heightLog2
 wmmat0 = refPts[ 0 ][ 1 ] - wmmat[ 2 ] * x0 - wmmat[ 3 ] * y0
 wmmat1 = refPts[ 0 ][ 0 ] - wmmat[ 4 ] * x0 - wmmat[ 5 ] * y0
 wmmat = reduce_warp_model( wmmat )
 wmmat[ 0 ] = Clip3( -WARPEDMODEL_TRANS_CLAMP,
                      WARPEDMODEL_TRANS_CLAMP - (1 << WARP_PARAM_REDUCE_BITS),
                      wmmat0 )
 wmmat[ 1 ] = Clip3( -WARPEDMODEL_TRANS_CLAMP,
                      WARPEDMODEL_TRANS_CLAMP - (1 << WARP_PARAM_REDUCE_BITS),
                      wmmat1 )


The insert warp candidate process in § 7.12.2.11 Insert warp candidate process is invoked with wmmat as
input.




AV2 Specification                                                                            Page 426 of 1169
```

<a id="s-7-12-2-4"></a>

##### § 7.12.2.4 Warp corner process

```text
§   7.12.2.4. Warp corner process

    The inputs to this process are:

      • a variable deltaRow specifying (in units of 4x4 luma samples) how far above the base location to look
        for a motion vector,
      • a variable deltaCol specifying (in units of 4x4 luma samples) how far left of the base location to look
        for a motion vector,
      • a variable adjustCol specifying an adjustment to the deltaCol location.

    The variables isSbBorder (specifying if the block is on a horizontal superblock boundary), mvRow and
    mvCol (specifying the corner location) and mvCol2 (specifying the location containing the motion vector),
    are computed as follows:

     mvRow = MiRow + deltaRow
     mvCol = MiCol + deltaCol
     isSbBorder = ( MiRow & (Num_4x4_Blocks_High[ SbSize ] - 1) ) == 0
     deltaCol += adjustCol
     if ( deltaRow < 0 && isSbBorder ) {
         mvCol2 = (MiCol - (MiCol & 1)) + (deltaCol - (deltaCol & 1))
     } else {
         mvCol2 = MiCol + deltaCol
     }


    If isSbBorder is equal to 1 and deltaCol is equal to 0 and Num_4x4_Blocks_Wide[ MiSize ] is less than or
    equal to 2, this process terminates immediately.

    For ref = 0..1, the following applies:

      • If is_inside( mvRow, mvCol2 ) is equal to 1 and RefFrames[ mvRow ][ mvCol2 ][ ref ] has been written
        for this frame and IsInters[ mvRow ][ mvCol2 ] is equal to 1 and RefFrames[ mvRow ][ mvCol2 ][ ref ]
        is equal to RefFrame[ 0 ], the following applies:

          CornerPts[CornersFound][0] = (mvRow + 1) * MI_SIZE
          CornerPts[CornersFound][1] = (mvCol + 1) * MI_SIZE
          if ( MotionModes[ mvRow ][ mvCol2 ] >= LOCALWARP ) {
              if ( ref > 0 ) {
                   return
              }
              CornerMvs[CornersFound] = get_warp_motion_vector_xy_pos(
                                            WarpParams[mvRow][mvCol2][ ref ],
                                            mvRow + 1,mvCol + 1 )
          } else {
              CornerMvs[CornersFound] = SubMvs[ mvRow ][ mvCol2 ][ ref ]
          }
          CornersFound++
          return


    where get_warp_motion_vector_xy_pos (which returns a motion vector for a given location by taking into
    account any warp parameters for a block) as follows:

     get_warp_motion_vector_xy_pos(mat,posRow,posCol) {
         y = posRow * MI_SIZE
         x = posCol * MI_SIZE
         xc = (mat[2] * x + mat[3] * y + mat[0]) - (x << WARPEDMODEL_PREC_BITS)
         yc = (mat[4] * x + mat[5] * y + mat[1]) - (y << WARPEDMODEL_PREC_BITS)
         mv[0] = Round2Signed( yc, WARPEDMODEL_PREC_BITS - 3 )



    AV2 Specification                                                                             Page 427 of 1169
          mv[1] = Round2Signed( xc, WARPEDMODEL_PREC_BITS - 3 )
          mv[0] = Clip3(MV_LOW + 1, MV_UPP - 1, mv[0] )
          mv[1] = Clip3(MV_LOW + 1, MV_UPP - 1, mv[1] )
          mv[0] = clamp_mv_row( mv[0] )
          mv[1] = clamp_mv_col( mv[1] )
          return mv
     }


```

<a id="s-7-12-2-5"></a>

##### § 7.12.2.5 Scan col process

```text
§   7.12.2.5. Scan col process

    The inputs to this process are:

      • a variable deltaCol specifying (in units of 4x4 luma samples) how far left to look for motion vectors,
      • a variable isCompound containing 0 for single prediction, or 1 to signal compound prediction.

    The variable bh4 specifying the height of the block in 4x4 luma samples is set equal to
    Num_4x4_Blocks_High[ MiSize ].

    If Num_4x4_Blocks_Wide[ MiSize ] is equal to 1, the offset is adjusted as follows:

     deltaCol += MiCol & 1


    A series of motion vector locations is scanned as follows:

     scan_point_if_valid(bh4 - 1, deltaCol, isCompound)
     if (bh4 > 1) {
         scan_point_if_valid(0, deltaCol, isCompound)
     }


    where the scan_point_if_valid function is specified as:

     scan_point_if_valid( deltaRow, deltaCol, isCompound ) {
         mvRow = MiRow + deltaRow
         mvCol = MiCol + deltaCol
         mvOtherCol = MiCol - 1
         if ( is_inside( mvRow, mvCol ) && MiColBase[ 0 ][ mvRow ][ mvCol ] !=
                                           MiColBase[ 0 ][ mvRow ][ mvOtherCol ] ) {
             scan_point( deltaRow, deltaCol, isCompound )
         }
     }


    where the call to scan_point invokes the process in § 7.12.2.6 Scan point process.

```

<a id="s-7-12-2-6"></a>

##### § 7.12.2.6 Scan point process

```text
§   7.12.2.6. Scan point process

    The inputs to this process are:

      • a variable deltaRow specifying (in units of 4x4 luma samples) how far above to look for a motion
        vector,
      • a variable deltaCol specifying (in units of 4x4 luma samples) how far left to look for a motion vector,
      • a variable isCompound containing 0 for single prediction, or 1 to signal compound prediction.

    The variable mvRow is set equal to MiRow + deltaRow.




    AV2 Specification                                                                              Page 428 of 1169
    The variable mvCol is set equal to MiCol + deltaCol.

    The position is adjusted to an aligned location on a superblock border as follows:

     isSbBorder = ( MiRow & (Num_4x4_Blocks_High[ SbSize ] - 1) ) == 0
     if ( deltaRow < 0 && isSbBorder ) {
         mvCol = (mvCol >> 1) << 1
         deltaCol = mvCol - MiCol
     }


    The variable weight is set as follows:

      • If deltaRow is equal to -1 and deltaCol is equal to -1, weight is set equal to 0.
      • Otherwise, if deltaCol is less than -1, weight is set equal to 0.
      • Otherwise, weight is set equal to 1.

    If is_inside( mvRow, mvCol ) is equal to 1 and RefFrames[ mvRow ][ mvCol ][ 0 ] has been written for this
    frame (this checks that the candidate location has been decoded), the following applies:

      • The add warp motion vector process in § 7.12.2.9 Add warp motion vector process is invoked with
        mvRow and mvCol as inputs.
      • If NumMvFound is greater than or equal to MAX_REF_MV_STACK_SIZE, this process immediately
        terminates.
      • The add reference motion vector process in § 7.12.2.10 Add reference motion vector process is
        invoked with mvRow, mvCol, isCompound, weight as inputs.

```

<a id="s-7-12-2-7"></a>

##### § 7.12.2.7 Temporal scan process

```text
§   7.12.2.7. Temporal scan process

    The input to this process is a variable isCompound containing 0 for single prediction, or 1 to signal
    compound prediction.

    This process generates motion vector candidates from the motion vectors in MotionFieldMvs.

    The variable bw4 specifying the width of the block in 4x4 luma samples is set equal to
    Num_4x4_Blocks_Wide[ MiSize ].

    The variable bh4 specifying the height of the block in 4x4 luma samples is set equal to
    Num_4x4_Blocks_High[ MiSize ].

    The variable stepW4 is set equal to ( bw4 >= 16 ) ? 4 : 2.

    The variable stepH4 is set equal to ( bh4 >= 16 ) ? 4 : 2.

    The process scans locations within the top 64x64 luma samples of the block as follows:

     startMvFound = NumMvFound
     rowEnd = Min( bh4, 16 )
     colEnd = Min( bw4, 16 )
     deltaRow = rowEnd - stepH4
     deltaCol = colEnd - stepW4
     if (deltaRow >= 0 && deltaCol >= 0) {
         add_tpl_ref_mv( deltaRow, deltaCol, isCompound )
     }
     if ( (rowEnd >= 3 * stepH4 || colEnd >= 3 * stepW4) &&



    AV2 Specification                                                                             Page 429 of 1169
           startMvFound == NumMvFound) {
          add_tpl_ref_mv( rowEnd >> 1, colEnd >> 1, isCompound )
     }


    where the call to add_tpl_ref_mv invokes the temporal sample process in § 7.12.2.8 Temporal sample
    process.

```

<a id="s-7-12-2-8"></a>

##### § 7.12.2.8 Temporal sample process

```text
§   7.12.2.8. Temporal sample process

    The inputs to this process are:

      • variables deltaRow and deltaCol specifying (in units of 4x4 luma samples) the offset to the candidate
        location,
      • a variable isCompound containing 0 for single prediction, or 1 to signal compound prediction.

    This process looks up a motion vector from the motion field and adds it into the stack.

    If NumMvFound is greater than or equal to MAX_REF_MV_STACK_SIZE, this process immediately
    terminates.

    The variable mvRow is set equal to MiRow + deltaRow.

    The variable mvCol is set equal to MiCol + deltaCol.

    If is_inside( mvRow, mvCol ) is equal to 0, this process terminates immediately.

    The variable x8 is set equal to mvCol >> 1.

    The variable y8 is set equal to mvRow >> 1.

    (x8 and y8 represent the position of the candidate in units of 8x8 luma samples.)

    If MotionFieldValid[ y8 ][ x8 ] is equal to 0, this process terminates immediately.

    The process is specified as follows:

     if ( !isCompound ) {
         candMv = get_motion_field_mv( RefFrame[ 0 ], y8, x8 )
         if ( PruneCount >= MAX_PR_NUM ) {
              idx = NumMvFound
         } else {
              for ( idx = 0; idx < NumMvFound; idx++ ) {
                  PruneCount++
                  if ( candMv == RefStackMv[ idx ][ 0 ] )
                      break
              }
         }
         weight = Abs(get_relative_dist( OrderHint,
                                           OrderHints[ RefFrame[ 0 ] ] )) <= 2 ? 2 : 1
         if ( idx < NumMvFound ) {
              WeightStack[ idx ] += weight
         } else {
              RefStackMv[ NumMvFound ][ 0 ] = candMv
              WeightStack[ NumMvFound ] = weight
              NumMvFound += 1
         }
     } else {
         cand0Mv = get_motion_field_mv(RefFrame[ 0 ], y8, x8)
         cand1Mv = get_motion_field_mv(RefFrame[ 1 ], y8, x8)
         if ( PruneCount >= MAX_PR_NUM ) {



    AV2 Specification                                                                           Page 430 of 1169
              idx = NumMvFound
          } else {
              for ( idx = 0; idx < NumMvFound; idx++ ) {
                   PruneCount++
                   if ( cand0Mv == RefStackMv[ idx ][ 0 ] &&
                        cand1Mv == RefStackMv[ idx ][ 1 ] ) {
                       break
                   }
              }
          }
          if ( idx < NumMvFound ) {
              WeightStack[ idx ] += 1
          } else {
              RefStackMv[ NumMvFound ][ 0 ] = cand0Mv
              RefStackMv[ NumMvFound ][ 1 ] = cand1Mv
              WeightStack[ NumMvFound ] = 1
              RefStackCwp[ NumMvFound ] = CWP_EQUAL
              NumMvFound += 1
          }
     }


    where the function get_motion_field_mv is defined as:

     get_motion_field_mv(dst, y8, x8) {
         if ( TrajValid[ dst ][ y8 ][ x8 ] ) {
             return TrajMv[ dst ][ y8 ][ x8 ]
         }
         mv = MotionFieldMvs[ y8 ][ x8 ]
         refOffset = MotionFieldOffset[ y8 ][ x8 ]
         refToDst = get_relative_dist( OrderHint, OrderHints[ dst ] )
         return get_mv_projection( mv, refToDst, refOffset )
     }


```

<a id="s-7-12-2-9"></a>

##### § 7.12.2.9 Add warp motion vector process

```text
§   7.12.2.9. Add warp motion vector process

    The inputs to this process are:

      • variables mvRow and mvCol specifying (in units of 4x4 luma samples) the candidate location.

    This process examines the candidate to find suitable locations for use with warped prediction.

    If IsInters[ mvRow ][ mvCol ] is equal to 1 and DeriveWrl is equal to 1 and MotionModes[ mvRow ]
    [ mvCol ] is greater than or equal to LOCALWARP and RefFrames[ mvRow ][ mvCol ][ 0 ] is equal to
    RefFrame[ 0 ], the insert warp candidate process in § 7.12.2.11 Insert warp candidate process is invoked
    with WarpParams[ mvRow ][ mvCol ][ 0 ] as input.

```

<a id="s-7-12-2-10"></a>

##### § 7.12.2.10 Add reference motion vector process

```text
§   7.12.2.10. Add reference motion vector process

    The inputs to this process are:

      • variables mvRow and mvCol specifying (in units of 4x4 luma samples) the candidate location,
      • a variable isCompound containing 0 for single prediction, or 1 to signal compound prediction,
      • a variable weight specifying the weight attached to this motion vector.

    This process examines the candidate to find matching reference frames.

    If IsInters[ mvRow ][ mvCol ] is equal to 0, this process terminates immediately.




    AV2 Specification                                                                           Page 431 of 1169
    If isCompound is equal to 0, the following applies for candList = 0..(1 - use_intrabc):

      • If RefFrames[ mvRow ][ mvCol ][ candList ] is equal to RefFrame[ 0 ], the search stack process in
        § 7.12.2.12 Search stack process is invoked with mvRow, mvCol, weight, and candList as inputs.
      • Otherwise, if RefFrames[ mvRow ][ mvCol ][ 0 ] is equal to TIP_FRAME and RefFrame[0] is equal to
        ( candList ? ClosestFuture : ClosestPast ), the derive single ref mv candidate from TIP mode process
        specified in § 7.12.2.17 Derive single ref mv candidate from TIP mode process is invoked with mvRow,
        mvCol, weight, and candList as inputs.
      • Otherwise, if candList is equal to 0 and RefFrame[ 0 ] is equal to TIP_FRAME and ClosestPast is
        equal to RefFrames[ mvRow ][ mvCol ][ 0 ] and ClosestFuture is equal to RefFrames[ mvRow ]
        [ mvCol ][ 1 ], the TIP add derived process specified in § 7.12.2.18 TIP add derived process is invoked
        with mvRow and mvCol as inputs.
      • Otherwise, if use_intrabc is equal to 0 and is_derivable_ref_frame(RefFrames[ mvRow ][ mvCol ],
        candList) is equal to 1 and RefFrame[ 0 ] is not equal to TIP_FRAME, the single add derived process
        specified in § 7.12.2.16 Single add derived process is invoked with mvRow, mvCol, and candList as
        inputs.

    Otherwise (isCompound is equal to 1), the following applies:

     if ( RefFrames[ mvRow ][ mvCol ][ 0 ] == TIP_FRAME &&
          RefFrame[ 0 ] == ClosestPast && RefFrame[ 1 ] == ClosestFuture) {
         derive_ref_mv_candidate_from_tip_mode( mvRow, mvCol, weight)
     } else if ( RefFrames[ mvRow ][ mvCol ][ 0 ] == RefFrame[ 0 ] &&
                 RefFrames[ mvRow ][ mvCol ][ 1 ] == RefFrame[ 1 ] ) {
         compound_search_stack( mvRow, mvCol, weight )
     } else {
         compound_add_derived(mvRow, mvCol)
     }


    The function call of compound_search_stack indicates that the compound search stack process in
    § 7.12.2.13 Compound search stack process is invoked with mvRow, mvCol, and weight as inputs.

    The function call of compound_add_derived indicates that the compound add derived process in
    § 7.12.2.14 Compound add derived process is invoked with mvRow and mvCol as inputs.

    The function call of derive_ref_mv_candidate_from_tip_mode indicates that the derive ref mv candidate
    from tip mode process in § 7.12.2.15 Derive ref mv candidate from tip mode process is invoked with
    mvRow, mvCol, and weight as inputs.

    The function is_derivable_ref_frame is specified as:

     is_derivable_ref_frame( candRefFrames, candList ) {
         return candRefFrames[ 0 ] == TIP_FRAME ||
                is_inter_ref_frame( candRefFrames[candList] )
     }


```

<a id="s-7-12-2-11"></a>

##### § 7.12.2.11 Insert warp candidate process

```text
§   7.12.2.11. Insert warp candidate process

    The input to this process is an array params specifying the candidate parameters.

    If NumWarpFound is greater than or equal to MAX_WARP_REF_CANDIDATES, this process immediately
    terminates.



    AV2 Specification                                                                            Page 432 of 1169
    Otherwise, the parameters are saved into the warp parameter stack as follows:

     for( i = 0; i < 6; i++ ) {
         WarpParamStack[ NumWarpFound ][ i ] = params[ i ]
     }
     NumWarpFound++


```

<a id="s-7-12-2-12"></a>

##### § 7.12.2.12 Search stack process

```text
§   7.12.2.12. Search stack process

    The inputs to this process are:

      • variables mvRow and mvCol specifying (in units of 4x4 luma samples) the candidate location,
      • a variable weight,
      • a variable candList specifying which list in the candidate matches our reference frame.

    This process searches the stack for an exact match with a candidate motion vector. If present, the weight
    of the candidate motion vector is added to the weight of its counterpart in the stack, otherwise the
    process adds a motion vector to the stack.

    The motion vector candMv is set equal to get_mv( mvRow, mvCol, 0, candList ).

    The process depends on whether the candidate motion vector is already in the stack as follows:

     candMvFound = 0
     if ( PruneCount < MAX_PR_NUM ) {
         for ( idx = 0; idx < NumMvFound; idx++ ) {
             PruneCount++
             if ( candMv == RefStackMv[ idx ][ 0 ] ) {
                 WeightStack[ idx ] += weight
                 candMvFound = 1
                 break
             }
         }
     }
     if ( !candMvFound && NumMvFound < MAX_REF_MV_STACK_SIZE ) {
         RefStackMv[ NumMvFound ][ 0 ][ 0 ] = candMv[ 0 ]
         RefStackMv[ NumMvFound ][ 0 ][ 1 ] = candMv[ 1 ]
         RefStackRowOffset[ NumMvFound ] = mvRow - MiRow
         RefStackColOffset[ NumMvFound ] = mvCol - MiCol
         WeightStack[ NumMvFound ] = weight
         NumMvFound++
     }


```

<a id="s-7-12-2-13"></a>

##### § 7.12.2.13 Compound search stack process

```text
§   7.12.2.13. Compound search stack process

    The inputs to this process are:

      • variables mvRow and mvCol specifying (in units of 4x4 luma samples) the candidate location,
      • a variable weight.

    This process searches the stack for an exact match with a candidate pair of motion vectors. If present,
    the weight of the candidate pair of motion vectors is added to the weight of its counterpart in the stack,
    otherwise the process adds the motion vectors to the stack.

    The array candMvs (containing two motion vectors) is set equal to SubMvs[ mvRow ][ mvCol ].




    AV2 Specification                                                                             Page 433 of 1169
    The variable candCwp is set equal to CwpIdxs[ mvRow ][ mvCol ].

    The variable candMode is set equal to YModes[ mvRow ][ mvCol ].

    The variable candSize is set equal to MiSizes[ PlaneStart ][ mvRow ][ mvCol ].

    The variable large is set as follows:

      • If Min( Block_Width[ candSize ],Block_Height[ candSize ] ) is greater than or equal to 8, large is set
        equal to 1.
      • Otherwise, large is set equal to 0.

    If large is equal to 1 and candMode is equal to GLOBAL_GLOBALMV, for refList = 0..1 the following
    applies:

      • If GmType[ RefFrame[ refList ] ] is greater than IDENTITY, candMvs[ refList ] is set equal to
        GlobalMvs[ refList ].

    The process depends on whether the candidate motion vector pair is already in the stack as follows:

     candMvFound = 0
     if ( PruneCount < MAX_PR_NUM ) {
         for ( idx = 0; idx < NumMvFound; idx++ ) {
             PruneCount++
             if ( candMvs[ 0 ][ 0 ] == RefStackMv[ idx ][ 0 ][ 0 ] &&
                 candMvs[ 0 ][ 1 ] == RefStackMv[ idx ][ 0 ][ 1 ] &&
                 candMvs[ 1 ][ 0 ] == RefStackMv[ idx ][ 1 ][ 0 ] &&
                 candMvs[ 1 ][ 1 ] == RefStackMv[ idx ][ 1 ][ 1 ] ) {
                 WeightStack[ idx ] += weight
                 candMvFound = 1
                 break
             }
         }
     }
     if (!candMvFound && NumMvFound < MAX_REF_MV_STACK_SIZE) {
         for (i = 0; i < 2; i++) {
             RefStackMv[ NumMvFound ][ i ][ 0 ] = candMvs[ i ][ 0 ]
             RefStackMv[ NumMvFound ][ i ][ 1 ] = candMvs[ i ][ 1 ]
         }
         RefStackCwp[ NumMvFound ] = candCwp
         WeightStack[ NumMvFound ] = weight
         RefStackRowOffset[ NumMvFound ] = mvRow - MiRow
         RefStackColOffset[ NumMvFound ] = mvCol - MiCol
         NumMvFound++
     }



      NOTE:       NumMvFound will always be less than MAX_REF_MV_STACK_SIZE when this process is
      called.

```

<a id="s-7-12-2-14"></a>

##### § 7.12.2.14 Compound add derived process

```text
§   7.12.2.14. Compound add derived process

    The inputs to this process are:

      • variables mvRow and mvCol specifying (in units of 4x4 luma samples) the candidate location.




    AV2 Specification                                                                             Page 434 of 1169
This process conditionally adds a candidate to the derived motion vector stack and the single motion
vector stack as follows:

 if ( enable_mv_traj && use_ref_frame_mvs && RefFrame[ 0 ] != RefFrame[ 1 ] ) {
     for (list = 0; list < 2; list++) {
          candRef = RefFrames[ mvRow ][ mvCol ][ list ]
          if ( is_inter_ref_frame( candRef ) && candRef != TIP_FRAME ) {
              candMv = get_mv(mvRow, mvCol, -1, list)
              trajY8 = MiRow >> 1
              trajX8 = MiCol >> 1
              trajCandValid = TrajValid[ candRef ][ trajY8 ][ trajX8 ]
              trajRef0Valid = TrajValid[ RefFrame[ 0 ] ][ trajY8 ][ trajX8 ]
              trajRef1Valid = TrajValid[ RefFrame[ 1 ] ][ trajY8 ][ trajX8 ]
              if ( trajCandValid && trajRef0Valid && trajRef1Valid ) {
                  trajCandMv = TrajMv[ candRef ][ trajY8 ][ trajX8 ]
                  trajRef0 = TrajMv[ RefFrame[ 0 ] ][ trajY8 ][ trajX8 ]
                  trajRef1 = TrajMv[ RefFrame[ 1 ] ][ trajY8 ][ trajX8 ]
                  for( c = 0; c < 2; c++ ) {
                      candMvs[ 0 ][ c ] = Clip3( MV_LOW + 1, MV_UPP - 1,
                                                candMv[ c ] + trajRef0[ c ] -
                                                trajCandMv[ c ] )
                      candMvs[ 1 ][ c ] = Clip3( MV_LOW + 1, MV_UPP - 1,
                                                 candMv[ c ] + trajRef1[ c ] -
                                                 trajCandMv[ c ] )
                  }
                  if ( DerivedMvCount < MAX_DR_STACK_SIZE &&
                       !comp_mv_in_stack( DerivedStackMv, DerivedMvCount,
                                          candMvs[ 0 ], candMvs[ 1 ] ) ) {
                      DerivedStackMv[DerivedMvCount][0][0] = candMvs[0][0]
                      DerivedStackMv[DerivedMvCount][0][1] = candMvs[0][1]
                      DerivedStackMv[DerivedMvCount][1][0] = candMvs[1][0]
                      DerivedStackMv[DerivedMvCount][1][1] = candMvs[1][1]
                      DerivedMvCount++
                  }
              }
          }
     }
 }
 if (RefFrames[ mvRow ][ mvCol ][ 0 ] == RefFrame[ 0 ] ||
     RefFrames[ mvRow ][ mvCol ][ 1 ] == RefFrame[ 0 ]) {
     candRefIdx0 = 0
     candRefIdx1 = 1
 } else if (RefFrames[ mvRow ][ mvCol ][ 0 ] == RefFrame[ 1 ] ||
              RefFrames[ mvRow ][ mvCol ][ 1 ] == RefFrame[ 1 ]) {
     candRefIdx0 = 1
     candRefIdx1 = 0
 } else {
     return
 }
 candList = RefFrames[ mvRow ][ mvCol ][ 0 ] == RefFrame[ candRefIdx0 ] ? 0 : 1
 candMv = get_mv(mvRow, mvCol, candRefIdx0, candList)
 for( candIdx = 0; candIdx < SingleMvCount &&
                    DerivedMvCount < MAX_DR_STACK_SIZE; candIdx++ ) {
     if (SingleRefFrame[candIdx] == RefFrame[candRefIdx1]) {
          l0Mv = candRefIdx0 == 0 ? candMv : SingleMv[candIdx]
          l1Mv = candRefIdx0 == 1 ? candMv : SingleMv[candIdx]
          if (!comp_mv_in_stack(DerivedStackMv, DerivedMvCount, l0Mv, l1Mv)) {
              DerivedStackMv[DerivedMvCount][0][0] = l0Mv[0]
              DerivedStackMv[DerivedMvCount][0][1] = l0Mv[1]
              DerivedStackMv[DerivedMvCount][1][0] = l1Mv[0]
              DerivedStackMv[DerivedMvCount][1][1] = l1Mv[1]
              DerivedMvCount++
          }
          break
     }
 }
 if ( SinglePruneCount < MAX_DR_PR_NUM ) {
     for( candIdx = 0; candIdx < SingleMvCount; candIdx++ ) {
          SinglePruneCount++



AV2 Specification                                                                          Page 435 of 1169
               if (SingleRefFrame[candIdx] == RefFrame[candRefIdx0] &&
                   SingleMv[candIdx][0] == candMv[0] &&
                   SingleMv[candIdx][1] == candMv[1]) {
                   return
               }
         }
     }
     if ( SingleMvCount < MAX_DR_STACK_SIZE ) {
         SingleRefFrame[SingleMvCount] = RefFrame[candRefIdx0]
         SingleMv[SingleMvCount][0] = candMv[0]
         SingleMv[SingleMvCount][1] = candMv[1]
         SingleMvCount++
     }


    The function get_mv (which gets a motion vector for a location) is specified as:

     get_mv(mvRow, mvCol, refList, candList) {
         candMode = YModes[ mvRow ][ mvCol ]
         candSize = MiSizes[ PlaneStart ][ mvRow ][ mvCol ]
         candRefFrame = RefFrames[ mvRow ][ mvCol ][ candList ]
         large = ( Min( Block_Width[ candSize ],Block_Height[ candSize ] ) >= 8 )
         if ( refList >= 0 &&
              ( candMode == GLOBALMV || candMode == GLOBAL_GLOBALMV ) &&
              candRefFrame != TIP_FRAME &&
              GmType[ candRefFrame ] > IDENTITY &&
              large ) {
             return GlobalMvs[ refList ]
         } else {
             return SubMvs[ mvRow ][ mvCol ][ candList ]
         }
     }


    The function comp_mv_in_stack (which determines if the motion vector pair is already in the stack) is
    specified as:

     comp_mv_in_stack(mvStack, count, list0Mv, list1Mv) {
         if ( DerivedPruneCount < MAX_DR_PR_NUM ) {
             for (i = 0; i < count; i++) {
                  DerivedPruneCount++
                  if (mvStack[i][0] == list0Mv &&
                      mvStack[i][1] == list1Mv) {
                      return 1
                  }
             }
         }
         return 0
     }


```

<a id="s-7-12-2-15"></a>

##### § 7.12.2.15 Derive ref mv candidate from tip mode process

```text
§   7.12.2.15. Derive ref mv candidate from tip mode process

    The inputs to this process are:

      • variables candRow and candCol specifying (in units of 4x4 luma samples) the candidate location,
      • a variable weight specifying the weight attached to this motion vector.

    The candidate is added to the stack of motion vectors as follows:

     candMvs = get_tip_cand(candRow,candCol)
     candMvFound = 0
     if ( PruneCount < MAX_PR_NUM ) {
         for ( idx = 0; idx < NumMvFound; idx++ ) {



    AV2 Specification                                                                          Page 436 of 1169
                PruneCount++
                match = candMvs[ 0 ] == RefStackMv[ idx ][ 0 ] &&
                        candMvs[ 1 ] == RefStackMv[ idx ][ 1 ]
                if (match) {
                    WeightStack[ idx ] += weight
                    candMvFound = 1
                    break
                }
         }
     }
     if (!candMvFound && NumMvFound < MAX_REF_MV_STACK_SIZE) {
         for (i = 0; i < 2; i++) {
             RefStackMv[ NumMvFound ][ i ] = candMvs[ i ]
         }
         RefStackCwp[ NumMvFound ] = CWP_EQUAL
         WeightStack[ NumMvFound ] = weight
         NumMvFound++
     }



      NOTE:       NumMvFound will always be less than MAX_REF_MV_STACK_SIZE when this process is
      called.

```

<a id="s-7-12-2-16"></a>

##### § 7.12.2.16 Single add derived process

```text
§   7.12.2.16. Single add derived process

    The inputs to this process are:

      • variables mvRow and mvCol specifying (in units of 4x4 luma samples) the candidate location,
      • a variable candList specifying which slot of the reference list to examine.

    The process conditionally adds a candidate to DerivedStackMv as follows:

     if ( RefFrames[mvRow][mvCol][0] == TIP_FRAME ) {
         candMvs = get_tip_cand(mvRow,mvCol)
         candMv = candMvs[ candList ]
         candRef = candList ? ClosestFuture : ClosestPast
     } else {
         candMv = get_mv(mvRow, mvCol, -1, candList)
         candRef = RefFrames[mvRow][mvCol][candList]
     }
     curDist = FrameDistance[ RefFrame[ 0 ] ]
     candDist = FrameDistance[ candRef ]
     haveProj = 0
     if ( use_ref_frame_mvs && enable_mv_traj ) {
         trajY8 = MiRow >> 1
         trajX8 = MiCol >> 1
         trajCurValid = TrajValid[ RefFrame[ 0 ] ][ trajY8 ][ trajX8 ]
         trajCandValid = TrajValid[ candRef ][ trajY8 ][ trajX8 ]
         if ( trajCurValid && trajCandValid ) {
              trajCurMv = TrajMv[ RefFrame[ 0 ] ][ trajY8 ][ trajX8 ]
              trajCandMv = TrajMv[ candRef ][ trajY8 ][ trajX8 ]
              haveProj = 1
              for( c = 0; c < 2; c++ ) {
                  projCandMv[ c ] = Clip3( MV_LOW + 1, MV_UPP - 1,
                                           candMv[c] + trajCurMv[c] - trajCandMv[c] )
              }
         }
     }
     if (!haveProj && ( (curDist > 0 && candDist > 0) ||
                         (curDist < 0 && candDist < 0) ) ) {
         projCandMv = get_mv_projection( candMv, Abs( curDist ), Abs( candDist ) )
         haveProj = 1
     }
     if (haveProj) {
         if ( DerivedPruneCount < MAX_DR_PR_NUM ) {



    AV2 Specification                                                                         Page 437 of 1169
               for (i = 0; i < DerivedMvCount; i++) {
                   DerivedPruneCount++
                   if (DerivedStackMv[i][0] == projCandMv) {
                       return
                   }
               }
          }
          if ( DerivedMvCount < MAX_DR_STACK_SIZE ) {
              DerivedStackMv[DerivedMvCount][0] = projCandMv
              DerivedMvCount++
          }
     }


```

<a id="s-7-12-2-17"></a>

##### § 7.12.2.17 Derive single ref mv candidate from TIP mode process

```text
§   7.12.2.17. Derive single ref mv candidate from TIP mode process

    The inputs to this process are:

      • variables candRow and candCol specifying (in units of 4x4 luma samples) the candidate location,
      • a variable weight specifying the weight attached to this motion vector,
      • a variable candList specifying which slot of the reference list to examine.

    The process conditionally adds a candidate to DerivedStackMv as follows:

     candMvs = get_tip_cand(candRow,candCol)
     candMvFound = 0
     if ( PruneCount < MAX_PR_NUM ) {
         for ( idx = 0; idx < NumMvFound; idx++ ) {
             PruneCount++
             match = candMvs[ candList ][ 0 ] == RefStackMv[ idx ][ 0 ][ 0 ] &&
                     candMvs[ candList ][ 1 ] == RefStackMv[ idx ][ 0 ][ 1 ]
             if (match) {
                 WeightStack[ idx ] += weight
                 candMvFound = 1
                 break
             }
         }
     }
     if ( !candMvFound ) {
         if ( NumMvFound < MAX_REF_MV_STACK_SIZE ) {
             RefStackMv[ NumMvFound ][ 0 ][ 0 ] = candMvs[ candList ][ 0 ]
             RefStackMv[ NumMvFound ][ 0 ][ 1 ] = candMvs[ candList ][ 1 ]
             WeightStack[ NumMvFound ] = weight
             NumMvFound++
         }
     }


```

<a id="s-7-12-2-18"></a>

##### § 7.12.2.18 TIP add derived process

```text
§   7.12.2.18. TIP add derived process

    The inputs to this process are:

      • variables mvRow and mvCol specifying (in units of 4x4 luma samples) the candidate location.

    The process conditionally adds a candidate to DerivedStackMv as follows:

     linearMv[0] = SubMvs[mvRow][mvCol][0][0] - SubMvs[mvRow][mvCol][1][0]
     linearMv[1] = SubMvs[mvRow][mvCol][0][1] - SubMvs[mvRow][mvCol][1][1]
     (refOffset, pastOffset, futureOffset) = get_tip_offsets()
     projMv = get_mv_projection( linearMv, pastOffset, refOffset )
     derivedMv[0] = Clip3( MV_LOW + 1, MV_UPP - 1,
                           SubMvs[mvRow][mvCol][0][0] - projMv[0])
     derivedMv[1] = Clip3( MV_LOW + 1, MV_UPP - 1,




    AV2 Specification                                                                         Page 438 of 1169
                           SubMvs[mvRow][mvCol][0][1] - projMv[1])
     if ( DerivedPruneCount < MAX_DR_PR_NUM ) {
         for (i = 0; i < DerivedMvCount; i++) {
             DerivedPruneCount++
             if (DerivedStackMv[i][0][0] == derivedMv[0] &&
                 DerivedStackMv[i][0][1] == derivedMv[1]) {
                 return
             }
         }
     }
     if (DerivedMvCount < MAX_DR_STACK_SIZE) {
         DerivedStackMv[DerivedMvCount][0][0] = derivedMv[0]
         DerivedStackMv[DerivedMvCount][0][1] = derivedMv[1]
         DerivedMvCount++
     }


```

<a id="s-7-12-2-19"></a>

##### § 7.12.2.19 Sorting process

```text
§   7.12.2.19. Sorting process

    The inputs to this process are:

      • a variable start representing the first position to be sorted,
      • a variable end representing the length of the array,
      • a variable isCompound containing 0 for single prediction, or 1 to signal compound prediction.

    This process moves the highest weight entry in the stack to the start.

    The process is specified as:

     maxWeight = WeightStack[start]
     maxWeightIdx = start
     for ( idx = start + 1; idx < end; idx++ ) {
         if ( maxWeight < WeightStack[ idx ] ) {
             maxWeight = WeightStack[ idx ]
             maxWeightIdx = idx
         }
     }
     if (maxWeightIdx != start) {
         swap_stack( start, maxWeightIdx )
     }


    When the function swap_stack is invoked, the entries at locations i and j are swapped in WeightStack and
    RefStackMv as follows:

     swap_stack( i, j ) {
       temp = WeightStack[ i ]
       WeightStack[ i ] = WeightStack[ j ]
       WeightStack[ j ] = temp
       temp = RefStackCwp[ i ]
       RefStackCwp[ i ] = RefStackCwp[ j ]
       RefStackCwp[ j ] = temp
       temp = RefStackRowOffset[ i ]
       RefStackRowOffset[ i ] = RefStackRowOffset[ j ]
       RefStackRowOffset[ j ] = temp
       temp = RefStackColOffset[ i ]
       RefStackColOffset[ i ] = RefStackColOffset[ j ]
       RefStackColOffset[ j ] = temp
       for ( list = 0; list < 1 + isCompound; list++ ) {
         for ( comp = 0; comp < 2; comp++ ) {
           temp = RefStackMv[ i ][ list ][ comp ]
           RefStackMv[ i ][ list ][ comp ] = RefStackMv[ j ][ list ][ comp ]
           RefStackMv[ j ][ list ][ comp ] = temp




    AV2 Specification                                                                          Page 439 of 1169
             }
         }
     }


```

<a id="s-7-12-2-20"></a>

##### § 7.12.2.20 Extra search process

```text
§   7.12.2.20. Extra search process

    The input to this process is a variable isCompound containing 0 for single prediction, or 1 to signal
    compound prediction.

    This process clamps the stack and adds additional motion vectors to RefStackMv.

    The candidates on the stack are clamped as follows:

     for ( idx = 0; idx < NumMvFound ; idx++ ) {
         for ( list = 0; list < (isCompound ? 2 : 1); list++ ) {
             refMv[ 0 ] = RefStackMv[ idx ][ list ][ 0 ]
             refMv[ 1 ] = RefStackMv[ idx ][ list ][ 1 ]
             refMv[ 0 ] = clamp_mv_row( refMv[ 0 ] )
             refMv[ 1 ] = clamp_mv_col( refMv[ 1 ] )
             RefStackMv[ idx ][ list ][ 0 ] = refMv[ 0 ]
             RefStackMv[ idx ][ list ][ 1 ] = refMv[ 1 ]
         }
     }


    A global mv candidate is added if not already present as follows:

     if ( NumMvFound < MAX_REF_MV_STACK_SIZE && !use_intrabc ) {
         found = 0
         if ( PruneCount < MAX_PR_NUM ) {
             for (idx = 0; idx < NumMvFound; idx++) {
                 PruneCount++
                 if ( GlobalMvs[ 0 ] == RefStackMv[ idx ][ 0 ] ) {
                     if ( !isCompound ||
                           (GlobalMvs[ 1 ] == RefStackMv[ idx ][ 1 ] ) ) {
                          found = 1
                          break
                     }
                 }
             }
         }
         if (!found) {
             for ( list = 0; list < (isCompound ? 2 : 1); list++ ) {
                 RefStackMv[ NumMvFound ][ list ][ 0 ] = GlobalMvs[ list ][ 0 ]
                 RefStackMv[ NumMvFound ][ list ][ 1 ] = GlobalMvs[ list ][ 1 ]
             }
             RefStackCwp[ NumMvFound ] = CWP_EQUAL
             NumMvFound++
         }
     }


    If Block_Width[ MiSize ] is greater than 32 and Block_Height[ MiSize ] is greater than 32, extra
    candidates are added as follows:

     num = NumMvFound
     if ( num > 1 ) {
         insert_mvp_candidate( isCompound, 0, 1 )
         insert_mvp_candidate( isCompound, 1, 0 )
     }
     if ( num > 2 ) {
         insert_mvp_candidate( isCompound, 0, 2 )
         insert_mvp_candidate( isCompound, 2, 0 )




    AV2 Specification                                                                             Page 440 of 1169
      insert_mvp_candidate( isCompound, 1, 2 )
      insert_mvp_candidate( isCompound, 2, 1 )
 }


where insert_mvp_candidate (which adds a candidate with a mixture of existing motion vectors) is
specified as follows:

 insert_mvp_candidate( isCompound, yCand, xCand ) {
     candMvs[ 0 ][ 0 ] = RefStackMv[ yCand ][ 0 ][ 0 ]
     candMvs[ 0 ][ 1 ] = RefStackMv[ xCand ][ 0 ][ 1 ]
     candMvs[ 1 ][ 0 ] = RefStackMv[ yCand ][ 1 ][ 0 ]
     candMvs[ 1 ][ 1 ] = RefStackMv[ xCand ][ 1 ][ 1 ]
     if ( NumMvFound < MAX_REF_MV_STACK_SIZE) {
         if ( PruneCount < MAX_PR_NUM ) {
             for ( idx = 0; idx < NumMvFound; idx++ ) {
                 PruneCount++
                 match = candMvs[ 0 ][ 0 ] == RefStackMv[ idx ][ 0 ][ 0 ] &&
                          candMvs[ 0 ][ 1 ] == RefStackMv[ idx ][ 0 ][ 1 ]
                 if ( !isCompound && match )
                      return
                 if ( isCompound && match &&
                          candMvs[ 1 ][ 0 ] == RefStackMv[ idx ][ 1 ][ 0 ] &&
                          candMvs[ 1 ][ 1 ] == RefStackMv[ idx ][ 1 ][ 1 ] )
                      return
             }
         }
         RefStackMv[ NumMvFound ][ 0 ][ 0 ] = candMvs[ 0 ][ 0 ]
         RefStackMv[ NumMvFound ][ 0 ][ 1 ] = candMvs[ 0 ][ 1 ]
         RefStackMv[ NumMvFound ][ 1 ][ 0 ] = candMvs[ 1 ][ 0 ]
         RefStackMv[ NumMvFound ][ 1 ][ 1 ] = candMvs[ 1 ][ 1 ]
         NumMvFound++
     }
 }


If DeriveWrl is equal to 1, additional warp candidates are added as follows:

 ref = RefFrame[ 0 ]
 c = WarpBankSize[ ref ]
 s = WarpBankStart[ ref ]
 for( i = c - 1 ; i >= 0 ; i-- ) {
     idx = (s + i) % WARP_PARAM_BANK_SIZE
     insert_warp_candidate( WarpBankParams[ref][idx] )
 }
 insert_warp_candidate( gm_params[ref] )
 for( i = 0 ; i < 2; i++ ) {
     insert_warp_candidate( Default_Warp_Params )
 }


Where setup_shear invokes the setup shear process specified in § 7.13.3.21 Setup shear process, and
insert_warp_candidate invokes the insert warp candidate process in § 7.12.2.11 Insert warp candidate
process.

The table Default_Warp_Params is defined as:

 Default_Warp_Params[6] = {
   0, 0, 1 << WARPEDMODEL_PREC_BITS, 0, 0, 1 << WARPEDMODEL_PREC_BITS
 }




AV2 Specification                                                                         Page 441 of 1169
    If use_intrabc is equal to 1, additional intra block copy candidates are added as follows:

     add_to_ref_bv(0, -Block_Height[ SbSize ])
     add_to_ref_bv(-Block_Width[ SbSize ] - INTRABC_DELAY_PIXELS, 0)
     add_to_ref_bv(0, -Block_Height[ MiSize ])
     add_to_ref_bv(-Block_Width[ MiSize ],0)


    where the function add_to_ref_bv is specified as:

     add_to_ref_bv(dx,dy) {
         if ( NumMvFound < max_bvp_drl_bits_minus_1 + 2 ) {
             RefStackMv[ NumMvFound ][ 0 ][ 0 ] = dy << 3
             RefStackMv[ NumMvFound ][ 0 ][ 1 ] = dx << 3
             NumMvFound++
         }
     }


```

<a id="s-7-12-2-21"></a>

##### § 7.12.2.21 Fill mvp from ref mv bank process

```text
§   7.12.2.21. Fill mvp from ref mv bank process

    The input to this process is a variable isCompound containing 0 for single prediction, or 1 to signal
    compound prediction.

    This process adds additional motion vectors to RefStackMv from the bank of motion vectors.

    The candidates are added as follows:

     ref = get_rmb_list_index( RefFrame )
     key = isCompound ? RefFrame[0] + (RefFrame[1] + 1) * BANK_REFS_PER_FRAME :
                         RefFrame[0]
     if ( use_intrabc ) {
         maxRefMvCount = max_bvp_drl_bits_minus_1 + 2
     } else {
         maxRefMvCount = max_drl_bits_minus_1 + 2
     }
     c = RefMvBankSize[ ref ]
     s = RefMvBankStart[ ref ]
     for( i = c - 1; i >= 0 && NumMvFound < maxRefMvCount; i-- ) {
         idx = (s + i) % REF_MV_BANK_SIZE
         if ( RefMvBankParams[ ref ][ idx ][ 1 ] == key ) {
              for(list=0;list<2;list++) {
                  for(comp=0;comp<2;comp++) {
                      candMvs[list][comp] =
                          RefMvBankParams[ ref ][ idx ][2 + list * 2 + comp]
                  }
              }
              check_rmb_cand(candMvs, isCompound, RefMvBankParams[ ref ][ idx ][ 0 ] )
         }
     }


    where the function check_rmb_cand (which checks if the motion vector is new and points inside the
    frame) is defined as:

     check_rmb_cand(candMvs, isCompound, cwp) {
         bw = Block_Width[ MiSize ]
         bh = Block_Height[ MiSize ]
         if ( PruneCount < MAX_PR_NUM ) {
             for ( idx = 0; idx < NumMvFound; idx++ ) {
                 PruneCount++
                 if ( candMvs[ 0 ] == RefStackMv[ idx ][ 0 ] &&
                     (!isCompound || candMvs[ 1 ] == RefStackMv[ idx ][ 1 ])




    AV2 Specification                                                                             Page 442 of 1169
                    ) {
                          return
                    }
              }
          }
          for (i = 0; i < 1 + isCompound; i++) {
              refY = MiRow * MI_SIZE + (candMvs[i][0] / 8)
              refX = MiCol * MI_SIZE + (candMvs[i][1] / 8)
              if ( refX <= -bw || refY <= -bh ||
                   refX >= MiCols * MI_SIZE ||
                   refY >= MiRows * MI_SIZE) {
                  return
              }
          }
          for (i = 0; i < 1 + isCompound; i++) {
              RefStackMv[ NumMvFound ][ i ][ 0 ] = candMvs[ i ][ 0 ]
              RefStackMv[ NumMvFound ][ i ][ 1 ] = candMvs[ i ][ 1 ]
              RefStackCwp[ NumMvFound ] = cwp
          }
          NumMvFound++
     }


    and the function get_rmb_list_index which returns the bank to use for the current choice of reference
    frames is defined as:

     get_rmb_list_index( refFrames ) {
         if ( !is_inter_ref_frame(refFrames[ 1 ]) && refFrames[ 0 ] <= 5 ) {
             return refFrames[ 0 ]
         } else if ( refFrames[ 0 ] == 0 && refFrames[ 1 ] == 0 ) {
             return 6
         } else if ( refFrames[ 0 ] == 0 && refFrames[ 1 ] == 1 ) {
             return 7
         } else {
             return 8
         }
     }


```

<a id="s-7-12-2-22"></a>

##### § 7.12.2.22 Fill mvp from derived smvp process

```text
§   7.12.2.22. Fill mvp from derived smvp process

    The input to this process is a variable isCompound containing 0 for single prediction, or 1 to signal
    compound prediction.

    This process adds additional derived motion vectors to RefStackMv.

    The candidates are added as follows:

     if ( use_intrabc ) {
         maxRefMvCount = max_bvp_drl_bits_minus_1 + 2
     } else {
         maxRefMvCount = max_drl_bits_minus_1 + 2
     }
     if ( NumMvFound >= maxRefMvCount ) {
         return
     }
     for( derivedIdx = 0; derivedIdx < DerivedMvCount; derivedIdx++) {
         found = 0
         if ( PruneCount < MAX_PR_NUM ) {
              for (idx = 0; idx < NumMvFound; idx++) {
                  PruneCount++
                  if ( stack_match(idx,derivedIdx,isCompound) ) {
                      found = 1
                      break
                  }
              }



    AV2 Specification                                                                             Page 443 of 1169
          }
          if ( !found && NumMvFound < maxRefMvCount ) {
              for (i = 0; i < 1 + isCompound; i++) {
                  for (comp = 0; comp < 2; comp++ ) {
                      RefStackMv[ NumMvFound ][ i ][ comp ] =
                           DerivedStackMv[derivedIdx][ i ][ comp ]
                  }
              }
              RefStackCwp[ NumMvFound ] = CWP_EQUAL
              NumMvFound++
          }
     }


    where stack_match (which returns true if a derived motion vector matches motion vectors already in
    RefStackMv) is specified as:

     stack_match(idx,derivedIdx,isCompound) {
         for( lst = 0; lst <= isCompound; lst++ ) {
             for( comp = 0; comp < 2; comp++ ) {
                  if ( DerivedStackMv[ derivedIdx ][ lst ][ comp ] !=
                       RefStackMv[ idx ][ lst ][ comp ] ) {
                      return 0
                  }
             }
         }
         return 1
     }


```

<a id="s-7-12-2-23"></a>

##### § 7.12.2.23 Clamping process

```text
§   7.12.2.23. Clamping process

    The input to this process is a variable isCompound containing 0 for single prediction, or 1 to signal
    compound prediction.

    This process clamps the candidates in RefStackMv.

    The variable numLists specifying the number of reference frames used for this block is set equal to
    ( isCompound ? 2 : 1 ).

    If use_intrabc is equal to 0, the motion vectors are clamped as follows:

     for ( list = 0; list < numLists; list++ ) {
         for ( idx = 0; idx < NumMvFound ; idx++ ) {
             refMv = RefStackMv[ idx ][ list ]
             refMv[ 0 ] = clamp_mv_row( refMv[ 0 ] )
             refMv[ 1 ] = clamp_mv_col( refMv[ 1 ] )
             RefStackMv[ idx ][ list ] = refMv
         }
     }


```

<a id="s-7-12-3"></a>

#### § 7.12.3 Find warp samples process

```text
§   7.12.3. Find warp samples process

```

<a id="s-7-12-3-1"></a>

##### § 7.12.3.1 General

```text
§   7.12.3.1. General

    The input to this process is a variable ref specifying which set of candidate motion vectors to prepare.

    The process examines the neighboring inter predicted blocks and estimates a local warp transformation
    based on the motion vectors.




    AV2 Specification                                                                             Page 444 of 1169
The process produces a variable NumSamples containing the number of valid candidates found, and an
array CandList containing sorted candidates.

The variable NumSamples[ ref ] is set equal to 0.

The variable w4 specifying the width of the block in 4x4 luma samples is set equal to
Num_4x4_Blocks_Wide[ MiSize ].

The variable h4 specifying the height of the block in 4x4 luma samples is set equal to
Num_4x4_Blocks_High[ MiSize ].

The process is specified as:

 doTopLeft = 1
 doTopRight = 1
 if (AvailU) {
     colOffset = MiColBase[ 0 ][ MiRow - 1 ][ MiCol ] - MiCol
     if (colOffset < 0)
         doTopLeft = 0
     for (i = colOffset; i < Min( w4, MiCols - MiCol ); i += srcW) {
         srcSize = MiSizes[ 0 ][ MiRow - 1 ][ MiCol + i ]
         srcW = Num_4x4_Blocks_Wide[ srcSize ]
         if ( above_sample_stored( i ) ) {
             add_sample( ref, -1, i )
         }
     }
     doTopRight = (i == w4) && i < (MiCols - MiCol)
 }
 if (AvailL) {
     rowOffset = MiRowBase[ 0 ][ MiRow ][ MiCol - 1 ] - MiRow
     if (rowOffset < 0)
         doTopLeft = 0
     for (i = rowOffset; i < Min( h4, MiRows - MiRow); i += srcH) {
         srcSize = MiSizes[ 0 ][ MiRow + i ][ MiCol - 1 ]
         srcH = Num_4x4_Blocks_High[ srcSize ]
         add_sample( ref, i, -1 )
     }
 }
 if ( doTopLeft && above_sample_stored( -1 ) ) {
     add_sample( ref, -1, -1 )
 }
 if ( doTopRight && w4 <= 16 && above_sample_stored( w4 ) ) {
     add_sample( ref, -1, w4 )
 }


where the call to add_sample specifies that the add sample process in § 7.12.3.2 Add sample process is
invoked.

The function above_sample_stored (which checks whether the warp parameters for a particular above
location are available) is specified as follows:

 above_sample_stored( deltaCol ) {
     if ( !is_inside( MiRow - 1, MiCol + deltaCol ) ) {
         return 0
     }
     isSbBorder = ( MiRow & (Num_4x4_Blocks_High[ SbSize ] - 1) ) == 0
     if (!isSbBorder) {
         return 1
     }
     if ((MiCol + deltaCol) % 2 == 0) {
         return 1
     }




AV2 Specification                                                                          Page 445 of 1169
          srcW4 = Num_4x4_Blocks_Wide[ MiSizes[ 0 ][ MiRow - 1 ][ MiCol + deltaCol ] ]
          if (srcW4 == 1) {
              return 0
          }
          return MiCol + deltaCol + 1 < MiCols
     }


```

<a id="s-7-12-3-2"></a>

##### § 7.12.3.2 Add sample process

```text
§   7.12.3.2. Add sample process

    The inputs to this process are:

      • a variable ref specifying which set of candidate motion vectors to prepare,
      • a variable deltaRow specifying (in units of 4x4 luma samples) how far above to look for a motion
        vector,
      • a variable deltaCol specifying (in units of 4x4 luma samples) how far left to look for a motion vector.

    The output of this process is to add a new sample to the list of candidates if it is a valid candidate and has
    not been seen before.

    If NumSamples[ ref ] is greater than or equal to LEAST_SQUARES_SAMPLES_MAX, this process
    immediately terminates.

    The variable mvRow is set equal to MiRow + deltaRow.

    The variable mvCol is set equal to MiCol + deltaCol.

    If RefFrames[ mvRow ][ mvCol ][ 0 ] has not been written for this frame, this process immediately
    terminates.

    The candidates are added as follows:

     for(list=0;list<2;list++) {
         if ( RefFrames[ mvRow ][ mvCol ][ list ] == RefFrame[ ref ] ) {
             candSz = MiSizes[ PlaneStart ][ mvRow ][ mvCol ]
             candW4 = Num_4x4_Blocks_Wide[ candSz ]
             candH4 = Num_4x4_Blocks_High[ candSz ]
             candRow = MiRowBase[ 0 ][ mvRow ][ mvCol ]
             candCol = MiColBase[ 0 ][ mvRow ][ mvCol ]
             midY = candRow * 4 + candH4 * 2 - 1
             midX = candCol * 4 + candW4 * 2 - 1
             cand[ 0 ] = midY * 8
             cand[ 1 ] = midX * 8
             cand[ 2 ] = midY * 8 + Mvs[ candRow ][ candCol ][ list ][ 0 ]
             cand[ 3 ] = midX * 8 + Mvs[ candRow ][ candCol ][ list ][ 1 ]
             for ( i = 0; i < 4; i++ )
                 CandList[ ref ][ NumSamples[ ref ] ][ i ] = cand[ i ]
             NumSamples[ ref ]++
             if ( NumSamples[ ref ] >= LEAST_SQUARES_SAMPLES_MAX )
                 return
         }
     }



      NOTE: candRow and candCol give the top-left position of the candidate block in units of 4x4 blocks.
      midX and midY give the central position of the candidate block in units of luma samples.




    AV2 Specification                                                                              Page 446 of 1169
```

<a id="s-7-13"></a>

### § 7.13 Prediction processes

```text
§   7.13. Prediction processes
```

<a id="s-7-13-1"></a>

#### § 7.13.1 General

```text
§   7.13.1. General

    The following sections define the processes used for predicting the sample values.

    These processes are triggered at points defined by function calls to predict_intra, predict_inter,
    predict_chroma_from_luma, and predict_palette in the residual syntax table described in § 5.20.7.23
    Residual syntax.

```

<a id="s-7-13-2"></a>

#### § 7.13.2 Intra prediction process

```text
§   7.13.2. Intra prediction process

```

<a id="s-7-13-2-1"></a>

##### § 7.13.2.1 General

```text
§   7.13.2.1. General

    The intra prediction process is invoked for intra coded blocks to predict a part of the block corresponding
    to a transform block. When the transform size is smaller than the block size, this process can be invoked
    multiple times within a single block for the same plane, and the invocations are in raster scan order
    within the block.

    This process is triggered by a call to predict_intra.

    The inputs to this process are:

      • a variable plane specifying which plane is being predicted,
      • variables x and y specifying the location of the top left sample in the CurrFrame[ plane ] array of the
        current transform block,
      • a variable haveLeft that is equal to 1 if there are valid samples to the left of this transform block,
      • a variable haveAbove that is equal to 1 if there are valid samples above this transform block,
      • a variable num4AboveRight that specifies the number of valid samples (in units of 4x4 samples) above
        the transform block to the right of this transform block,
      • a variable num4BelowLeft that specifies the number of valid samples (in units of 4x4 samples) to the
        left of the transform block below this transform block,
      • a variable mode specifying the type of intra prediction to apply,
      • a variable log2W specifying the base 2 logarithm of the width of the region to be predicted,
      • a variable log2H specifying the base 2 logarithm of the height of the region to be predicted.

    The process makes use of the already reconstructed samples in the current frame CurrFrame to form a
    prediction for the current block.

    The outputs of this process are intra predicted samples in the current frame CurrFrame.

    The variable w is set equal to 1 << log2W.

    The variable h is set equal to 1 << log2H.

    The variable maxX is set equal to ( MiCols * MI_SIZE ) - 1.

    The variable maxY is set equal to ( MiRows * MI_SIZE ) - 1.

    If plane is greater than 0 and w is greater than 32, the variable num4AboveRight is set equal to 0.



    AV2 Specification                                                                               Page 447 of 1169
If plane is greater than 0 and h is greater than 32, the variable num4BelowLeft is set equal to 0.

The variable pxTopRight is set equal to 4 * num4AboveRight.

The variable pxBotLeft is set equal to 4 * num4BelowLeft.

If plane is greater than 0, then:

  • maxX is set equal to ( ( MiCols * MI_SIZE ) >> SubsamplingX ) - 1.
  • maxY is set equal to ( ( MiRows * MI_SIZE ) >> SubsamplingY ) - 1.

If is_inter is equal to 0 and plane is greater than 0 and UVMode is equal to UV_CFL_PRED and cfl_index
is equal to CFL_MULTI, the luma reference samples in the arrays CflRef are captured as follows:

 CflAbove = haveAbove ? 2 : 0
 CflLeft = haveLeft ? 2 : 0
 subX = SubsamplingX
 subY = SubsamplingY
 lumaW = w << subX
 lumaH = h << subY
 if (lumaW <= 4 || !haveAbove) {
     pxTopRight = 0
 }
 if (lumaH <= 4 || !haveLeft) {
     pxBotLeft = 0
 }
 rightLumaX = Min( MiColEnd * MI_SIZE, (x + w + pxTopRight) << subX)
 bottomLumaY = Min( MiRowEnd * MI_SIZE, (y + h + pxBotLeft ) << subY)
 sbRow = MiRow >> Mi_Height_Log2[ SbSize ]
 sbChromaY = ( sbRow * Block_Height[ SbSize ] ) >> subY
 CflRefWidth = Min((CflLeft << subX) + (rightLumaX - (x << subX)), 128)
 CflRefHeight = Min((CflAbove << subY) + (bottomLumaY - (y << subY)), 128)
 for( i = 0; i < h + CflAbove; i++ ) {
     for( j = 0; j < w + CflLeft ; j++ ) {
         CflRef[ 0 ][ i ][ j ] = 0
     }
 }
 for( i = 0; i < Round2( CflRefHeight, subY ); i++ ) {
     for( j = 0; j < Round2( CflRefWidth, subX ); j++ ) {
         chromaX = x + j - CflLeft
         chromaY = y + i - CflAbove
         if ( i < CflAbove || j < CflLeft ) {
             CflRef[ 1 ][ i ][ j ] =
                 CurrFrame[ plane ][ Max(chromaY, sbChromaY - 1) ][ chromaX ]
         }
         if ( cfl_ref_luma_avail(i,j,w,h) ) {
             CflRef[ 0 ][ i ][ j ] =
                 get_cfl_luma_sample( chromaX, chromaY, j == 0, i == 0 )
         }
     }
 }


where the function get_cfl_luma_sample (which gets an estimate of luma corresponding to the chroma
location) is defined as:

 get_cfl_luma_sample(chromaX,chromaY,clampX,clampY) {
     lumaX = chromaX << SubsamplingX
     lumaY = chromaY << SubsamplingY
     sbRow = MiRow >> Mi_Height_Log2[ SbSize ]
     limitLumaY = sbRow * Block_Height[ SbSize ] - 1
     filterIdx = cfl_ds_filter_index
     if (filterIdx == 3) {
         filterIdx = 0



AV2 Specification                                                                            Page 448 of 1169
      }
      t = 0
      subX = SubsamplingX
      subY = SubsamplingY
      for (dy = -subY; dy <= subY; dy++) {
          for (dx = -subX; dx <= subX; dx++) {
               v = CurrFrame[0]
                             [Max( limitLumaY,lumaY + (clampY ? Max(0, dy) : dy) )]
                             [lumaX + (clampX ? Max(0, dx) : dx)]
               if (subX && subY) {
                   t += Cfl_Filters_420[filterIdx][dy + subY][dx + subX] * v
               } else if (subX) {
                   t += Cfl_Filters_422[filterIdx][dx + subX] * v
               } else {
                   t = 8 * v
               }
          }
      }
      r = t >> 3
      return r
 }


The variable MrlIndex and the arrays AboveRow and LeftCol are prepared as follows:

 MrlIndex = (plane == 0) ? mrl_index : 0
 sbHeight = Block_Height[ SbSize ]
 sbBoundary = (y & (sbHeight - 1)) == 0
 aboveMrlIndex = sbBoundary ? 0 : MrlIndex
 useDip = plane == 0 && use_dip
 if ( useDip ) {
     numAboveNeeded = w + (w >> 2)
     numLeftNeeded = h + (h >> 2)
 } else {
     numAboveNeeded = w + h + (MrlIndex << 1)
     numLeftNeeded = w + h + (MrlIndex << 1)
 }
 for ( i = 0; i < numLeftNeeded; i++ ) {
     if ( haveLeft == 0 && haveAbove == 1 ) {
          LeftCol[ i ] = CurrFrame[ plane ][ y - 1 - aboveMrlIndex ][ x ]
          LeftSecCol[ i ] = CurrFrame[ plane ][ y - 1 ][ x ]
     } else if ( haveLeft == 0 ) {
          LeftCol[ i ] = ( 1 << ( BitDepth - 1 ) ) + 1
          LeftSecCol[ i ] = ( 1 << ( BitDepth - 1 ) ) + 1
     } else {
          leftLimit = Min( maxY, y + h + 4 * num4BelowLeft - 1 )
          LeftCol[ i ] =
              CurrFrame[ plane ][ Min(leftLimit, y+i) ][ x - 1 - MrlIndex ]
          LeftSecCol[ i ] = CurrFrame[ plane ][ Min(leftLimit, y+i) ][ x - 1 ]
     }
 }
 for ( i = 0; i < numAboveNeeded; i++ ) {
     if ( haveAbove == 0 && haveLeft == 1 ) {
          AboveRow[ i ] = CurrFrame[ plane ][ y ][ x - 1 - MrlIndex ]
          AboveSecRow[ i ] = CurrFrame[ plane ][ y ][ x - 1 ]
     } else if ( haveAbove == 0 ) {
          AboveRow[ i ] = ( 1 << ( BitDepth - 1 ) ) - 1
          AboveSecRow[ i ] = ( 1 << ( BitDepth - 1 ) ) - 1
     } else {
          aboveLimit = Min( maxX, x + w + 4 * num4AboveRight - 1 )
          AboveRow[ i ] =
              CurrFrame[ plane ][ y - 1 - aboveMrlIndex ][ Min(aboveLimit, x+i) ]
          AboveSecRow[ i ] = CurrFrame[ plane ][ y - 1 ][ Min(aboveLimit, x+i) ]
     }
 }
 for ( i = 1; i <= 1 + MrlIndex; i++) {
     if ( haveAbove == 1 && haveLeft == 1 ) {
          AboveRow[ -i ] = CurrFrame[ plane ][ y - 1 - aboveMrlIndex ][ x - i ]
          LeftCol[ -i ] = CurrFrame[ plane ][ y - Min(i, 1 + aboveMrlIndex) ]
                                   [ x - 1 - MrlIndex ]



AV2 Specification                                                                     Page 449 of 1169
          AboveSecRow[ -i ] = CurrFrame[ plane ][ y - 1 ][ x - i ]
          LeftSecCol[ -i ] = CurrFrame[ plane ][ y - 1 ][ x - 1 ]
      } else if ( haveAbove == 1 ) {
          AboveRow[ -i ] = CurrFrame[ plane ][ y - 1 - aboveMrlIndex ][ x ]
          LeftCol[ -i ] = AboveRow[ -i ]
          AboveSecRow[ -i ] = CurrFrame[ plane ][ y - 1 ][ x ]
          LeftSecCol[ -i ] = AboveSecRow[ -i ]
      } else if ( haveLeft == 1 ) {
          AboveRow[ -i ] = CurrFrame[ plane ][ y ][ x - 1 - MrlIndex ]
          LeftCol[ -i ] = AboveRow[ -i ]
          AboveSecRow[ -i ] = CurrFrame[ plane ][ y ][ x - 1 ]
          LeftSecCol[ -i ] = AboveSecRow[ -i ]
      } else {
          AboveRow[ -i ] = 1 << ( BitDepth - 1 )
          LeftCol[ -i ] = 1 << ( BitDepth - 1 )
          AboveSecRow[ -i ] = 1 << ( BitDepth - 1 )
          LeftSecCol[ -i ] = 1 << ( BitDepth - 1 )
      }
 }


The variable largeChroma is set as follows:

  • If plane is equal to 0, largeChroma is set equal to 0.
  • Otherwise, if w is greater than 32 or h is greater than 32, largeChroma is set equal to 1.
  • Otherwise, largeChroma is set equal to 0.

A 2D array named pred containing the intra predicted samples is constructed as follows:

  • If useDip is equal to 1, the data driven intra prediction process specified in § 7.13.2.3 Data driven
    intra prediction process is invoked with w and h as inputs and the output is assigned to pred.
  • Otherwise, if is_directional_mode( mode ) is equal to 1, the directional intra prediction process
    specified in § 7.13.2.7 Directional intra prediction process is invoked with plane, x, y, haveLeft,
    haveAbove, mode, w, h, maxX, maxY as inputs and the output is assigned to pred.
  • Otherwise, if mode is equal to SMOOTH_PRED or SMOOTH_V_PRED or SMOOTH_H_PRED, the
    smooth intra prediction process specified in § 7.13.2.13 Smooth intra prediction process is invoked
    with mode, log2W, log2H, w, and h as inputs, and the output is assigned to pred.
  • Otherwise, if largeChroma is equal to 1 and mode is equal to DC_PRED and is_inter is equal to 0 and
    UVMode is equal to UV_CFL_PRED, the DC intra prediction subsampled process specified in
    § 7.13.2.11 DC intra prediction subsampled process is invoked with haveLeft, haveAbove, log2W, and
    log2H as inputs and the output is assigned to pred.
  • Otherwise, if mode is equal to DC_PRED, the DC intra prediction process specified in § 7.13.2.10 DC
    intra prediction process is invoked with haveLeft, haveAbove, log2W, and log2H as inputs and the
    output is assigned to pred.
  • Otherwise (mode is equal to PAETH_PRED), the basic intra prediction process specified in § 7.13.2.2
    Basic intra prediction process is invoked with w, and h as inputs, and the output is assigned to pred.

If all of the following conditions are true, the IBP DC process (which modifies pred) specified in
§ 7.13.2.12 IBP DC process is invoked with haveLeft, haveAbove, log2W, log2H, w, h, and pred as inputs:

  • enable_ibp is equal to 1.
  • useDip is equal to 0.
  • mode is equal to DC_PRED.



AV2 Specification                                                                                Page 450 of 1169
      • w is not equal to 4 or h is not equal to 4.
      • plane is equal to 0 or UVMode is not equal to UV_CFL_PRED.

    The current frame is updated as follows:

      • CurrFrame[ plane ][ y + i ][ x + j ] is set equal to pred[ i ][ j ] for i = 0..h-1 and j = 0..w-1.

```

<a id="s-7-13-2-2"></a>

##### § 7.13.2.2 Basic intra prediction process

```text
§   7.13.2.2. Basic intra prediction process

    The inputs to this process are:

      • a variable w specifying the width of the region to be predicted,
      • a variable h specifying the height of the region to be predicted.

    The output of this process is a 2D array named pred containing the intra predicted samples.

    The process generates filtered samples from the samples in LeftCol and AboveRow as follows:

      • The following ordered steps apply for i = 0..h-1, for j = 0..w-1:

          1. The variable base is set equal to AboveRow[ j ] + LeftCol[ i ] - AboveRow[ -1 ].
          2. The variable pLeft is set equal to Abs( base - LeftCol[ i ]).
          3. The variable pTop is set equal to Abs( base - AboveRow[ j ]).
          4. The variable pTopLeft is set equal to Abs( base - AboveRow[ -1 ] ).
          5. The predicted sample is computed as follows:

                ▪ If pLeft is less than or equal to pTop and pLeft is less than or equal to pTopLeft, pred[ i ][ j ] is
                  set equal to LeftCol[ i ].
                ▪ Otherwise, if pTop is less than or equal to pTopLeft, pred[ i ][ j ] is set equal to AboveRow[ j ].
                ▪ Otherwise, pred[ i ][ j ] is set equal to AboveRow[ -1 ].

    The output of the process is the array pred.

```

<a id="s-7-13-2-3"></a>

##### § 7.13.2.3 Data driven intra prediction process

```text
§   7.13.2.3. Data driven intra prediction process

    The inputs to this process are:

      • a variable w specifying the width of the region to be predicted,
      • a variable h specifying the height of the region to be predicted.

    The output of this process is a 2D array named pred containing the intra predicted samples.

    The following ordered steps apply:

     1. The DIP features process specified in § 7.13.2.4 DIP features process is invoked with w and h as
        inputs, and the output is assigned to f.
     2. The DIP transform process specified in § 7.13.2.5 DIP transform process is invoked with f as input,
        and the output is assigned to dipPred.




    AV2 Specification                                                                                   Page 451 of 1169
     3. The DIP resample process specified in § 7.13.2.6 DIP resample process is invoked with w, h, and
        dipPred as inputs, and the output is assigned to pred.

```

<a id="s-7-13-2-4"></a>

##### § 7.13.2.4 DIP features process

```text
§   7.13.2.4. DIP features process

    The inputs to this process are:

      • a variable w specifying the width of the region to be predicted,
      • a variable h specifying the height of the region to be predicted.

    The output of this process is a 1D array named f containing 11 features extracted from previously
    decoded samples in the current frame.

    The features are prepared as follows:

     f[ 0 ] = AboveRow[-1]
     fAbove = dip_avg( 0, w )
     fLeft = dip_avg( 1, h )
     for(i = 0; i < 4; i++) {
         f[ i + 1 ] = dip_transpose ? fLeft[ i ] : fAbove[ i ]
         f[ i + 5 ] = dip_transpose ? fAbove[ i ] : fLeft[ i ]
     }
     f[ 9 ] = dip_transpose ? fLeft[ 4 ] : fAbove[ 4 ]
     f[ 10 ] = dip_transpose ? fAbove[ 4 ] : fLeft[ 4 ]


    where the function dip_avg downsamples the previously decoded samples as follows:

     dip_avg( dir, n ) {
         down = n >> 2
         for( i = 0; i < 5; i++) {
             t = 0
             for ( j = 0; j < down; j++ ) {
                  t += dir ? LeftCol[ i * down + j ] : AboveRow[ i * down + j ]
             }
             f[ i ] = ( t + (down >> 1) ) / down
         }
         return f
     }


```

<a id="s-7-13-2-5"></a>

##### § 7.13.2.5 DIP transform process

```text
§   7.13.2.5. DIP transform process

    The input to this process is an array of 11 features named f.

    The output of this process is an 8 by 8 2D array pred of the predicted samples.

    The prediction is formed as follows:

     for( i = 0; i < 8; i++ ) {
         for( j = 0; j < 8; j++ ) {
             c = 0
             for( k = 0; k < 11; k++ ) {
                 c += Dip_Weights[ dip_mode ][ i * 8 + j ][ k ] * f[ k ]
             }
             v = Clip1( Round2( c, 10 ) )
             if ( dip_transpose ) {
                 pred[ j ][ i ] = v
             } else {
                 pred[ i ][ j ] = v




    AV2 Specification                                                                          Page 452 of 1169
               }
          }
     }


```

<a id="s-7-13-2-6"></a>

##### § 7.13.2.6 DIP resample process

```text
§   7.13.2.6. DIP resample process

    The inputs to this process are:

      • a variable w specifying the width of the region to be predicted,
      • a variable h specifying the height of the region to be predicted,
      • an 8 by 8 array dipPred of predicted samples.

    The output of this process is the 2D array pred containing the predicted samples resampled to a size of w
    by h.

    The samples are formed as follows:

     upx = Max(1, w / 8)
     upy = Max(1, h / 8)
     downx = Max(1, 8 / w)
     downy = Max(1, 8 / h)
     for( i = 0; i < Min(h, 8); i++ ) {
         y = (i + 1) * upy - 1
         for( j = 0; j < Min(w, 8); j++) {
             p0 = j == 0 ? LeftCol[ y ] : dipPred[ i * downy ][ (j - 1) * downx ]
             p1 = dipPred[ i * downy ][ j * downx ]
             for( k = 0; k < upx; k++) {
                 x = j * upx + k
                 w1 = k + 1
                 horzInterp[ i ][ x ] = ( (upx - w1) * p0 + w1 * p1 ) / upx
             }
         }
     }
     for( x = 0; x < w; x++) {
         for( i = 0; i < Min(h, 8); i++) {
             p0 = i == 0 ? AboveRow[ x ] : horzInterp[ i - 1 ][ x ]
             p1 = horzInterp[ i ][ x ]
             for( k = 0; k < upy; k++) {
                 y = i * upy + k
                 w1 = k + 1
                 pred[ y ][ x ] = ( (upy - w1) * p0 + w1 * p1 ) / upy
             }
         }
     }


```

<a id="s-7-13-2-7"></a>

##### § 7.13.2.7 Directional intra prediction process

```text
§   7.13.2.7. Directional intra prediction process

    The inputs to this process are:

      • a variable plane specifying which plane is being predicted,
      • variables x and y specifying the location of the top left sample in the CurrFrame[ plane ] array of the
        current transform block,
      • a variable haveLeft that is equal to 1 if there are valid samples to the left of this transform block,
      • a variable haveAbove that is equal to 1 if there are valid samples above this transform block,
      • a variable mode specifying the type of intra prediction to apply,
      • a variable w specifying the width of the region to be predicted,



    AV2 Specification                                                                               Page 453 of 1169
  • a variable h specifying the height of the region to be predicted,
  • a variable maxX specifying the largest valid x coordinate for the current plane,
  • a variable maxY specifying the largest valid y coordinate for the current plane.

The output of this process is a 2D array containing the intra predicted samples.

The process uses a directional filter to generate filtered samples from the samples in LeftCol and
AboveRow.

The variable angleDelta is derived as follows:

  • If plane is equal to 0, angleDelta is set equal to AngleDeltaY.
  • Otherwise (plane is not equal to 0), angleDelta is set equal to AngleDeltaUV.

The variable pAngle is derived by the following ordered steps:

 1. The variable pAngle is set equal to ( Mode_To_Angle[ mode ] + angleDelta * ANGLE_STEP +
    Mrl_Index_To_Delta[ MrlIndex ] ).
 2. If is_inter is equal to 0 (meaning we are not using inter intra prediction), the variable pAngle is
    modified as follows:

      (unusedMode, pAngle) = wide_angle_mapping(mode, w, h, pAngle)


The variable not4x4 is set equal to ( w!=4 || h!=4 ).

The variable applyIbp is set equal to enable_ibp && not4x4.

The following ordered steps (which prepare filtered edge samples) apply:

 1. If enable_intra_edge_filter is equal to 1 and MrlIndex is equal to 0, the following applies:

      filterTypeAbove = 0
      filterTypeLeft = 0
      angleAbove = pAngle - 90
      angleLeft = pAngle - 180
      needRight = pAngle < 90
      needBottom = pAngle > 180
      if ( pAngle != 90 && pAngle != 180 ) {
          filterTypeAbove = get_filter_type_above( plane )
          filterTypeLeft = get_filter_type_left( plane )
          if ( applyIbp ) {
              needRight |= pAngle > 180
              needBottom |= pAngle < 90
              if (angleAbove > 90) {
                   angleAbove -= 180
              }
              if (angleLeft < -90) {
                   angleLeft += 180
              }
          } else {
              filterType = filterTypeAbove | filterTypeLeft
              filterTypeAbove = filterType
              filterTypeLeft = filterType
          }
          if ( ( applyIbp || (pAngle > 90 && pAngle < 180) ) && ( w + h ) >= 24 ) {
              LeftCol[ -1 ] = filter_corner( )
              AboveRow[ -1 ] = LeftCol[ -1 ]




AV2 Specification                                                                              Page 454 of 1169
           }
           if ( haveAbove == 1 ) {
               strength = intra_edge_filter_strength_selection( w, h, filterTypeAbove, angleAbove )
               numPx = Min( w, ( maxX - x + 1 ) ) + ( needRight ? h : 0 ) + 1
               intra_edge_filter( numPx, strength, 0 )
           }
           if ( haveLeft == 1 ) {
               strength = intra_edge_filter_strength_selection( w, h, filterTypeLeft, angleLeft )
               numPx = Min( h, ( maxY - y + 1 ) ) + ( needBottom ? w : 0 ) + 1
               intra_edge_filter( numPx, strength, 1 )
           }
      }




    The call of get_filter_type_above indicates that the intra filter type above process specified in
    § 7.13.2.15 Intra filter type above process is invoked.

    The call of get_filter_type_left indicates that the intra filter type left process specified in § 7.13.2.16
    Intra filter type left process is invoked.

    The call of intra_edge_filter_strength_selection indicates that the intra edge filter strength selection
    process specified in § 7.13.2.17 Intra edge filter strength selection process is invoked.

    The call of intra_edge_filter indicates that the intra edge filter process specified in § 7.13.2.18 Intra
    edge filter process is invoked.
 2. The single directional prediction process specified in § 7.13.2.8 Single directional prediction process
    is invoked with pAngle, w, h, MrlIndex, and plane as inputs, and the output is assigned to pred.
 3. If MrlIndex is greater than 0 and mrl_sec_index is equal to 1 and not4x4 is equal to 1, the following
    ordered steps apply:

      1. LeftCol is set equal to a copy of LeftSecCol.
      2. AboveRow is set equal to a copy of AboveSecRow.
      3. The single directional prediction process specified in § 7.13.2.8 Single directional prediction
         process is invoked with pAngle, w, h, 0, and plane as inputs, and the output is assigned to pred2.
      4. Set combinedPred[r][c] equal to ( pred[r][c] + pred2[r][c] + 1 ) >> 1 for r = 0..h-1 and c = 0..w-1.
      5. The process terminates immediately with combinedPred as output.

The constant table Mrl_Index_To_Delta is defined as follows:

 Mrl_Index_To_Delta[4] = {
     0, 1, -1, 0
 }


The variable useIBP is set equal to 1 if all of the following conditions are true, otherwise, useIBP is set
equal to 0:

  • applyIbp is equal to 1.
  • angleDelta is even.
  • plane is equal to 0.
  • pAngle is less than 90 or pAngle is greater than 180.
  • MrlIndex is equal to 0.


AV2 Specification                                                                                 Page 455 of 1169
    If useIBP is equal to 0, this process immediately terminates with pred as output.

    Otherwise, the weights and secondAngle are computed as follows:

     if (pAngle < 90) {
         weights = ibp_weights(pAngle)
         secondAngle = pAngle + 180
     } else {
         weights = ibp_weights(270 - pAngle)
         secondAngle = pAngle - 180
     }


    The call of ibp_weights indicates that the IBP weights process specified in § 7.13.2.9 IBP weights process is
    invoked.

    The single directional prediction process specified in § 7.13.2.8 Single directional prediction process is
    invoked with secondAngle, w, h, MrlIndex, and plane as inputs, and the output is assigned to secondPred.

    The combined prediction is formed as a weighted blend of the two predictions as follows:

     cShift = w >> (IBP_WEIGHT_SIZE_LOG2 + 1)
     rShift = h >> (IBP_WEIGHT_SIZE_LOG2 + 1)
     for (r = 0; r < h; r++) {
         for (c = 0; c < w; c++) {
             s = pAngle < 90 ? weights[r >> rShift][c >> cShift] :
                               weights[c >> cShift][r >> rShift]
             combinedPred[r][c] = Round2( pred[r][c] * s +
                                          secondPred[r][c] * (IBP_WEIGHT_MAX - s),
                                          IBP_WEIGHT_SHIFT)
         }
     }


    The output of the process is the array combinedPred.

```

<a id="s-7-13-2-8"></a>

##### § 7.13.2.8 Single directional prediction process

```text
§   7.13.2.8. Single directional prediction process

    The inputs to this process are:

      • a variable pAngle specifying the angle to use for directional prediction,
      • a variable w specifying the width of the region to be predicted,
      • a variable h specifying the height of the region to be predicted,
      • a variable mrlIndex specifying the distance of the edge samples from the block,
      • a variable plane specifying whether to use IDIF filtering.

    The output of this process is a 2D array named pred containing the intra predicted samples.

    The variable enableIdif is set equal to plane == 0.

    If enableIdif is equal to 1, the following applies:

     minBase = -(1 + mrlIndex)
     maxBase = w + h - 1 + (mrlIndex << 1)
     if ( pAngle > 90 && pAngle < 180 ) {
         LeftCol[h] = LeftCol[h - 1]
         AboveRow[w] = AboveRow[w - 1]
         LeftCol[h + 1] = LeftCol[h - 1]



    AV2 Specification                                                                              Page 456 of 1169
     AboveRow[w + 1] = AboveRow[w - 1]
 } else {
     LeftCol[maxBase + 1] = LeftCol[maxBase]
     AboveRow[maxBase + 1] = AboveRow[maxBase]
     LeftCol[maxBase + 2] = LeftCol[maxBase]
     AboveRow[maxBase + 2] = AboveRow[maxBase]
 }
 LeftCol[minBase - 1] = LeftCol[minBase]
 AboveRow[minBase - 1] = AboveRow[minBase]


 1. If pAngle is less than 90, the following steps apply for i = 0..h-1, for j = 0..w-1:

       ◦ The variable dx is set equal to Dr_Intra_Derivative[ pAngle ].
       ◦ The variable idx is set equal to ( i + 1 + mrlIndex ) * dx.
       ◦ The variable base is set equal to (idx >> 6 ) + j.
       ◦ The variable shift is set equal to ( idx >> 1 ) & 0x1F.
       ◦ The variable maxBaseX is set equal to (w + h - 1 + (mrlIndex << 1) ).
       ◦ If base is less than maxBaseX + enableIdif, the samples are filtered as follows:

           if ( enableIdif ) {
               s = 0
               for(t = 0 ; t < 4; t++) {
                    s += Dr_Interp_Filter[ shift ][ t ] * AboveRow[ base + t - 1 ]
               }
               pred[ i ][ j ] = Clip1( Round2( s, 7 ) )
           } else {
               pred[ i ][ j ] = Round2( AboveRow[ base ] * ( 32 - shift ) + AboveRow[ base + 1 ] * shift, 5 )
           }


       ◦ Otherwise (base is greater than or equal to maxBaseX + enableIdif), pred[ i ][ j ] is set equal to
         AboveRow[ maxBaseX ].
 2. Otherwise, if pAngle is greater than 90 and pAngle is less than 180, the following steps apply for i =
    0..h-1, for j = 0..w-1:

       ◦ The variable dx is set equal to Dr_Intra_Derivative[ 180 - pAngle ].
       ◦ The variable dy is set equal to Dr_Intra_Derivative[ pAngle - 90 ].
       ◦ The variable idx is set equal to ( j << 6 ) - ( i + 1 + mrlIndex) * dx.
       ◦ The variable base is set equal to idx >> 6 .
       ◦ If base is greater than or equal to -(1 + mrlIndex), the following steps apply:

            ▪ The variable shift is set equal to ( idx >> 1 ) & 0x1F.
            ▪ The samples are filtered as follows:


                if ( enableIdif ) {
                    s = 0
                    for(t = 0 ; t < 4; t++) {
                         s += Dr_Interp_Filter[ shift ][ t ] * AboveRow[ base + t - 1 ]
                    }
                    pred[ i ][ j ] = Clip1( Round2( s, 7 ) )
                } else {




AV2 Specification                                                                                 Page 457 of 1169
                      pred[ i ][ j ] = Round2( AboveRow[ base ] * ( 32 - shift ) + AboveRow[ base + 1 ] * shift,
                5 )
                }


       ◦ Otherwise, the following steps apply:

            ▪ The variable idx is set equal to ( i << 6 ) - ( j + 1 + mrlIndex ) * dy.
            ▪ The variable base is set equal to idx >> 6.
            ▪ The variable shift is set equal to ( idx >> 1 ) & 0x1F.
            ▪ The samples are filtered as follows:

                if ( enableIdif ) {
                    s = 0
                    for(t = 0 ; t < 4; t++) {
                         s += Dr_Interp_Filter[ shift ][ t ] * LeftCol[ base + t - 1 ]
                    }
                    pred[ i ][ j ] = Clip1( Round2( s, 7 ) )
                } else {
                    pred[ i ][ j ] = Round2( LeftCol[ base ] * ( 32 - shift ) + LeftCol[ base + 1 ] * shift, 5 )
                }


 3. Otherwise, if pAngle is greater than 180, the following steps apply for i = 0..h-1, for j = 0..w-1:

       ◦ The variable dy is set equal to Dr_Intra_Derivative[ 270 - pAngle ].
       ◦ The variable idx is set equal to ( j + 1 + mrlIndex ) * dy.
       ◦ The variable base is set equal to ( idx >> 6 ) + i.
       ◦ The variable shift is set equal to ( idx >> 1 ) & 0x1F.
       ◦ The variable maxBaseY is set equal to (w + h - 1 + (mrlIndex << 1)).
       ◦ If base is less than maxBaseY + enableIdif, the samples are filtered as follows:

           if ( enableIdif ) {
               s = 0
               for(t = 0 ; t < 4; t++) {
                    s += Dr_Interp_Filter[ shift ][ t ] * LeftCol[ base + t - 1 ]
               }
               pred[ i ][ j ] = Clip1( Round2( s, 7 ) )
           } else {
               pred[ i ][ j ] = Round2( LeftCol[ base ] * ( 32 - shift ) + LeftCol[ base + 1 ] * shift, 5 )
           }


       ◦ Otherwise (base is greater than or equal to maxBaseY + enableIdif), pred[ i ][ j ] is set equal to
         LeftCol[ maxBaseY ].
 4. Otherwise, if pAngle is equal to 90, pred[ i ][ j ] is set equal to AboveRow[ j ] with j = 0..w-1 and i =
    0..h-1 (each row of the block is filled with a copy of AboveRow).
 5. Otherwise, if pAngle is equal to 180, pred[ i ][ j ] is set equal to LeftCol[ i ] with j = 0..w-1 and i =
    0..h-1 (each column of the block is filled with a copy of LeftCol).

The output of the process is the array pred.




AV2 Specification                                                                                    Page 458 of 1169
    The filter taps in the constant table Dr_Interp_Filter (used when enableIdif is equal to 1) are defined as:

     Dr_Interp_Filter[ 32 ][ 4 ] = {
         { 0, 128, 0, 0 },     { -2, 127, 4, -1 },   { -3, 125, 8, -2 },
         { -5, 123, 13, -3 }, { -6, 121, 17, -4 }, { -7, 118, 22, -5 },
         { -9, 116, 27, -6 }, { -9, 112, 32, -7 }, { -10, 109, 37, -8 },
         { -11, 106, 41, -8 }, { -11, 102, 46, -9 }, { -12, 98, 52, -10 },
         { -12, 94, 56, -10 }, { -12, 90, 61, -11 }, { -12, 85, 66, -11 },
         { -12, 81, 71, -12 }, { -12, 76, 76, -12 }, { -12, 71, 81, -12 },
         { -11, 66, 85, -12 }, { -11, 61, 90, -12 }, { -10, 56, 94, -12 },
         { -10, 52, 98, -12 }, { -9, 46, 102, -11 }, { -8, 41, 106, -11 },
         { -8, 37, 109, -10 }, { -7, 32, 112, -9 }, { -6, 27, 116, -9 },
         { -5, 22, 118, -7 }, { -4, 17, 121, -6 }, { -3, 13, 123, -5 },
         { -2, 8, 125, -3 },   { -1, 4, 127, -2 }
     }


```

<a id="s-7-13-2-9"></a>

##### § 7.13.2.9 IBP weights process

```text
§   7.13.2.9. IBP weights process

    The input to this process is a variable pAngle specifying the angle to use for directional prediction.

    The output of this process is a 2D array named weights containing the blending weights.

    The array weights is computed as follows:

     pAngle = Max( 39, pAngle )
     dy = Dr_Intra_Derivative[90 - pAngle]
     for (r = 0; r < IBP_WEIGHT_SIZE; r++) {
         y = dy
         for (c = 0; c < IBP_WEIGHT_SIZE; c++) {
             dist = ((r + 1) << 6) + y
             (shift, div) = resolve_divisor(dist)
             shift -= DIV_LUT_BITS
             weight0 = Round2(y * div, shift)
             weights[r][c] = weight0
             y += dy
         }
     }


    The output of the process is the array weights.

```

<a id="s-7-13-2-10"></a>

##### § 7.13.2.10 DC intra prediction process

```text
§   7.13.2.10. DC intra prediction process

    The inputs to this process are:

      • a variable haveLeft that is equal to 1 if there are valid samples to the left of this transform block,
      • a variable haveAbove that is equal to 1 if there are valid samples above this transform block,
      • a variable log2W specifying the base 2 logarithm of the width of the region to be predicted,
      • a variable log2H specifying the base 2 logarithm of the height of the region to be predicted.

    The output of this process is a 2D array named pred containing the intra predicted samples.

    The variable w is set equal to 1 << log2W.

    The variable h is set equal to 1 << log2H.




    AV2 Specification                                                                               Page 459 of 1169
    The process averages the available edge samples in LeftCol and AboveRow to generate the prediction as
    follows:

      • If haveLeft is equal to 1 and haveAbove is equal to 1, pred[ i ][ j ] is set equal to avg with i = 0..h-1
        and j = 0..w-1. The variable avg (the average of the samples in union of AboveRow and LeftCol) is
        specified as follows:

          sum = 0
          for ( k = 0; k < h; k++ )
              sum += LeftCol[ k ]
          for ( k = 0; k < w; k++ )
              sum += AboveRow[ k ]

          avg = Clip1( approx_divide(sum, w + h) )


      • Otherwise, if haveLeft is equal to 1 and haveAbove is equal to 0, pred[ i ][ j ] is set equal to leftAvg
        with i = 0..h-1 and j = 0..w-1. The variable leftAvg is specified as follows:

          sum = 0
          for ( k = 0; k < h; k++ ) {
              sum += LeftCol[ k ]
          }
          leftAvg = Round2( sum, log2H )


      • Otherwise, if haveLeft is equal to 0 and haveAbove is equal to 1, pred[ i ][ j ] is set equal to aboveAvg
        with i = 0..h-1 and j = 0..w-1. The variable aboveAvg is specified as follows:

          sum = 0
          for ( k = 0; k < w; k++ ) {
              sum += AboveRow[ k ]
          }
          aboveAvg = Round2( sum, log2W )


      • Otherwise (haveLeft is equal to 0 and haveAbove is equal to 0), pred[ i ][ j ] is set equal to 1 <<
        ( BitDepth - 1 ) with i = 0..h-1 and j = 0..w-1.


    The output of the process is the array pred.

```

<a id="s-7-13-2-11"></a>

##### § 7.13.2.11 DC intra prediction subsampled process

```text
§   7.13.2.11. DC intra prediction subsampled process

    The inputs to this process are:

      • a variable haveLeft that is equal to 1 if there are valid samples to the left of this transform block,
      • a variable haveAbove that is equal to 1 if there are valid samples above this transform block,
      • a variable log2W specifying the base 2 logarithm of the width of the region to be predicted,
      • a variable log2H specifying the base 2 logarithm of the height of the region to be predicted.

    The output of this process is a 2D array named pred containing the intra predicted samples.

    The variable w is set equal to 1 << log2W.

    The variable h is set equal to 1 << log2H.




    AV2 Specification                                                                                 Page 460 of 1169
    The process averages the available edge samples in LeftCol and AboveRow to generate the prediction as
    follows:

     sum = 0
     count = 0
     if ( haveLeft ) {
         stepH = h > 32 ? 2 : 1
         for ( k = 0; k < h; k += stepH ) {
              sum += LeftCol[ k ]
              count++
         }
     }
     if ( haveAbove ) {
         stepW = w > 32 ? 2 : 1
         for ( k = 0; k < w; k += stepW ) {
              sum += AboveRow[ k ]
              count++
         }
     }
     if ( count == 0 ) {
         avg = 1 << (BitDepth - 1)
     } else {
         avg = Clip1( approx_divide(sum, count) )
     }
     for ( i = 0; i < h; i++ )
         for ( j = 0; j < w; j++ )
              pred[ i ][ j ] = avg


    where approx_divide approximates the division of sum by count and is specified as:

     approx_divide(num, den) norange {
         (shift, scale) = resolve_divisor(den)
         return Round2(num * scale, shift)
     }



      NOTE: The divide is only approximate so the average value computed by approx_divide needs to be
      clipped so that the predicted value fits within BitDepth bits.

```

<a id="s-7-13-2-12"></a>

##### § 7.13.2.12 IBP DC process

```text
§   7.13.2.12. IBP DC process

    The inputs to this process are:

      • a variable haveLeft that is equal to 1 if there are valid samples to the left of this transform block,
      • a variable haveAbove that is equal to 1 if there are valid samples above this transform block,
      • a variable log2W specifying the base 2 logarithm of the width of the region to be predicted,
      • a variable log2H specifying the base 2 logarithm of the height of the region to be predicted,
      • a variable w specifying the width of the region to be predicted,
      • a variable h specifying the height of the region to be predicted,
      • an array pred containing the DC predicted samples.

    This process modifies the intra predicted samples in the array pred as follows:

     if (haveAbove) {
         for (r = 0; r < (h >> 2); r++) {
             for (c = (w < h && haveLeft) ? w >> 2 : 0; c < w; c++) {



    AV2 Specification                                                                               Page 461 of 1169
                    s = Ibp_Weights[log2H - 2][r]
                    pred[ r ][ c ] = Round2( AboveRow[c] * (IBP_WEIGHT_MAX - s) +
                                             pred[ r ][ c ] * s, IBP_WEIGHT_SHIFT )
               }
          }
     }
     if (haveLeft) {
         for (r = (w >= h && haveAbove) ? h >> 2 : 0; r < h; r++) {
             for (c = 0; c < (w >> 2); c++) {
                 s = Ibp_Weights[log2W - 2][c]
                 pred[ r ][ c ] = Round2( LeftCol[r] * (IBP_WEIGHT_MAX - s) +
                                          pred[ r ][ c ] * s, IBP_WEIGHT_SHIFT )
             }
         }
     }


    where the constant table Ibp_Weights is defined as:

     Ibp_Weights[ 5 ][ 16 ] = {
         { 96, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0 },
         { 86, 107, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0 },
         { 77, 90, 102, 115, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0 },
         { 71, 78, 86, 92, 100, 107, 114, 121, 0, 0, 0, 0, 0, 0, 0, 0 },
         { 68, 72, 76, 79, 83, 87, 90, 94, 98, 102, 106, 109, 113, 117, 121, 124 }
     }


```

<a id="s-7-13-2-13"></a>

##### § 7.13.2.13 Smooth intra prediction process

```text
§   7.13.2.13. Smooth intra prediction process

    The inputs to this process are:

      • a variable mode specifying the type of intra prediction to apply,
      • a variable log2W specifying the base 2 logarithm of the width of the region to be predicted,
      • a variable log2H specifying the base 2 logarithm of the height of the region to be predicted,
      • a variable w specifying the width of the region to be predicted,
      • a variable h specifying the height of the region to be predicted.

    The output of this process is a 2D array named pred containing the intra predicted samples.

    The process uses linear interpolation to generate filtered samples from the samples in LeftCol and
    AboveRow.

    The variable bl is set equal to LeftCol[h].

    The variable tr is set equal to AboveRow[w].

    The variable scale is set equal to Round2(log2W + log2H - 4,2).

    The array pred is derived as follows:

     for ( i = 0; i < h; i++ ) {
         for ( j = 0; j < w; j++ ) {
             sTop = BLEND_WEIGHT_MAX >> Min(6, (i << 1) >> scale)
             sLeft = BLEND_WEIGHT_MAX >> Min(6, (j << 1) >> scale)
             top = AboveRow[ j ]
             left = LeftCol[ i ]
             predH = tr + Round2( (left - tr) * (w - 1 - j), log2W )
             predV = bl + Round2( (top - bl) * (h - 1 - i), log2H )
             predH2 = predH + Round2( (left - predH) * sLeft, 6 )



    AV2 Specification                                                                             Page 462 of 1169
               predV2 = predV + Round2( (top - predV) * sTop, 6 )
               if ( mode == SMOOTH_H_PRED ) {
                   pred[ i ][ j ] = predH2
               } else if ( mode == SMOOTH_V_PRED ) {
                   pred[ i ][ j ] = predV2
               } else {
                   pred[ i ][ j ] = Round2( predV2 + predH2, 1 )
               }
          }
     }


    The output of the process is the array pred.

```

<a id="s-7-13-2-14"></a>

##### § 7.13.2.14 Filter corner process

```text
§   7.13.2.14. Filter corner process

    This process uses a three tap filter to compute the value to be used for the top-left corner.

    The variable s is set equal to LeftCol[ 0 ] * 5 + AboveRow[ -1 ] * 6 + AboveRow[ 0 ] * 5.

    The output of this process is Round2(s, 4).

```

<a id="s-7-13-2-15"></a>

##### § 7.13.2.15 Intra filter type above process

```text
§   7.13.2.15. Intra filter type above process

    The input to this process is a variable plane specifying the color plane being processed.

    The output of this process is a variable that is set to 1 if the block above uses a smooth prediction mode.

    The process is specified as follows:

     get_filter_type_above( plane ) {
         aboveSmooth = 0
         if ( ( plane == 0 ) ? AvailU : AvailUChroma ) {
             if ( plane > 0 && TreeType == SHARED_PART ) {
                 r = ChromaMiRow - 1
                 c = ChromaMiCol
             } else {
                 r = MiRow - 1
                 c = MiCol
             }
             aboveSmooth = is_smooth( r, c, plane )
         }
         return aboveSmooth
     }


    where the function is_smooth indicates if a prediction mode is one of the smooth intra modes and is
    specified as:

     is_smooth( row, col, plane ) {
       if ( plane == 0 ) {
         mode = YModes[ row ][ col ]
       } else {
         return UVSmooth[ row ][ col ]
       }
       return (mode == SMOOTH_PRED || mode == SMOOTH_V_PRED || mode == SMOOTH_H_PRED)
     }


```

<a id="s-7-13-2-16"></a>

##### § 7.13.2.16 Intra filter type left process

```text
§   7.13.2.16. Intra filter type left process

    The input to this process is a variable plane specifying the color plane being processed.



    AV2 Specification                                                                               Page 463 of 1169
    The output of this process is a variable that is set to 1 if the block to the left uses a smooth prediction
    mode.

    The process is specified as follows:

     get_filter_type_left( plane ) {
         leftSmooth = 0
         if ( ( plane == 0 ) ? AvailL : AvailLChroma ) {
             if ( plane > 0 && TreeType == SHARED_PART ) {
                 r = ChromaMiRow
                 c = ChromaMiCol - 1
             } else {
                 r = MiRow
                 c = MiCol - 1
             }
             leftSmooth = is_smooth( r, c, plane )
         }
         return leftSmooth
     }


```

<a id="s-7-13-2-17"></a>

##### § 7.13.2.17 Intra edge filter strength selection process

```text
§   7.13.2.17. Intra edge filter strength selection process

    The inputs to this process are:

      • a variable w containing the width of the transform in samples,
      • a variable h containing the height of the transform in samples,
      • a variable filterType that is 0 or 1 that controls the strength of filtering,
      • a variable delta containing an angle difference in degrees.

    The output is an intra edge filter strength from 0 to 3 inclusive.

    The variable d is set equal to Abs( delta ).

    The variable blkWh (containing the sum of the dimensions) is set equal to w + h.

    The output variable strength is specified as follows:

     strength = 0
     if ( filterType == 0 ) {
         if ( blkWh <= 8 ) {
              if ( d >= 56 ) strength = 1
         } else if ( blkWh <= 12 ) {
              if ( d >= 40 ) strength = 1
         } else if ( blkWh <= 16 ) {
              if ( d >= 40 ) strength = 1
         } else if ( blkWh <= 24 ) {
              if ( d >= 8 ) strength = 1
              if ( d >= 16 ) strength = 2
              if ( d >= 32 ) strength = 3
         } else if ( blkWh <= 32 ) {
              strength = 1
              if ( d >= 4 ) strength = 2
              if ( d >= 32 ) strength = 3
         } else {
              strength = 3
         }
     } else {
         if ( blkWh <= 8 ) {
              if ( d >= 40 ) strength = 1
              if ( d >= 64 ) strength = 2



    AV2 Specification                                                                                Page 464 of 1169
          } else if ( blkWh <= 16 ) {
              if ( d >= 20 ) strength = 1
              if ( d >= 48 ) strength = 2
          } else if ( blkWh <= 24 ) {
              if ( d >= 4 ) strength = 3
          } else {
              strength = 3
          }
     }


```

<a id="s-7-13-2-18"></a>

##### § 7.13.2.18 Intra edge filter process

```text
§   7.13.2.18. Intra edge filter process

    The inputs to this process are:

      • a size sz (sz will always be less than or equal to 129),
      • a filter strength between 0 and 3 inclusive,
      • an edge direction left (when equal to 1, it specifies a vertical edge; when equal to 0, it specifies a
        horizontal edge.

    The process filters the LeftCol (if left is equal to 1) or AboveRow (if left is equal to 0) arrays.

    If strength is equal to 0, the process returns without doing anything.

    The array edge is derived by setting edge[ i ] equal to ( left ? LeftCol[ i - 1 ] : AboveRow[ i - 1 ] ) for i =
    0..sz-1.

    Otherwise (strength is not equal to 0), the following ordered steps apply for i = 1..sz-1:

     1. The variable s is set equal to 0.
     2. The following steps now apply for j = 0..INTRA_EDGE_TAPS-1:

          1. The variable k is set equal to Clip3( 0, sz - 1, i - 2 + j ).
          2. The variable s is incremented by Intra_Edge_Kernel[ strength - 1 ][ j ] * edge[ k ].
     3. If left is equal to 1, LeftCol[ i - 1 ] is set equal to ( s + 8 ) >> 4.
     4. If left is equal to 0, AboveRow[ i - 1 ] is set equal to ( s + 8 ) >> 4.

    The array Intra_Edge_Kernel is specified as follows:

     Intra_Edge_Kernel[INTRA_EDGE_KERNELS][INTRA_EDGE_TAPS] = {
       { 0, 4, 8, 4, 0 },
       { 0, 5, 6, 5, 0 },
       { 2, 4, 4, 4, 2 }
     }


```

<a id="s-7-13-3"></a>

#### § 7.13.3 Inter prediction process

```text
§   7.13.3. Inter prediction process

```

<a id="s-7-13-3-1"></a>

##### § 7.13.3.1 General

```text
§   7.13.3.1. General

    The inter prediction process is invoked for inter coded blocks and inter intra blocks.

    The inputs to this process are:

      • a variable plane specifying which plane is being predicted,



    AV2 Specification                                                                                  Page 465 of 1169
  • variables x and y specifying the location of the top left sample in the CurrFrame[ plane ] array of the
    region to be predicted,
  • variables w and h specifying the width and height of the region to be predicted,
  • variables candRow and candCol specifying the location (in units of 4x4 blocks) of the motion vector
    information to be used,
  • a variable fromBuildTip specifying if this was called from the build TIP process,
  • a variable sub8x8Inter specifying whether to disable compound prediction.

The outputs of this process are predicted samples in the current frame CurrFrame.

This process is triggered by a function call to predict_inter.

The variable PuWidth is set equal to w.

The variable PuHeight is set equal to h.

The variable tipPred (indicating if the block has specified TIP) is set equal to RefFrames[ candRow ]
[ candCol ][ 0 ] == TIP_FRAME.


  NOTE:       tipPred is equal to 0 when called from the build TIP process.


The array refFrames is prepared as follows:

  • If tipPred is equal to 1 and Tip_Weighting_Factor[ tip_global_wtd_index ] is equal to 16,
    refFrames[ 0 ] is set equal to ClosestPast and refFrames[ 1 ] is set equal to NONE.
  • Otherwise, if tipPred is equal to 1, refFrames[ 0 ] is set equal to ClosestPast and refFrames[ 1 ] is set
    equal to ClosestFuture.
  • Otherwise, if fromBuildTip is equal to 1 and CwpIdx is equal to 16, refFrames[ 0 ] is set equal to
    RefFrames[ candRow ][ candCol ][ 0 ] and refFrames[ 1] is set equal to NONE.
  • Otherwise, refFrames[ i ] is set equal to RefFrames[ candRow ][ candCol ][ i ] for i = 0..1.

The constant table Tip_Weighting_Factor is defined as:

 Tip_Weighting_Factor[ 8 ] = { 8,   12, 16, 18, 20, 4,   6,   -4 }


The variable BlockInterp (giving the interpolation filter to be used by the predict subblock process) is set
equal to InterpFilters[ candRow ][ candCol ].

The variable subX is set equal to ( plane > 0) ? SubsamplingX : 0.

The variable subY is set equal to ( plane > 0) ? SubsamplingY : 0.

The variable isCompound (equal to 1 if two inter predictions will be prepared, equal to 0 if only a single
inter prediction will be prepared) is prepared as follows:

  • If sub8x8Inter is equal to 1, isCompound is set equal to 0.
  • Otherwise, if plane is greater than 0 and tipPred is equal to 0 and fromBuildTip is equal to 0 and
    is_thin_4xn_nx4_block() is equal to 1, isCompound is set equal to 0.



AV2 Specification                                                                               Page 466 of 1169
  • Otherwise, isCompound is set equal to is_inter_ref_frame( refFrames[ 1 ] ).


  NOTE:       Inter intra prediction only requires a single prediction so has isCompound equal to 0.


The variable LumaUseOptflowRefinement (specifying if the luma plane uses optical flow refinement) is
set as follows:

 if (tipPred) {
     LumaUseOptflowRefinement = opfl_refine_type != REFINE_NONE &&
          Tip_Weighting_Factor[ tip_global_wtd_index ] == CWP_EQUAL &&
          opfl_allowed_for_refs(refFrames) && enable_tip_refinemv
     if ( enable_tip_refinemv ? (w << subX) == 256 && (h << subY) == 256 :
                                 (w << subX) >= 16 && (h << subY) >= 16 ) {
          tipSize = BLOCK_16X16
          LumaUseOptflowRefinement = 0
     } else {
          tipSize = BLOCK_8X8
     }
 } else if (isCompound && opfl_allowed_for_refs( RefFrame )) {
     LumaUseOptflowRefinement = use_optflow
 } else {
     LumaUseOptflowRefinement = 0
 }


The variable useOptflowRefinement (specifying if the current plane uses optical flow refinement) is set as
follows:

 if ( tipPred || fromBuildTip ) {
     useOptflowRefinement = (plane == 0) && LumaUseOptflowRefinement
 } else {
     useOptflowRefinement = LumaUseOptflowRefinement
 }


The variable useRefinemv (specifying if the prediction uses motion vector refinement) is specified as
follows:

 if ( tipPred ) {
     useRefinemv = NumFutureRefs > 0 && NumPastRefs > 0 &&
                   enable_refinemv && enable_tip_refinemv
 } else if ( fromBuildTip ) {
     useRefinemv = 0
 } else {
     useRefinemv = use_refinemv
 }



  NOTE: The variable useRefinemv means that the predict refinemv process will be invoked.
  However, this does not necessarily mean that the motion vector search is used. The search is only
  used if the input useSearch to the predict refinemv process is true.


If plane is equal to 0, the warp parameters are prepared as follows:

  • If motion_mode is equal to LOCALWARP, the warp estimation process in § 7.13.3.23 Warp estimation
    process is invoked with 0 as input.
  • If motion_mode is equal to LOCALWARP and isCompound is equal to 1, the warp estimation process
    in § 7.13.3.23 Warp estimation process is invoked with 1 as input.



AV2 Specification                                                                              Page 467 of 1169
      • If motion_mode is equal to EXTENDWARP, the warp estimation process in § 7.13.3.24 Extend warp
        estimation process is invoked with BlockMvs[ 0 ] as input.

    The block is predicted in parts as follows:

     if (tipPred) {
         sw = Block_Width[ tipSize ] >> subX
         sh = Block_Height[ tipSize ] >> subY
         for( i = 0; i < h; i += sh ) {
              for( j = 0; j < w; j += sw) {
                  predict_tip( plane, x + j, y + i, j, i, sw, sh, refFrames,
                               useRefinemv, useOptflowRefinement)
              }
         }
     } else if (useRefinemv) {
         tw = Min(w, 16 >> subX)
         th = Min(h, 16 >> subY)
         for( i = 0; i < h; i += th ) {
              for( j = 0; j < w; j += tw) {
                  predict_refinemv( plane, x + j, y + i, j, i, tw, th,
                                    Mvs[ candRow ][ candCol ], refFrames,
                                    useOptflowRefinement, useSearch=1, tipPred=0 )
              }
         }
     } else {
         mvs = Mvs[ candRow ][ candCol ]
         useRefArea = fromBuildTip && plane > 0 && enable_tip_refinemv &&
                       NumFutureRefs > 0 && NumPastRefs > 0 &&
                       (LumaUseOptflowRefinement || TipInterpFilter == EIGHTTAP_SHARP)
         if ( useRefArea ) {
              get_ref_area(plane,x,y,w,h,mvs,refFrames)
         }
         predict_block( plane, x, y, w, h, 0, 0, mvs, refFrames, isCompound,
                         useRefinemv=0, useOptflowRefinement, tipPred=0, fromBuildTip,
                         useRefArea )
     }


    The function call to predict_tip indicates that the predict TIP process specified in § 7.13.3.2 Predict TIP
    process is invoked.

    The function call to predict_refinemv indicates that the predict refine mv specified in § 7.13.3.3 Predict
    refine mv process is invoked.

    The function call to predict_block indicates that the predict block process specified in § 7.13.3.7 Predict
    block process is invoked.

    If use_bawp is equal to 1 and plane == 0 || use_bawp_chroma, the block adaptive weighted prediction process
    in § 7.13.3.25 Block adaptive weighted prediction process is invoked with plane, x, y, w, h, BlockMvs[ 0 ],
    and 0 as inputs.

    If plane is equal to 0 and use_intrabc is equal to 1 and morph_pred is equal to 1, the build morphological
    prediction process specified in § 7.13.3.26 Build morphological prediction process is invoked with x, y, w,
    h, Mvs[ candRow ][ candCol ][ 0 ] as inputs.

```

<a id="s-7-13-3-2"></a>

##### § 7.13.3.2 Predict TIP process

```text
§   7.13.3.2. Predict TIP process

    The inputs to this process are:

      • a variable plane specifying which plane is being predicted,




    AV2 Specification                                                                              Page 468 of 1169
      • variables x and y specifying the location of the top left sample in the CurrFrame[ plane ] array of the
        region to be predicted,
      • a variable j specifying the x offset of the subblock within the inter predicted block,
      • a variable i specifying the y offset of the subblock within the inter predicted block,
      • variables w and h specifying the width and height of the region to be predicted,
      • an array refFrames of the references to use for prediction,
      • a variable useRefinemv specifying if refined motion vectors are being used,
      • a variable useOptflowRefinement specifying if optical flow refinement has been used.

    The TIP motion vector is prepared as follows:

     subX = ( plane > 0) ? SubsamplingX : 0
     subY = ( plane > 0) ? SubsamplingY : 0
     lumaRow = y >> (2 - subY)
     lumaCol = x >> (2 - subX)
     candMvs = get_tip_cand(lumaRow , lumaCol)


    Then the block is predicted as follows:

     if (useRefinemv) {
         useSearch = enable_refinemv && is_refinemv_allowed_reference(refFrames)
         predict_refinemv( plane, x, y, j, i, w, h, candMvs, refFrames,
                            useOptflowRefinement, useSearch, tipPred = 1 )
     } else {
         if ( plane == 0 ) {
              for ( i2 = i; i2 < i + h; i2 += MI_SIZE ) {
                  for( j2 = j; j2 < j + w; j2 += MI_SIZE ) {
                      RefineMvs[ i2 >> 2 ][ j2 >> 2 ] = candMvs
                  }
              }
         }
         predict_block( plane, x, y, w, h, j, i, candMvs, refFrames,
                         isCompound = refFrames[1] != NONE, useRefinemv = 0,
                         useOptflowRefinement, tipPred = 1, fromBuildTip = 0,
                         useRefArea = 0 )
     }


    The function call to predict_refinemv indicates that the predict refine mv specified in § 7.13.3.3 Predict
    refine mv process is invoked.

    The function call to predict_block indicates that the predict block process specified in § 7.13.3.7 Predict
    block process is invoked.

```

<a id="s-7-13-3-3"></a>

##### § 7.13.3.3 Predict refine mv process

```text
§   7.13.3.3. Predict refine mv process

    The inputs to this process are:

      • a variable plane specifying which plane is being predicted,
      • variables x and y specifying the location of the top left sample in the CurrFrame[ plane ] array of the
        region to be predicted,
      • a variable j specifying the x offset of the subblock within the inter predicted block,
      • a variable i specifying the y offset of the subblock within the inter predicted block,



    AV2 Specification                                                                              Page 469 of 1169
  • variables w and h specifying the width and height of the region to be predicted,
  • an array candMvs of the starting motion vectors,
  • an array refFrames of the references to use for prediction,
  • a variable useOptflowRefinement specifying if optical flow refinement has been used,
  • a variable useSearch specifying if a search for the best motion vector is done,
  • a variable tipPred specifying if this block uses TIP.

The variable useRefArea is set as follows:

 if ( tipPred ) {
     if ( plane == 0 ) {
          useRefArea = useSearch
     } else {
          useRefArea = ( useSearch || LumaUseOptflowRefinement )
     }
 } else {
     useRefArea = 1
 }


If useRefArea is equal to 1, the get ref area process in § 7.13.3.4 Get ref area process is invoked with
plane, x, y, w, h, candMvs, refFrames as inputs.

The refined motion vectors offsetMvs are prepared as follows:

 if (plane == 0) {
     if (useSearch) {
          (dx,dy) = search_refinemv(x, y, w, h, tipPred, candMvs, refFrames)
     } else {
          dx = 0
          dy = 0
     }
     offsetMvs = offset_refinemv(candMvs, dx, dy)
     for (i2 = i; i2 < i + h; i2 += MI_SIZE) {
          for(j2 = j; j2 < j + w; j2 += MI_SIZE) {
              RefineMvs[ i2 >> 2 ][ j2 >> 2 ] = offsetMvs
          }
     }
 } else {
     offsetMvs = RefineMvs[i >> (2 - SubsamplingY)][j >> (2 - SubsamplingX)]
 }


The function call to search_refinemv indicates that the search refine mv process in § 7.13.3.6 Search
refine mv process is invoked.

The function offset_refinemv adds the offset to a motion vector as follows:

 offset_refinemv(srcMvs, dx, dy) {
     dstMvs[ 0 ][ 0 ] = srcMvs[ 0 ][ 0 ] + dy * 8
     dstMvs[ 0 ][ 1 ] = srcMvs[ 0 ][ 1 ] + dx * 8
     dstMvs[ 1 ][ 0 ] = srcMvs[ 1 ][ 0 ] - dy * 8
     dstMvs[ 1 ][ 1 ] = srcMvs[ 1 ][ 1 ] - dx * 8
     return dstMvs
 }




AV2 Specification                                                                              Page 470 of 1169
    Then the predict block process specified in § 7.13.3.7 Predict block process is invoked with plane, x, y, w,
    h, j, i, offsetMvs, refFrames, isCompound equal to 1, useRefinemv equal to 1, useOptflowRefinement,
    tipPred, fromBuildTip equal to 0, and useRefArea as inputs.

```

<a id="s-7-13-3-4"></a>

##### § 7.13.3.4 Get ref area process

```text
§   7.13.3.4. Get ref area process

    The inputs to this process are:

      • a variable plane specifying which plane is being predicted,
      • variables x and y specifying the location of the top left sample in the CurrFrame[ plane ] array of the
        region to be predicted,
      • variables w and h specifying the width and height of the region to be predicted,
      • an array candMvs of the starting motion vectors,
      • an array refFrames of the references to use for prediction.

    The get ref area single process specified in § 7.13.3.5 Get ref area single process is invoked with plane, x,
    y, w, h, candMvs, refFrames, refList equal to 0 as inputs.

    If is_inter_ref_frame(refFrames[1]) is equal to 1, the get ref area single process specified in § 7.13.3.5 Get
    ref area single process is invoked with plane, x, y, w, h, candMvs, refFrames, refList equal to 1 as inputs.

```

<a id="s-7-13-3-5"></a>

##### § 7.13.3.5 Get ref area single process

```text
§   7.13.3.5. Get ref area single process

    The inputs to this process are:

      • a variable plane specifying which plane is being predicted,
      • variables x and y specifying the location of the top left sample in the CurrFrame[ plane ] array of the
        region to be predicted,
      • variables w and h specifying the width and height of the region to be predicted,
      • an array candMvs of the starting motion vectors,
      • an array refFrames of the references to use for prediction,
      • a variable refList specifying which reference list is being predicted.

    Variables specifying the allowed reference area are prepared as follows:

     subX = ( plane > 0) ? SubsamplingX : 0
     subY = ( plane > 0) ? SubsamplingY : 0
     refIdx = ref_frame_idx[ refFrames[refList] ]
     (startX, startY, stepX, stepY) = motion_vector_scaling( plane, refIdx, x, y,
                                                             candMvs[ refList ], 0 )
     lastX = ( (RefMiCols[ refIdx ] * MI_SIZE ) >> subX) - 1
     lastY = ( (RefMiRows[ refIdx ] * MI_SIZE ) >> subY) - 1
     if ( w == 4 ) {
         RefFirstX[refList] = Clip3( 0, lastX, (startX >> 10) - 1 )
         RefLastX[refList] = Clip3( 0, lastX,
                                    ( (startX + stepX * (w - 1) ) >> 10 ) + 2 )
     } else {
         RefFirstX[refList] = Clip3( 0, lastX, (startX >> 10) - 3 )
         RefLastX[refList] = Clip3( 0, lastX,
                                    ( (startX + stepX * (w - 1) ) >> 10 ) + 4 )
     }
     if ( h == 4 ) {
         RefFirstY[refList] = Clip3( 0, lastY, (startY >> 10) - 1 )




    AV2 Specification                                                                               Page 471 of 1169
         RefLastY[refList] = Clip3( 0, lastY,
                                    ( (startY + stepY * (h - 1) ) >> 10 ) + 2 )
     } else {
         RefFirstY[refList] = Clip3( 0, lastY, (startY >> 10) - 3 )
         RefLastY[refList] = Clip3( 0, lastY,
                                    ( (startY + stepY * (h - 1) ) >> 10 ) + 4 )
     }


    The function call to motion_vector_scaling indicates that the motion vector scaling process in § 7.13.3.17
    Motion vector scaling process is invoked.

```

<a id="s-7-13-3-6"></a>

##### § 7.13.3.6 Search refine mv process

```text
§   7.13.3.6. Search refine mv process

    The inputs to this process are:

      • variables x and y specifying the location of the top left sample in the CurrFrame[ plane ] array of the
        region to be predicted,
      • variables w and h specifying the width and height of the region to be predicted,
      • a variable tipPred specifying if this block uses TIP,
      • an array candMvs of the starting motion vectors,
      • an array refFrames of the references to use for prediction.

    The process searches for an appropriate integer offset to apply to the motion vectors.

    The output of the process is the chosen offset.

    For i = 0..1, for comp = 0..1, the following ordered steps (which detect if applying the offsets to the
    motion vector would cause an overflow) apply:

     1. The variable t is set equal to candMvs[ i ][ comp ].
     2. If t - 4 * 8 is less than MV_LOW + 1 or t + 2 * 8 is greater than MV_UPP - 1, the process immediately
        terminates with outputs of 0 and 0.

    The size of the region is expanded by 2 samples in all directions as follows:

     x -= 2
     y -= 2
     w += 4
     h += 4


    The variable allowCentre (specifying if the central position corresponding to no offset is searched) is set
    equal to tipPred || !is_switchable_refinemv().

    The variables bestDy, bestDx, and bestSad are set equal to 0.

    The variable th (specifying a threshold value) is set equal to (w * h) << 1.

    If allowCentre is equal to 1, the following ordered steps apply:

     1. The sad_refinemv function specified below is invoked with x, y, w, h, 0, 0, candMvs, refFrames as
        inputs, and the output is assigned to bestSad.
     2. bestSad is set equal to bestSad - (bestSad >> 3).



    AV2 Specification                                                                              Page 472 of 1169
     3. If bestSad is less than th, the process immediately terminates with outputs of 0 and 0.

    The positions are searched as follows:

     for( idx = 0; idx < 24; idx++) {
         tryDy = Refinemv_Neighbors[ idx ][ 0 ]
         tryDx = Refinemv_Neighbors[ idx ][ 1 ]
         sad = sad_refinemv(x, y, w, h, tryDx, tryDy, candMvs, refFrames)
         if ( (idx == 0 && !allowCentre) || sad < bestSad ) {
             bestDy = tryDy
             bestDx = tryDx
             bestSad = sad
         }
     }


    The outputs of this process are bestDx and bestDy.

    The constant table Refinemv_Neighbors (containing the search locations) is specified as:

     Refinemv_Neighbors[ 24 ][ 2 ] = {
         { -2, -2 }, { -2, -1 }, { -2, 0 }, { -2, 1 }, { -2, 2 }, { -1, -2 },
         { -1, -1 }, { -1, 0 }, { -1, 1 }, { -1, 2 }, { 0, -2 }, { 0, -1 },
         { 0, 1 },   { 0, 2 },   { 1, -2 }, { 1, -1 }, { 1, 0 }, { 1, 1 },
         { 1, 2 },   { 2, -2 }, { 2, -1 }, { 2, 0 }, { 2, 1 }, { 2, 2 }
     }


    The function get_sad (which computes the sum of absolute differences between two predictions with
    optional downsampling) is specified as:

     get_sad(w, h, ds) {
         sad = 0
         for (i = 0; i < h; i += 1 + ds) {
             for (j = 0; j < w; j++) {
                 sad += Abs(Clip1(Preds[0][i][j]) - Clip1(Preds[1][i][j]))
             }
         }
         return sad
     }


    The function sad_refinemv (which computes the sum of absolute values for a specific offset) is specified
    as:

     sad_refinemv(x, y, w, h, dx, dy, candMvs, refFrames) {
         mvs = offset_refinemv(candMvs, dx, dy)
         make_inter_predictions(x, y, w, h, mvs, refFrames, 1)
         return get_sad(w, h, 1) >> (BitDepth - 8)
     }


    The function call to make_inter_predictions indicates that the make inter predictions process specified in
    § 7.13.3.13 Make inter predictions process is invoked.

```

<a id="s-7-13-3-7"></a>

##### § 7.13.3.7 Predict block process

```text
§   7.13.3.7. Predict block process

    The inputs to this process are:

      • a variable plane specifying which plane is being predicted,




    AV2 Specification                                                                             Page 473 of 1169
  • variables x and y specifying the location of the top left sample in the CurrFrame[ plane ] array of the
    region to be predicted,
  • variables w and h specifying the width and height of the region to be predicted,
  • a variable j specifying the x offset of the subblock within the inter predicted block,
  • a variable i specifying the y offset of the subblock within the inter predicted block,
  • an array mvs of the motion vectors to use for prediction,
  • an array refFrames of the references to use for prediction,
  • a variable isCompound specifying if two inter predictions are required,
  • a variable useRefinemv specifying if refined motion vectors are being used,
  • a variable useOptflowRefinement specifying if optical flow refinement has been used,
  • a variable tipPred specifying if this block uses TIP,
  • a variable fromBuildTip specifying if the prediction process is called from the build TIP process,
  • a variable useRefArea specifying if the prediction is to be clipped to sample only from within a
    reference area.

If plane is equal to 0 and useOptflowRefinement is equal to 1, the array OpflMvs is filled in with the
original value of the motion vector and MvDeltas is cleared as follows:

 for(i2=0;i2<h;i2+=4) {
     for(j2=0;j2<w;j2+=4) {
         for(list=0;list<2;list++) {
             for(comp=0;comp<2;comp++) {
                 OpflMvs[(i2 + i)>>2][(j2 + j)>>2][list][comp] =
                     mvs[list][comp] * 2
                 MvDeltas[(i2 + i)>>2][(j2 + j)>>2][list][comp] = 0
             }
         }
     }
 }


The reference area for chroma blocks is prepared (when necessary) as follows:

 if ( plane > 0 &&
      !useOptflowRefinement && LumaUseOptflowRefinement &&
      (tipPred || fromBuildTip) && !useRefArea ) {
     get_ref_area(plane,x,y,w,h,mvs,refFrames)
     useRefArea = 1
 }


The block is predicted as follows:

  • If useOptflowRefinement is equal to 1, the predict optflow block process specified in § 7.13.3.8
    Predict optflow block process is invoked with plane, x, y, w, h, j, i, mvs, refFrames, useRefinemv,
    tipPred, fromBuildTip, useRefArea as inputs.
  • Otherwise, if plane is greater than 0 and LumaUseOptflowRefinement is equal to 1 and either tipPred
    is equal to 1 or fromBuildTip is equal to 1, the predict subblock process specified in § 7.13.3.14
    Predict subblock process is invoked with plane, x, y, w, h, OpflMvs[i >> (2 - SubsamplingY)][j >> (2 -
    SubsamplingX)], prescaled equal to 1, refFrames, isCompound, useRefinemv, useOptflowRefinement
    equal to 0, tipPred, fromBuildTip, useRefArea as inputs.



AV2 Specification                                                                              Page 474 of 1169
      • Otherwise, the predict subblock process specified in § 7.13.3.14 Predict subblock process is invoked
        with plane, x, y, w, h, mvs, prescaled equal to 0, refFrames, isCompound, useRefinemv,
        useOptflowRefinement equal to 0, tipPred, fromBuildTip, useRefArea as inputs.

```

<a id="s-7-13-3-8"></a>

##### § 7.13.3.8 Predict optflow block process

```text
§   7.13.3.8. Predict optflow block process

    The inputs to this process are:

      • a variable plane specifying which plane is being predicted,
      • variables x and y specifying the location of the top left sample in the CurrFrame[ plane ] array of the
        region to be predicted,
      • variables w and h specifying the width and height of the region to be predicted,
      • a variable j specifying the x offset of the subblock within the inter predicted block,
      • a variable i specifying the y offset of the subblock within the inter predicted block,
      • an array mvs of the motion vectors to use for prediction,
      • an array refFrames of the references to use for prediction,
      • a variable useRefinemv specifying if refined motion vectors are being used,
      • a variable tipPred specifying if this block uses TIP,
      • a variable fromBuildTip specifying if the prediction process is called from the build TIP process,
      • a variable useRefArea specifying if the prediction is to be clipped to sample only from within a
        reference area.

    If plane is equal to 0, the make inter predictions process specified in § 7.13.3.13 Make inter predictions
    process is invoked with x, y, w, h, mvs, refFrames, useRefArea as input.

    If tipPred is equal to 1 or fromBuildTip is equal to 1 (in these cases plane will always be equal to 0), the
    following ordered steps apply:

     1. the variable sad is set equal to get_sad(w, h, 0) >> (BitDepth - 8).
     2. the variable sadThresh is set equal to TipFrameMode == TIP_FRAME_AS_OUTPUT ? 15 : 6.
     3. If sad is less than sadThresh, the following ordered steps apply:

          1. The predict subblock process specified in § 7.13.3.14 Predict subblock process is invoked with
             plane, x, y, w, h, mvs, prescaled equal to 0, refFrames, isCompound equal to 1, useRefinemv,
             useOptflowRefinement equal to 0, tipPred, fromBuildTip, useRefArea as inputs.
          2. This process immediately terminates.

    The variables defining the size of the subblocks are prepared as follows:

     subX = ( plane > 0) ? SubsamplingX : 0
     subY = ( plane > 0) ? SubsamplingY : 0
     use4x4 = (!tipPred && !fromBuildTip)
     lumaN = ( (h << subY) <= 8 && (w << subX) <= 8 && use4x4) ? 4 : 8
     sw = Max(4, lumaN >> subX)
     sh = Max(4, lumaN >> subY)




    AV2 Specification                                                                               Page 475 of 1169
    If plane is equal to 0, the get optflow based mv process specified in § 7.13.3.9 Get optflow based mv
    process is invoked with j, i, w, h, lumaN, mvs, and refFrames as inputs.

    The block is then predicted out of subblocks of size sw by sh as follows:

     if ( !useRefArea && tipPred && useRefinemv ) {
         get_ref_area( 0, x, y, w, h, mvs, refFrames )
         useRefArea = 1
     }
     if ( !useRefArea && fromBuildTip ) {
         get_ref_area( 0, x, y, w, h, mvs, refFrames )
         useRefArea = 1
     }
     setRefArea = !useRefArea && !fromBuildTip &&
                  ( tipPred || plane > 0 || (sh == 8 && sw == 8) )
     for ( i2 = 0; i2 < h; i2 += sh ) {
         for ( j2 = 0; j2 < w; j2 += sw ) {
             if ( setRefArea ) {
                 get_ref_area( plane, x + j2, y + i2, sw, sh, mvs, refFrames )
             }
             for( refList = 0; refList < 2; refList++ ) {
                 opflMvs[ refList ] = prepare_optflow_transl( plane, refList,
                                                              j + j2, i + i2 )
             }
             predict_subblock( plane, x + j2, y + i2, sw, sh, opflMvs,
                               prescaled=1, refFrames, isCompound=1, useRefinemv,
                               useOptflowRefinement=1, tipPred,
                               fromBuildTip, useRefArea || setRefArea )
         }
     }


    The function call to get_ref_area indicates that the get ref area process in § 7.13.3.4 Get ref area process
    is invoked.

    The function call to predict_subblock indicates that the predict subblock process specified in § 7.13.3.14
    Predict subblock process is invoked.

    The prepare_optflow_transl function (which prepares the motion vector) is specified as:

     prepare_optflow_transl(plane, refList, j, i) {
         subX = ( plane > 0) ? SubsamplingX : 0
         subY = ( plane > 0) ? SubsamplingY : 0
         r = i >> (2 - subY)
         c = j >> (2 - subX)
         return OpflMvs[ r ][ c ][ refList ]
     }


```

<a id="s-7-13-3-9"></a>

##### § 7.13.3.9 Get optflow based mv process

```text
§   7.13.3.9. Get optflow based mv process

    The inputs to this process are:

      • a variable optX specifying the x offset of the subblock within the inter predicted block,
      • a variable optY specifying the y offset of the subblock within the inter predicted block,
      • variables w and h specifying the width and height of the subblock,
      • a variable n specifying that the size of the optical flow blocks is n by n,
      • an array mvs of the motion vectors to use for prediction,
      • an array refFrames of the references to use for prediction.


    AV2 Specification                                                                               Page 476 of 1169
    The length 2 array dist is prepared as follows:

     for( i = 0; i < 2; i++ ) {
         dist[ i ] = get_relative_dist( OrderHint, OrderHints[ refFrames[ i ] ] )
     }


    If dist[ 0 ] is equal to 0 or dist[ 1 ] is equal to 0, the process terminates immediately.

    The distances are modified as follows (this reduces the size of the distances while preserving their ratio):

     if ( Abs(dist[0]) == Abs(dist[1]) ) {
         dist[0] = dist[0] < 0 ? -1 : 1
         dist[1] = dist[1] < 0 ? -1 : 1
     } else if ( Abs(dist[0]) > Abs(dist[1]) ) {
         dist[0] = dist[0] < 0 ? -2 : 2
         dist[1] = dist[1] < 0 ? -1 : 1
     } else {
         dist[0] = dist[0] < 0 ? -1 : 1
         dist[1] = dist[1] < 0 ? -2 : 2
     }


    The optflow difference process specified in § 7.13.3.10 Optflow difference process is invoked with w, h,
    and dist as inputs, and the outputs are assigned to tmp and pDiff.

    The compute gradient process specified in § 7.13.3.11 Compute gradient process is invoked with w, h, and
    tmp as inputs, and the outputs are assigned to xGrad and yGrad.

    The optical flow motion vectors are prepared as follows:

     for( i = 0; i < h; i += n ) {
         for( j = 0; j < w; j += n ) {
             compute_opfl_mv(optX,optY,i,j,n,xGrad,yGrad,pDiff,dist,mvs)
         }
     }


    The function call to compute_opfl_mv indicates that the compute optflow motion vector process specified
    in § 7.13.3.12 Compute optflow motion vector process is invoked.

```

<a id="s-7-13-3-10"></a>

##### § 7.13.3.10 Optflow difference process

```text
§   7.13.3.10. Optflow difference process

    The inputs to this process are:

      • variables w and h specifying the width and height of the subblock,
      • an array dist containing the scaled order hint distances for each reference list.

    The process clips and scales the predictions as follows:

     for(i=0;i<h;i++) {
         for(j=0;j<w;j++) {
             src0 = Clip1(Preds[0][i][j])
             src1 = Clip1(Preds[1][i][j])
             tmp[i][j] = Round2Signed(dist[0] * src0 - dist[1] * src1, BitDepth - 8)
             pDiff[i][j] = Round2Signed( src0 - src1, BitDepth - 8 )
         }
     }




    AV2 Specification                                                                             Page 477 of 1169
    The outputs of this process are the 2D arrays tmp and pDiff.

```

<a id="s-7-13-3-11"></a>

##### § 7.13.3.11 Compute gradient process

```text
§   7.13.3.11. Compute gradient process

    The inputs to this process are:

      • variables w and h specifying the width and height of the subblock,
      • an array tmp containing the scaled differences between the predicted samples from the two
        reference lists.

    The arrays xGrad and yGrad (approximating the gradient of the values in tmp) are computed as follows:

     for( i = 0; i < h; i++ ) {
         for( j = 0; j < w; j++ ) {
             jStart = (j >> OPFL_GRAD_UNIT_LOG2) << OPFL_GRAD_UNIT_LOG2
             jEnd = Min(jStart + OPFL_GRAD_UNIT,w) - 1
             iStart = (i >> OPFL_GRAD_UNIT_LOG2) << OPFL_GRAD_UNIT_LOG2
             iEnd = Min(iStart + OPFL_GRAD_UNIT,h) - 1
             jPrev = Max(j - 1, jStart)
             jPrev2 = Max(j - 2, jStart)
             jNext = Min(j + 1, jEnd)
             jNext2 = Min(j + 2, jEnd)
             temp = 42 * (tmp[i][jNext] - tmp[i][jPrev]) -
                    5 * (tmp[i][jNext2] - tmp[i][jPrev2])
             if (j + 1 > jEnd || j - 1 < jStart) {
                 temp = temp << 1
             }
             xGrad[i][j] = Round2Signed(temp,7)

               iPrev = Max(i - 1, iStart)
               iPrev2 = Max(i - 2, iStart)
               iNext = Min(i + 1, iEnd)
               iNext2 = Min(i + 2, iEnd)
               temp = 42 * (tmp[iNext][j] - tmp[iPrev][j]) -
                      5 * (tmp[iNext2][j] - tmp[iPrev2][j])
               if (i + 1 > iEnd || i - 1 < iStart) {
                   temp = temp << 1
               }
               yGrad[i][j] = Round2Signed(temp,7)
          }
     }


    The outputs of this process are xGrad and yGrad.

```

<a id="s-7-13-3-12"></a>

##### § 7.13.3.12 Compute optflow motion vector process

```text
§   7.13.3.12. Compute optflow motion vector process

    The inputs to this process are:

      • a variable optX specifying the x offset of the subblock within the inter predicted block,
      • a variable optY specifying the y offset of the subblock within the inter predicted block,
      • a variable iBase specifying the y offset of the optical flow block within the subblock,
      • a variable jBase specifying the x offset of the optical flow block within the subblock,
      • a variable n specifying that the size of the optical flow blocks is n by n,
      • arrays xGrad and yGrad containing the estimated gradient vector at each sample within the subblock,
      • an array pDiff containing the differences between predicted samples at each sample within the
        subblock,



    AV2 Specification                                                                               Page 478 of 1169
  • an array dist containing the scaled order hint distances for each reference list,
  • an array mvs of the motion vectors to use for prediction.

The process prepares motion vectors in OpflMvs for a particular optical flow block of size n by n within
the subblock. It also stores the delta from the original motion vector in MvDeltas.

Statistics about the correlations are gathered as follows:

 su2 = 0
 sv2 = 0
 suv = 0
 suw = 0
 svw = 0
 for (i = 0; i < n; i++) {
     for (j = 0; j < n; j++) {
         u = xGrad[iBase + i][jBase + j]
         v = yGrad[iBase + i][jBase + j]
         w = pDiff[iBase + i][jBase + j]
         su2 += u * u
         suv += u * v
         sv2 += v * v
         suw += u * w
         svw += v * w
     }
 }
 su2 += n * n
 sv2 += n * n


The determinant of a matrix equation is computed as follows:

 msbSu2 = 1 + GetMsb(su2)
 msbSv2 = 1 + GetMsb(sv2)
 msbSuv = 1 + GetMsb(Abs(suv))
 msbSuw = 1 + GetMsb(Abs(suw))
 msbSvw = 1 + GetMsb(Abs(svw))
 maxMultMsb = Max(msbSu2 + msbSv2, Max( Max(msbSv2 + msbSuw, msbSuv + msbSvw),
                                        Max(msbSu2 + msbSvw, msbSuv + msbSuw) ))
 redbit = Max(0, maxMultMsb - MAX_LS_BITS + 3) >> 1
 su2 = Round2Signed(su2, redbit)
 sv2 = Round2Signed(sv2, redbit)
 suv = Round2Signed(suv, redbit)
 suw = Round2Signed(suw, redbit)
 svw = Round2Signed(svw, redbit)
 det = su2 * sv2 - suv * suv


If the determinant det is less than or equal to 0, this process immediately terminates.

The matrix equation is solved and the results stored as follows:

 bits = MV_REFINE_PREC_BITS - 1
 sol[0] = sv2 * suw - suv * svw
 sol[1] = su2 * svw - suv * suw
 sol = divide_and_round_array(sol, det, bits)
 vx0 = -sol[0]
 vy0 = -sol[1]
 vx1 = vx0 * dist[1]
 vy1 = vy0 * dist[1]
 vx0 = vx0 * dist[0]
 vy0 = vy0 * dist[0]

 mvDelta[0][0] = Clip3(-OPFL_MV_DELTA_LIMIT,OPFL_MV_DELTA_LIMIT,vy0)
 mvDelta[0][1] = Clip3(-OPFL_MV_DELTA_LIMIT,OPFL_MV_DELTA_LIMIT,vx0)



AV2 Specification                                                                            Page 479 of 1169
     mvDelta[1][0] = Clip3(-OPFL_MV_DELTA_LIMIT,OPFL_MV_DELTA_LIMIT,vy1)
     mvDelta[1][1] = Clip3(-OPFL_MV_DELTA_LIMIT,OPFL_MV_DELTA_LIMIT,vx1)
     for(list=0;list<2;list++) {
         for(comp=0;comp<2;comp++) {
             MvDeltas[(optY + iBase)>>2][(optX + jBase)>>2][list][comp] =
                 mvDelta[list][comp]
             newComp = mvs[list][comp] * 2 + mvDelta[list][comp]
             OpflMvs[(optY + iBase)>>2][(optX + jBase)>>2][list][comp] =
                 Clip3( -(1<<17), (1<<17) - 1, newComp )
         }
     }


    where divide_and_round_array is defined as:

     divide_and_round_array(sol, den, shift) {
         if (den == 1) {
             invDen = 1
             denShift = 0
         } else {
             (denShift, invDen) = resolve_divisor(den)
         }
         invDenMsb = GetMsb(invDen)
         for (i = 0; i < 2; i++) {
             result[i] = 0
             if (sol[i] != 0) {
                  sgn = sol[i] > 0
                  tmp = sgn ? sol[i] : -sol[i]
                  numRedBits = Max(0, GetMsb(tmp) + invDenMsb + 4 - MAX_LS_BITS)
                  if (numRedBits > 0)
                      tmp = Round2Signed(tmp, numRedBits)
                  incBits = shift + numRedBits - denShift
                  if ( incBits <= -31) {
                      tmp = Round2Signed( tmp, -incBits - 30 )
                      mult = tmp * invDen
                      tmp = Round2Signed(mult, 30)
                  } else {
                      mult = tmp * invDen
                      if (incBits >= 0)
                           tmp = mult << incBits
                      else
                           tmp = Round2Signed(mult, -incBits)
                  }
                  result[i] = sgn ? tmp : -tmp
             }
         }
         return result
     }


```

<a id="s-7-13-3-13"></a>

##### § 7.13.3.13 Make inter predictions process

```text
§   7.13.3.13. Make inter predictions process

    The inputs to this process are:

      • variables x and y specifying the location of the top left sample in the CurrFrame[ plane ] array of the
        region to be predicted,
      • variables w and h specifying the width and height of the region to be predicted,
      • an array mvs of the motion vectors to use for prediction,
      • an array refFrames of the references to use for prediction,
      • a variable useRefArea specifying whether to only use samples within the reference area.

    The rounding variables derivation process specified in § 7.13.3.16 Rounding variables derivation process
    is invoked with the input variable isCompound set equal to 0.


    AV2 Specification                                                                             Page 480 of 1169
    The process forms two inter predictions as follows:

     for ( refList = 0; refList < 2; refList++ ) {
         refFrame = refFrames[ refList ]
         refIdx = ref_frame_idx[ refFrame ]
         (startX, startY, stepX, stepY) = motion_vector_scaling( plane = 0, refIdx,
                                                                 x, y,
                                                                 mvs[ refList ], 0 )
         block_inter_prediction( plane = 0, refList, refIdx, startX, startY,
                                 stepX, stepY, w, h, useRefArea, BILINEAR )
     }


    The function call to motion_vector_scaling indicates that the motion vector scaling process in § 7.13.3.17
    Motion vector scaling process is invoked.

    The function call to block_inter_prediction indicates that the block inter prediction process specified in
    § 7.13.3.18 Block inter prediction process is invoked.

```

<a id="s-7-13-3-14"></a>

##### § 7.13.3.14 Predict subblock process

```text
§   7.13.3.14. Predict subblock process

    The inputs to this process are:

      • a variable plane specifying which plane is being predicted,
      • variables x and y specifying the location of the top left sample in the CurrFrame[ plane ] array of the
        region to be predicted,
      • variables w and h specifying the width and height of the region to be predicted,
      • an array candMvs of the motion vectors to use for prediction,
      • a variable prescaled specifying the precision of the motion vectors in candMvs (prescaled is equal to
        0 for 1/8 th precision, prescaled is equal to 1 for 1/16 th precision),
      • an array refFrames of the references to use for prediction,
      • a variable isCompound specifying if two inter predictions are required,
      • a variable useRefinemv specifying if refined motion vectors are being used,
      • a variable useOptflowRefinement specifying if optical flow refinement has been used,
      • a variable tipPred specifying if this block uses TIP,
      • a variable fromBuildTip specifying if the prediction process is called from the build TIP process,
      • a variable useRefArea specifying if the prediction is to be clipped to sample only from within a
        reference area.

    The rounding variables derivation process specified in § 7.13.3.16 Rounding variables derivation process
    is invoked with the variable isCompound as input.

    The save subpu size process specified in § 7.13.3.15 Save subpu size process is invoked with plane, x, y, w,
    and h as inputs.

    The prediction arrays are formed as follows:

     for ( refList = 0; refList < ( isCompound ? 2 : 1 ); refList++ ) {
         refFrame = refFrames[ refList ]
         mv = candMvs[ refList ]
         if ( useRefinemv || useOptflowRefinement ||



    AV2 Specification                                                                             Page 481 of 1169
                 tipPred || fromBuildTip ||
                 force_integer_mv )
           useWarp = 0
      else if ( motion_mode == LOCALWARP ||
                 motion_mode == EXTENDWARP ||
                 motion_mode == DELTAWARP )
           useWarp = 1
      else if ( ( YMode == GLOBALMV || YMode == GLOBAL_GLOBALMV ) &&
                 GmType[ refFrame ] > IDENTITY &&
                 Min(Block_Height[MiSize], Block_Width[MiSize]) >= 8 )
           useWarp = 2
      else
           useWarp = 0

      if ( use_intrabc == 0 ) {
          refIdx = ref_frame_idx[ refFrame ]
      } else {
          refIdx = -1
          RefFrameWidth[ -1 ] = FrameWidth
          RefFrameHeight[ -1 ] = FrameHeight
      }

      (startX, startY, stepX, stepY) = motion_vector_scaling( plane, refIdx, x, y,
                                                              mv, prescaled )

      if ( useWarp != 0 ) {
          if (useWarp == 1) {
               params = LocalWarpParams[ refList ]
          } else {
               params = gm_params[ refFrame ]
          }
          (shearValid, _, _, _, _) = setup_shear( params )
          skipPred = !shearValid || w < 8 || h < 8 || is_scaled( refFrame, 0 )
          for ( y8 = 0; y8 <= ((h-1) >> 3); y8++ ) {
               for ( x8 = 0; x8 <= ((w-1) >> 3); x8++ ) {
                   block_warp( useWarp, params, plane, refList, x, y, y8, x8,
                               skipPred )
               }
          }
          if (skipPred) {
               for ( y4 = 0; y4 < (h >> 2); y4++ ) {
                   for ( x4 = 0; x4 < (w >> 2); x4++ ) {
                       ext_block_warp( params, plane, refList, x, y, y4, x4, w, h )
                   }
               }
          }
      } else {
          if (motion_mode >= LOCALWARP ) {
               for ( y8 = 0; y8 <= ((h-1) >> 3); y8++ ) {
                   for ( x8 = 0; x8 <= ((w-1) >> 3); x8++ ) {
                       block_warp( 1, LocalWarpParams[refList], plane, refList,
                                    x, y, y8, x8, 1 )
                   }
               }
          }
          if ( fromBuildTip ) {
               interp = TipInterpFilter
          } else if ( tipPred || useOptflowRefinement || useRefinemv ) {
               interp = EIGHTTAP_SHARP
          } else {
               interp = BlockInterp
          }
          block_inter_prediction( plane, refList, refIdx, startX, startY,
                                    stepX, stepY, w, h, useRefArea=useRefArea,
                                    interp )
      }
      RefStartX[ refList ] = startX >> SCALE_SUBPEL_BITS
      RefStartY[ refList ] = startY >> SCALE_SUBPEL_BITS
 }




AV2 Specification                                                                     Page 482 of 1169
The function call to motion_vector_scaling indicates that the motion vector scaling process in § 7.13.3.17
Motion vector scaling process is invoked.

The function call to block_warp indicates that the block warp process specified in § 7.13.3.19 Block warp
process is invoked.

The function call to ext_block_warp indicates that the extended block warp process specified in
§ 7.13.3.20 Extended block warp process is invoked.

The function call to block_inter_prediction indicates that the block inter prediction process specified in
§ 7.13.3.18 Block inter prediction process is invoked.

An array named Mask is prepared as follows:

  • If isCompound is equal to 1 and compound_type is equal to COMPOUND_WEDGE and plane is equal
    to 0, the wedge mask process in § 7.13.3.27 Wedge mask process is invoked with w, h as inputs.
  • Otherwise, if isCompound is equal to 1 and compound_type is equal to COMPOUND_DIFFWTD and
    plane is equal to 0, the difference weight mask process in § 7.13.3.28 Difference weight mask process
    is invoked with w, h as inputs.
  • Otherwise, no mask array is needed.

The variable cwpWeight is set as follows:

  • If tipPred is equal to 1, the variable cwpWeight is set equal to
    Tip_Weighting_Factor[ tip_global_wtd_index ].
  • Otherwise (tipPred is equal to 0), the variable cwpWeight is set equal to CwpIdx.

The variable compoundWarp is set as follows:

  • If YMode is equal to NEW_NEWMV and motion_mode is equal to LOCALWARP, compoundWarp is set
    equal to 1.
  • Otherwise (YMode is not equal to NEW_NEWMV or motion_mode is not equal to LOCALWARP),
    compoundWarp is set equal to 0.

The inter predicted samples are then derived as follows:

  • If isCompound is equal to 0, CurrFrame[ plane ][ y + i ][ x + j ] is set equal to Clip1( Preds[ 0 ][ i ]
    [ j ] ) for i = 0..h-1 and j = 0..w-1.
  • Otherwise, if compound_type is equal to COMPOUND_AVERAGE and enable_imp_msk_bld is equal to
    1 and cwpWeight is equal to CWP_EQUAL and YMode is not equal to GLOBAL_GLOBALMV and
    is_scaled( refFrames[ 0 ], 0 ) is equal to 0 and is_scaled( refFrames[ 1 ], 0 ) is equal to 0 and
    compoundWarp is equal to 0, CurrFrame[ plane ][ y + i ][ x + j ] is set equal to
    Clip1( Round2( get_mask(plane,i,j) * Preds[ 0 ][ i ][ j ] + ( 2 - get_mask(plane,i,j) ) * Preds[ 1 ][ i ][ j ],
    1 + InterPostRound ) ) for i = 0..h-1 and j = 0..w-1.
  • Otherwise, if compound_type is equal to COMPOUND_AVERAGE, CurrFrame[ plane ][ y + i ][ x + j ]
    is set equal to Clip1( Round2( cwpWeight * Preds[ 0 ][ i ][ j ] + (16 - cwpWeight) * Preds[ 1 ][ i ][ j ], 4
    + InterPostRound ) ) for i = 0..h-1 and j = 0..w-1.
  • Otherwise, the mask blend process in § 7.13.3.30 Mask blend process is invoked with plane, x, y, w, h
    as inputs.



AV2 Specification                                                                                   Page 483 of 1169
    The get_mask function is defined as:

     get_mask(plane,i,j) {
         subX = (plane > 0) ? SubsamplingX : 0
         subY = (plane > 0) ? SubsamplingY : 0
         lastX = (MiCols * MI_SIZE >> subX) - 1
         lastY = (MiRows * MI_SIZE >> subY) - 1
         refY0 = RefStartY[0] + i
         refY1 = RefStartY[1] + i
         refX0 = RefStartX[0] + j
         refX1 = RefStartX[1] + j
         ref0Onscreen = refX0 >= 0 && refX0 <= lastX && refY0 >= 0 && refY0 <= lastY
         ref1Onscreen = refX1 >= 0 && refX1 <= lastX && refY1 >= 0 && refY1 <= lastY
         if ( ref0Onscreen && !ref1Onscreen ) {
             m = 2
         } else if ( ref1Onscreen && !ref0Onscreen ) {
             m = 0
         } else {
             m = 1
         }
         return m
     }


```

<a id="s-7-13-3-15"></a>

##### § 7.13.3.15 Save subpu size process

```text
§   7.13.3.15. Save subpu size process

    The inputs to this process are:

      • a variable plane specifying which plane is being predicted,
      • variables x and y specifying the location of the top left sample in the CurrFrame[ plane ] array of the
        region to be predicted,
      • variables w and h specifying the width and height of the region to be predicted.

    If w is equal to PuWidth and h is equal to PuHeight, this process terminates immediately.

    Otherwise, the size of the sub prediction unit (for use in deblocking filtering) is saved as follows:

     subX = ( plane > 0) ? SubsamplingX : 0
     subY = ( plane > 0) ? SubsamplingY : 0
     subPuSz = find_tx_size(w, h)
     lumaRow = y >> (2 - subY)
     lumaCol = x >> (2 - subX)
     for ( r = 0; r < h >> (MI_SIZE_LOG2 - subY); r++ ) {
         for ( c = 0; c < w >> (MI_SIZE_LOG2 - subX); c++ ) {
             SubPuColBase[plane > 0][lumaRow + r][lumaCol + c] = lumaCol
             SubPuRowBase[plane > 0][lumaRow + r][lumaCol + c] = lumaRow
             SubPuSize[plane > 0][lumaRow + r][lumaCol + c] = subPuSz
         }
     }


```

<a id="s-7-13-3-16"></a>

##### § 7.13.3.16 Rounding variables derivation process

```text
§   7.13.3.16. Rounding variables derivation process

    The input to this process is a variable isCompound.

    The rounding variables InterRound0, InterRound1, and InterPostRound are derived as follows:

      • InterRound0 (representing the amount to round by after horizontal filtering) is set equal to 3.
      • InterRound1 (representing the amount to round by after vertical filtering) is set equal to
        ( isCompound ? 7 : 11).



    AV2 Specification                                                                               Page 484 of 1169
      • InterPostRound (representing the amount to round by at the end of the prediction process) is set
        equal to 2 * FILTER_BITS - ( InterRound0 + InterRound1 ).


      NOTE:       The rounding is chosen to ensure that the output of the horizontal filter always fits within 16
      bits.

```

<a id="s-7-13-3-17"></a>

##### § 7.13.3.17 Motion vector scaling process

```text
§   7.13.3.17. Motion vector scaling process

    The inputs to this process are:

      • a variable plane specifying which plane is being predicted,
      • a variable refIdx specifying which reference frame is being used,
      • variables x and y specifying the location of the top left sample in the CurrFrame[ plane ] array of the
        region to be predicted,
      • a variable mv specifying the clamped motion vector,
      • a variable prescaled specifying the precision of mv. (If prescaled is equal to 0, mv is in units of 1/8 th
        of a luma sample, i.e., with 3 fractional bits. Otherwise, mv is in units of 1/16 th of a luma sample.)

    The outputs of this process are the variables startX and startY giving the reference block location in units
    of 1/1024 th of a sample, and variables stepX and stepY giving the step size in units of 1/1024 th of a
    sample.

    This process is responsible for computing the sampling locations in the reference frame based on the
    motion vector. The sampling locations are also adjusted to compensate for any difference in the size of
    the reference frame compared to the current frame.


      NOTE: When intra block copy is being used, refIdx will be equal to -1 to signal prediction from the
      frame currently being decoded. The arrays RefFrameWidth and RefFrameHeight include values at
      index -1 giving the dimensions of the current frame.


    The variable xScale is set equal to ( ( RefFrameWidth[ refIdx ] << REF_SCALE_SHIFT ) + ( FrameWidth / 2 ) ) /
    FrameWidth.


    The variable yScale is set equal to
    ( ( RefFrameHeight[ refIdx ] << REF_SCALE_SHIFT ) + ( FrameHeight / 2 ) ) / FrameHeight.


    (xScale and yScale specify the size of the reference frame relative to the current frame in units where (1
    << 14) is equivalent to both frames having the same size.)


    The variables subX and subY are set equal to the subsampling for the current plane as follows:

      • If plane is equal to 0, subX is set equal to 0 and subY is set equal to 0.
      • Otherwise, subX is set equal to SubsamplingX and subY is set equal to SubsamplingY.

    The variable halfSample (representing half the size of a sample in units of 1/16 th of a sample) is set
    equal to ( 1 << ( SUBPEL_BITS - 1 ) ).




    AV2 Specification                                                                               Page 485 of 1169
    The variables origX and origY are set as follows:

     if ( prescaled ) {
         origX = ( (x << SUBPEL_BITS) + Round2Signed( mv[1], subX ) + halfSample )
         origY = ( (y << SUBPEL_BITS) + Round2Signed( mv[0], subY ) + halfSample )
     } else {
         origX = ( (x << SUBPEL_BITS) + ( ( 2 * mv[1] ) >> subX ) + halfSample )
         origY = ( (y << SUBPEL_BITS) + ( ( 2 * mv[0] ) >> subY ) + halfSample )
     }


    (origX and origY specify the location of the centre of the sample at the top-left corner of the reference
    block in the current frame’s coordinate system in units of 1/16 th of a sample, i.e., with SUBPEL_BITS=4
    fractional bits.)

    The variable baseX is set equal to (origX * xScale - ( halfSample << REF_SCALE_SHIFT ) ).

    The variable baseY is set equal to (origY * yScale - ( halfSample << REF_SCALE_SHIFT ) ).

    (baseX and baseY specify the location of the top-left corner of the block in the reference frame in the
    reference frame’s coordinate system with 18 fractional bits.)

    The variable off (containing a rounding offset for the filter tap selection) is set equal to ( ( 1 <<
    (SCALE_SUBPEL_BITS - SUBPEL_BITS) ) / 2 ).


    The output variable startX is set equal to (Round2Signed( baseX, REF_SCALE_SHIFT + SUBPEL_BITS -
    SCALE_SUBPEL_BITS) + off).

    The output variable startY is set equal to (Round2Signed( baseY, REF_SCALE_SHIFT + SUBPEL_BITS -
    SCALE_SUBPEL_BITS) + off).

    (startX and startY specify the location of the top-left corner of the block in the reference frame in the
    reference frame’s coordinate system with SCALE_SUBPEL_BITS=10 fractional bits.)

    The output variable stepX is set equal to Round2Signed( xScale, REF_SCALE_SHIFT -
    SCALE_SUBPEL_BITS).

    The output variable stepY is set equal to Round2Signed( yScale, REF_SCALE_SHIFT -
    SCALE_SUBPEL_BITS).

    (stepX and stepY are the size of one current frame sample in the reference frame’s coordinate system
    with 10 fractional bits.)

```

<a id="s-7-13-3-18"></a>

##### § 7.13.3.18 Block inter prediction process

```text
§   7.13.3.18. Block inter prediction process

    The inputs to this process are:

      • a variable plane,
      • a variable refList specifying which reference list is being predicted,
      • a variable refIdx specifying which reference frame is being used (or -1 for intra block copy),
      • variables x and y giving the block location in units of 1/1024 th of a sample,
      • variables xStep and yStep giving the step size in units of 1/1024 th of a sample,
      • variables w and h giving the width and height of the block in units of samples,


    AV2 Specification                                                                                 Page 486 of 1169
  • a variable useRefArea specifying if the prediction is to be clipped to sample only from within a
    reference area,
  • a variable interp specifying the interpolation filter to use.

The output from this process are updated values in the Preds[ refList ] array.

The variable ref specifying the reference frame contents is set as follows:

  • If refIdx is equal to -1, ref is set equal to CurrFrame.
  • Otherwise (refIdx is greater than or equal to 0), ref is set equal to FrameStore[ refIdx ].

The variables subX and subY are set equal to the subsampling for the current plane as follows:

  • If plane is equal to 0, subX is set equal to 0 and subY is set equal to 0.
  • Otherwise, subX is set equal to SubsamplingX and subY is set equal to SubsamplingY.

The variables firstX, firstY, lastX, lastY (giving the clipping region) are set as follows:

 if ( useRefArea ) {
     firstX = RefFirstX[refList]
     firstY = RefFirstY[refList]
     lastX = RefLastX[refList]
     lastY = RefLastY[refList]
 } else if ( use_intrabc ) {
     lastX = (MiCols * MI_SIZE >> subX) - 1
     lastY = (MiRows * MI_SIZE >> subY) - 1
     firstX = 0
     firstY = 0
 } else {
     lastX = ( (RefMiCols[ refIdx ] * MI_SIZE) >> subX) - 1
     lastY = ( (RefMiRows[ refIdx ] * MI_SIZE) >> subY) - 1
     firstX = 0
     firstY = 0
 }


The variable intermediateHeight specifying the height required for the intermediate array is set equal to
(((h - 1) * yStep + (1 << SCALE_SUBPEL_BITS) - 1) >> SCALE_SUBPEL_BITS) + 8.


The sub-sample interpolation is effected via two one-dimensional convolutions. First a horizontal filter is
used to build up a temporary array, and then this array is vertically filtered to obtain the final prediction.
The fractional parts of the motion vectors determine the filtering process. If the fractional part is zero,
then the filtering is equivalent to a straight sample copy.

The filtering is applied as follows:

  • The array intermediate is specified as follows:

      interpFilter = interp
      if ( w <= 4 ) {
          if ( interpFilter == EIGHTTAP || interpFilter == EIGHTTAP_SHARP ) {
              interpFilter = 4
          } else if ( interpFilter == EIGHTTAP_SMOOTH ) {
              interpFilter = 5
          }
      }
      for ( r = 0; r < intermediateHeight; r++ ) {
          for ( c = 0; c < w; c++ ) {




AV2 Specification                                                                                 Page 487 of 1169
                s = 0
                p = x + xStep * c
                for ( t = 0; t < 8; t++ )
                    s += Subpel_Filters[ interpFilter ][ (p >> 6) & SUBPEL_MASK ][ t ] *
                      ref[ plane ] [ Clip3( firstY, lastY, (y >> 10) + r - 3 ) ]
                                   [ Clip3( firstX, lastX, (p >> 10) + t - 3 ) ]
                intermediate[ r ][ c ] = Round2(s, InterRound0)
           }
      }


  • The array Preds is updated as follows:

      interpFilter = interp
      if ( h <= 4 ) {
          if ( interpFilter == EIGHTTAP || interpFilter == EIGHTTAP_SHARP ) {
              interpFilter = 4
          } else if ( interpFilter == EIGHTTAP_SMOOTH ) {
              interpFilter = 5
          }
      }
      for ( r = 0; r < h; r++ ) {
          for ( c = 0; c < w; c++ ) {
              s = 0
              p = (y & 1023) + yStep * r
              for ( t = 0; t < 8; t++ )
                  s += Subpel_Filters[ interpFilter ][ (p >> 6) & SUBPEL_MASK ][ t ] *
                    intermediate[ (p >> 10) + t ][ c ]
              Preds[ refList ][ r ][ c ] = Round2(s, InterRound1)
          }
      }


    where the constant array Subpel_Filters is specified as:

      Subpel_Filters[ 6 ][ 16 ][ 8 ] = {
        {
           { 0, 0, 0, 128, 0, 0, 0, 0 },
           { 0, 2, -6, 126, 8, -2, 0, 0 },
           { 0, 2, -10, 122, 18, -4, 0, 0 },
           { 0, 2, -12, 116, 28, -8, 2, 0 },
           { 0, 2, -14, 110, 38, -10, 2, 0 },
           { 0, 2, -14, 102, 48, -12, 2, 0 },
           { 0, 2, -16, 94, 58, -12, 2, 0 },
           { 0, 2, -14, 84, 66, -12, 2, 0 },
           { 0, 2, -14, 76, 76, -14, 2, 0 },
           { 0, 2, -12, 66, 84, -14, 2, 0 },
           { 0, 2, -12, 58, 94, -16, 2, 0 },
           { 0, 2, -12, 48, 102, -14, 2, 0 },
           { 0, 2, -10, 38, 110, -14, 2, 0 },
           { 0, 2, -8, 28, 116, -12, 2, 0 },
           { 0, 0, -4, 18, 122, -10, 2, 0 },
           { 0, 0, -2, 8, 126, -6, 2, 0 }
        },
        {
           { 0, 0, 0, 128, 0, 0, 0, 0 },
           { 0, 2, 28, 62, 34, 2, 0, 0 },
           { 0, 0, 26, 62, 36, 4, 0, 0 },
           { 0, 0, 22, 62, 40, 4, 0, 0 },
           { 0, 0, 20, 60, 42, 6, 0, 0 },
           { 0, 0, 18, 58, 44, 8, 0, 0 },
           { 0, 0, 16, 56, 46, 10, 0, 0 },
           { 0, -2, 16, 54, 48, 12, 0, 0 },
           { 0, -2, 14, 52, 52, 14, -2, 0 },
           { 0, 0, 12, 48, 54, 16, -2, 0 },
           { 0, 0, 10, 46, 56, 16, 0, 0 },
           { 0, 0, 8, 44, 58, 18, 0, 0 },
           { 0, 0, 6, 42, 60, 20, 0, 0 },
           { 0, 0, 4, 40, 62, 22, 0, 0 },



AV2 Specification                                                                          Page 488 of 1169
              { 0, 0, 4, 36, 62, 26, 0, 0 },
              { 0, 0, 2, 34, 62, 28, 2, 0 }
         },
         {
              { 0, 0, 0, 128, 0, 0, 0, 0 },
              { -2, 2, -6, 126, 8, -2, 2, 0 },
              { -2, 6, -12, 124, 16, -6, 4, -2 },
              { -2, 8, -18, 120, 26, -10, 6, -2 },
              { -4, 10, -22, 116, 38, -14, 6, -2 },
              { -4, 10, -22, 108, 48, -18, 8, -2 },
              { -4, 10, -24, 100, 60, -20, 8, -2 },
              { -4, 10, -24, 90, 70, -22, 10, -2 },
              { -4, 12, -24, 80, 80, -24, 12, -4 },
              { -2, 10, -22, 70, 90, -24, 10, -4 },
              { -2, 8, -20, 60, 100, -24, 10, -4 },
              { -2, 8, -18, 48, 108, -22, 10, -4 },
              { -2, 6, -14, 38, 116, -22, 10, -4 },
              { -2, 6, -10, 26, 120, -18, 8, -2 },
              { -2, 4, -6, 16, 124, -12, 6, -2 },
              { 0, 2, -2, 8, 126, -6, 2, -2 }
         },
         {
              { 0, 0, 0, 128, 0, 0, 0, 0 },
              { 0, 0, 0, 120, 8, 0, 0, 0 },
              { 0, 0, 0, 112, 16, 0, 0, 0 },
              { 0, 0, 0, 104, 24, 0, 0, 0 },
              { 0, 0, 0, 96, 32, 0, 0, 0 },
              { 0, 0, 0, 88, 40, 0, 0, 0 },
              { 0, 0, 0, 80, 48, 0, 0, 0 },
              { 0, 0, 0, 72, 56, 0, 0, 0 },
              { 0, 0, 0, 64, 64, 0, 0, 0 },
              { 0, 0, 0, 56, 72, 0, 0, 0 },
              { 0, 0, 0, 48, 80, 0, 0, 0 },
              { 0, 0, 0, 40, 88, 0, 0, 0 },
              { 0, 0, 0, 32, 96, 0, 0, 0 },
              { 0, 0, 0, 24, 104, 0, 0, 0 },
              { 0, 0, 0, 16, 112, 0, 0, 0 },
              { 0, 0, 0, 8, 120, 0, 0, 0 }
         },
         {
              { 0, 0, 0, 128, 0, 0, 0, 0 },
              { 0, 0, -4, 126, 8, -2, 0, 0 },
              { 0, 0, -8, 122, 18, -4, 0, 0 },
              { 0, 0, -10, 116, 28, -6, 0, 0 },
              { 0, 0, -12, 110, 38, -8, 0, 0 },
              { 0, 0, -12, 102, 48, -10, 0, 0 },
              { 0, 0, -14, 94, 58, -10, 0, 0 },
              { 0, 0, -12, 84, 66, -10, 0, 0 },
              { 0, 0, -12, 76, 76, -12, 0, 0 },
              { 0, 0, -10, 66, 84, -12, 0, 0 },
              { 0, 0, -10, 58, 94, -14, 0, 0 },
              { 0, 0, -10, 48, 102, -12, 0, 0 },
              { 0, 0, -8, 38, 110, -12, 0, 0 },
              { 0, 0, -6, 28, 116, -10, 0, 0 },
              { 0, 0, -4, 18, 122, -8, 0, 0 },
              { 0, 0, -2, 8, 126, -4, 0, 0 }
         },
         {
              { 0, 0, 0, 128, 0, 0, 0, 0 },
              { 0, 0, 30, 62, 34, 2, 0, 0 },
              { 0, 0, 26, 62, 36, 4, 0, 0 },
              { 0, 0, 22, 62, 40, 4, 0, 0 },
              { 0, 0, 20, 60, 42, 6, 0, 0 },
              { 0, 0, 18, 58, 44, 8, 0, 0 },
              { 0, 0, 16, 56, 46, 10, 0, 0 },
              { 0, 0, 14, 54, 48, 12, 0, 0 },
              { 0, 0, 12, 52, 52, 12, 0, 0 },
              { 0, 0, 12, 48, 54, 14, 0, 0 },
              { 0, 0, 10, 46, 56, 16, 0, 0 },
              { 0, 0, 8, 44, 58, 18, 0, 0 },
              { 0, 0, 6, 42, 60, 20, 0, 0 },



AV2 Specification                                     Page 489 of 1169
                  { 0, 0, 4, 40, 62, 22, 0, 0 },
                  { 0, 0, 4, 36, 62, 26, 0, 0 },
                  { 0, 0, 2, 34, 62, 30, 0, 0 }
              }
          }



      NOTE: All the values in Subpel_Filters are even. The last two filter types are used for small blocks
      and only have four filter taps. The filter at index 4 has a four tap version of the EIGHTTAP filter. The
      filter at index 5 has a four tap version of the EIGHTTAP_SMOOTH filter.

```

<a id="s-7-13-3-19"></a>

##### § 7.13.3.19 Block warp process

```text
§   7.13.3.19. Block warp process

    The inputs to this process are:

      • a variable useWarp (equal to 1 for local warp, or 2 for global warp),
      • an array warpParams specifying the warp parameters,
      • a variable plane,
      • a variable refList specifying that the RefFrame[ refList ] is to be used by the process for prediction,
      • variables x and y specifying the location of the top left sample in the CurrFrame[ plane ] array of the
        region to be predicted,
      • variables i8 and j8 specifying the offset (in units of 8 samples) relative to the top left sample,
      • a variable skipPred specifying if the prediction part is to be skipped.

    The process updates a section of the SubMvs array with warped motion vectors.

    Also, if skipPred is equal to 0, this process updates the array Preds[ refList ] containing warped inter
    predicted samples.

    The process only updates a section of the Preds array. The size of the updated section is 8x8 samples,
    clipped to the size of the block. Variables i8 and j8 give the location of the section to update.

    The variable refIdx specifying which reference frame is being used is set equal to
    ref_frame_idx[ RefFrame[ refList ] ].

    The variable ref specifying the reference frame contents is set equal to FrameStore[ refIdx ].

    The variables subX and subY are set equal to the subsampling for the current plane as follows:

      • If plane is equal to 0, subX is set equal to 0 and subY is set equal to 0.
      • Otherwise, subX is set equal to SubsamplingX and subY is set equal to SubsamplingY.

    The variable firstX is set equal to 0.

    The variable firstY is set equal to 0.

    The variable lastX is set equal to ( (RefMiCols[ refIdx ] * MI_SIZE) >> subX) - 1.

    The variable lastY is set equal to ( (RefMiRows[ refIdx ] * MI_SIZE) >> subY) - 1.

    (firstX and firstY specify the coordinates of the top left sample of the bounding box.)



    AV2 Specification                                                                                Page 490 of 1169
(lastX and lastY specify the coordinates of the bottom right sample of the bounding box.)

The variable srcX is set equal to (x + j8 * 8 + 4) << subX.

The variable srcY is set equal to (y + i8 * 8 + 4) << subY.

(srcX and srcY specify a location in the luma plane that will be projected using the warp parameters.)

The variable dstX is set equal to warpParams[2] * srcX + warpParams[3] * srcY + warpParams[0].

The variable dstY is set equal to warpParams[4] * srcX + warpParams[5] * srcY + warpParams[1].

(dstX and dstY specify the destination location in the luma plane using WARPEDMODEL_PREC_BITS bits
of precision).

If plane is equal to 0 and useWarp is equal to 1, the warped motion vectors are saved in the SubMvs array
as follows:

 mv[0] = Round2Signed( dstY - (srcY << WARPEDMODEL_PREC_BITS),
                       WARPEDMODEL_PREC_BITS - 3)
 mv[1] = Round2Signed( dstX - (srcX << WARPEDMODEL_PREC_BITS),
                       WARPEDMODEL_PREC_BITS - 3)
 mv[0] = Clip3(MV_LOW + 1, MV_UPP - 1, mv[0])
 mv[1] = Clip3(MV_LOW + 1, MV_UPP - 1, mv[1])
 row = y >> MI_SIZE_LOG2
 col = x >> MI_SIZE_LOG2
 for( i = 0; i < 2; i++ ) {
     for( j = 0; j < 2; j++ ) {
         SubMvs[row + i8 * 2 + i][col + j8 * 2 + j][ refList ] = mv
     }
 }


If skipPred is equal to 1, the process immediately terminates.

The setup shear process specified in § 7.13.3.21 Setup shear process is invoked with warpParams as
input, and the outputs are assigned to warpValid, alpha, beta, gamma, and delta. (warpValid will always
be equal to 1 at this point.)

The sub-sample interpolation is effected via two one-dimensional convolutions. First a horizontal filter is
used to build up an intermediate array, and then this array is vertically filtered to obtain the final
prediction.

The filtering is applied as follows:

  • The array intermediate is specified as follows:

      x4 = dstX >> subX
      y4 = dstY >> subY
      ix4 = x4 >> WARPEDMODEL_PREC_BITS
      sx4 = x4 & ((1 << WARPEDMODEL_PREC_BITS) - 1)
      iy4 = y4 >> WARPEDMODEL_PREC_BITS
      sy4 = y4 & ((1 << WARPEDMODEL_PREC_BITS) - 1)

      for ( i1 = -7; i1 < 8; i1++ ) {
          for ( i2 = -4; i2 < 4; i2++ ) {
              sx = sx4 + alpha * i2 + beta * i1
              offs = Round2(sx, WARPEDDIFF_PREC_BITS) + 3 * WARPEDPIXEL_PREC_SHIFTS
              s = 0
              for ( i3 = 0; i3 < 8; i3++ ) {



AV2 Specification                                                                             Page 491 of 1169
                        s += Warped_Filters[ offs ][ i3 ] *
                          ref[ plane ][ Clip3( firstY, lastY, iy4 + i1 ) ]
                                      [ Clip3( firstX, lastX, ix4 + i2 - 3 + i3 ) ]
                    }
                    intermediate[(i1 + 7)][(i2 + 4)] = Round2(s, InterRound0)
               }
          }


      • The array Preds is specified as follows:

          for ( i1 = -4; i1 < 4; i1++ ) {
              for ( i2 = -4; i2 < 4; i2++ ) {
                  sy = sy4 + gamma * i2 + delta * i1
                  offs = Round2(sy, WARPEDDIFF_PREC_BITS) + 3 * WARPEDPIXEL_PREC_SHIFTS
                  s = 0
                  for ( i3 = 0; i3 < 8; i3++ ) {
                      s += Warped_Filters[offs][i3] *
                           intermediate[(i1 + i3 + 4)][(i2 + 4)]
                  }
                  Preds[ refList ][ i8 * 8 + i1 + 4 ][ j8 * 8 + i2 + 4 ] = Round2(s, InterRound1)
              }
          }


```

<a id="s-7-13-3-20"></a>

##### § 7.13.3.20 Extended block warp process

```text
§   7.13.3.20. Extended block warp process

    The inputs to this process are:

      • an array warpParams specifying the warp parameters,
      • a variable plane,
      • a variable refList specifying that RefFrame[ refList ] is to be used by the process for prediction,
      • variables x and y specifying the location of the top left sample in the CurrFrame[ plane ] array of the
        region to be predicted,
      • variables i4 and j4 specifying the offset (in units of 4 samples) relative to the top left sample,
      • variables w and h giving the width and height of the block in units of samples.

    This process updates the Preds array containing extended warp inter predicted samples.

    The process only updates a section of the Preds array. The size of the updated section is 4x4 samples.
    Variables i4 and j4 give the location of the section to update.

    The variable refIdx specifying which reference frame is being used is set equal to
    ref_frame_idx[ RefFrame[ refList ] ].

    The variables subX and subY are set equal to the subsampling for the current plane as follows:

      • If plane is equal to 0, subX is set equal to 0 and subY is set equal to 0.
      • Otherwise, subX is set equal to SubsamplingX and subY is set equal to SubsamplingY.

    The variable firstX is set equal to 0.

    The variable firstY is set equal to 0.

    The variable lastX is set equal to ( (RefMiCols[ refIdx ] * MI_SIZE) >> subX) - 1.

    The variable lastY is set equal to ( (RefMiRows[ refIdx ] * MI_SIZE) >> subY) - 1.


    AV2 Specification                                                                                Page 492 of 1169
The variable scaled is set equal to is_scaled( RefFrame[ refList ], 0 ).

The bounding box is modified as follows:

 i8 = i4 >> 1
 j8 = j4 >> 1
 bboxW = Min(w, 8)
 bboxH = Min(h, 8)
 mv = get_sub_block_warp_mv( warpParams, plane, x + j8 * 8, y + i8 * 8,
                             bboxW, bboxH, 0 )
 mv[ 0 ] = clamp_mv_row( mv[ 0 ] )
 mv[ 1 ] = clamp_mv_col( mv[ 1 ] )
 (startX, startY, stepX, stepY) = motion_vector_scaling( plane, refIdx,
                                                         x + j8 * 8,
                                                         y + i8 * 8, mv, 0 )

 firstX = Clip3( 0, lastX, (startX >> 10) - 3)
 firstY = Clip3( 0, lastY, (startY >> 10) - 3)
 lastX = Clip3( 0, lastX, ((startX + stepX * (bboxW - 1)) >> 10) + 4)
 lastY = Clip3( 0, lastY, ((startY + stepY * (bboxH - 1)) >> 10) + 4)


(firstX and firstY specify the coordinates of the top left sample of the bounding box.)

(lastX and lastY specify the coordinates of the bottom right sample of the bounding box.)

The variable srcX is set equal to (x + j4 * 4 + 2) << subX.

The variable srcY is set equal to (y + i4 * 4 + 2) << subY.

(srcX and srcY specify a location in the luma plane that will be projected using the warp parameters.)

The variable dstX is set equal to warpParams[2] * srcX + warpParams[3] * srcY + warpParams[0].

The variable dstY is set equal to warpParams[4] * srcX + warpParams[5] * srcY + warpParams[1].

(dstX and dstY specify the destination location in the luma plane using WARPEDMODEL_PREC_BITS bits
of precision).

The sub-sample interpolation is effected via two one-dimensional convolutions. First a horizontal filter is
used to build up an intermediate array, and then this array is vertically filtered to obtain the final
prediction as follows:

 x4 = dstX >> subX
 y4 = dstY >> subY
 if ( scaled ) {
     xScale = ( ( RefFrameWidth[ refIdx ] << REF_SCALE_SHIFT ) +
              ( FrameWidth / 2 ) ) / FrameWidth
     yScale = ( ( RefFrameHeight[ refIdx ] << REF_SCALE_SHIFT ) +
              ( FrameHeight / 2 ) ) / FrameHeight
     x4 -= 2 << WARPEDMODEL_PREC_BITS
     y4 -= 2 << WARPEDMODEL_PREC_BITS
     x4 = Round2Signed( x4 * xScale, REF_SCALE_SHIFT )
     y4 = Round2Signed( y4 * yScale, REF_SCALE_SHIFT )
     stepX = Round2Signed( xScale, REF_SCALE_SHIFT - SCALE_SUBPEL_BITS) <<
                 (WARPEDMODEL_PREC_BITS - SCALE_SUBPEL_BITS)
     stepY = Round2Signed( yScale, REF_SCALE_SHIFT - SCALE_SUBPEL_BITS) <<
                 (WARPEDMODEL_PREC_BITS - SCALE_SUBPEL_BITS)

      iy4 = y4 >> WARPEDMODEL_PREC_BITS
      sy4 = y4 & ((1 << WARPEDMODEL_PREC_BITS) - 1)




AV2 Specification                                                                             Page 493 of 1169
      intermediateHeight = ( (y4 + stepY * 3 ) >> WARPEDMODEL_PREC_BITS ) - iy4 +
                           EXT_WARP_TAPS

      for (k = 0; k < intermediateHeight; k++) {
          for (l = 0; l < 4; l++) {
              ix4 = (x4 + stepX * l) >> WARPEDMODEL_PREC_BITS
              sx4 = (x4 + stepX * l) & ((1 << WARPEDMODEL_PREC_BITS) - 1)
              offsX = Round2(sx4, EXT_WARP_ROUND_BITS)
              intX = ix4
              intY = iy4 + k - 2
              s = 0
              for (m = 0; m < EXT_WARP_TAPS; m++) {
                  s += Ext_Warped_Filters[ offsX ][ m ] *
                       FrameStore[ refIdx ][ plane ]
                                 [ Clip3( firstY, lastY, intY ) ]
                                 [ Clip3( firstX, lastX, intX - 2 + m ) ]
              }
              intermediate[ k ][ l ] = Round2( s, InterRound0 )
          }
      }


     for (l = 0; l < 4; l++) {
          for (k = 0; k < 4; k++) {
              iy4off = ( (y4 + stepY * k ) >> WARPEDMODEL_PREC_BITS ) - iy4
              sy4 = (y4 + stepY * k ) & ((1 << WARPEDMODEL_PREC_BITS) - 1)
              offsY = Round2(sy4, EXT_WARP_ROUND_BITS)
              s = 0
              for (m = 0; m < EXT_WARP_TAPS; m++) {
                  s += Ext_Warped_Filters[ offsY ][ m ] *
                       intermediate[ iy4off + m ][ l ]
              }
              Preds[ refList ][ i4 * 4 + k ][ j4 * 4 + l ] =
                  Round2( s, InterRound1 )
          }
     }
 } else {
     ix4 = x4 >> WARPEDMODEL_PREC_BITS
     sx4 = x4 & ((1 << WARPEDMODEL_PREC_BITS) - 1)
     iy4 = y4 >> WARPEDMODEL_PREC_BITS
     sy4 = y4 & ((1 << WARPEDMODEL_PREC_BITS) - 1)
     offsX = Round2(sx4, EXT_WARP_ROUND_BITS)

      for (k = -4; k < 5; k++) {
          for (l = -2; l < 2; l++) {
              s = 0
              for (m = 0; m < EXT_WARP_TAPS; m++) {
                  s += Ext_Warped_Filters[ offsX ][ m ] *
                      FrameStore[ refIdx ][ plane ]
                              [ Clip3( firstY, lastY, iy4 + k ) ]
                              [ Clip3( firstX, lastX, ix4 + l - 2 + m ) ]
              }
              intermediate[(k + 4)][(l + 2)] = Round2( s, InterRound0 )
          }
      }

      offsY = Round2(sy4, EXT_WARP_ROUND_BITS)
      for (k = -2; k < 2; k++) {
          for (l = -2; l < 2; l++) {
              s = 0
              for (m = 0; m < EXT_WARP_TAPS; m++) {
                  s += Ext_Warped_Filters[offsY][m] *
                      intermediate[(k + m + 2)][(l + 2)]
              }
              Preds[ refList ][ i4 * 4 + k + 2 ][ j4 * 4 + l + 2 ] =
                  Round2( s, InterRound1 )
          }
      }
 }




AV2 Specification                                                                   Page 494 of 1169
      NOTE: The difference between this and the block warp process is that extended warp predicts 4x4
      blocks with fixed phase, while the block warp predicts 8x8 blocks with variable phase. This means
      that extended warp is equivalent to a translation, while block warp approximates an affine
      transformation.

```

<a id="s-7-13-3-21"></a>

##### § 7.13.3.21 Setup shear process

```text
§   7.13.3.21. Setup shear process

    The input to this process is an array warpParams representing an affine transformation.

    The outputs of this process are the variable warpValid and variables alpha, beta, gamma, delta
    representing two shearing operations that combine to make the full affine transformation.

    The variable maxValue is set equal to 32767 - (1 << (WARP_PARAM_REDUCE_BITS - 1)).

    The variable alpha0 is set equal to Clip3( -32768, maxValue, warpParams[ 2 ] - (1 << WARPEDMODEL_PREC_BITS) ).

    The variable beta0 is set equal to Clip3( -32768, maxValue, warpParams[ 3 ] ).

    The resolve divisor process specified in § 7.13.3.22 Resolve divisor process is invoked with
    warpParams[ 2 ] as input, and the outputs are assigned to divShift and divFactor.

    The variable v is set equal to ( warpParams[ 4 ] << WARPEDMODEL_PREC_BITS ).

    The variable gamma0 is set equal to Clip3( -32768, maxValue, Round2Signed( v * divFactor, divShift ) ).

    The variable w is set equal to ( warpParams[ 3 ] * warpParams[ 4 ] ).

    The variable delta0 is set equal to Clip3( -32768, maxValue, warpParams[ 5 ] - Round2Signed( w * divFactor,
    divShift ) - (1 << WARPEDMODEL_PREC_BITS) ).


    The output variables alpha, beta, gamma, delta are set as follows:

     alpha = Round2Signed( alpha0, WARP_PARAM_REDUCE_BITS ) << WARP_PARAM_REDUCE_BITS
     beta = Round2Signed( beta0, WARP_PARAM_REDUCE_BITS ) << WARP_PARAM_REDUCE_BITS
     gamma = Round2Signed( gamma0, WARP_PARAM_REDUCE_BITS ) << WARP_PARAM_REDUCE_BITS
     delta = Round2Signed( delta0, WARP_PARAM_REDUCE_BITS ) << WARP_PARAM_REDUCE_BITS


    The output warpValid is set as follows:

      • If 4 * Abs( alpha ) + 7 * Abs( beta ) is greater than or equal to (3 << WARPEDMODEL_PREC_BITS), warpValid
        is set equal to 0.
      • If 4 * Abs( gamma ) + 4 * Abs( delta ) is greater than or equal to (3 << WARPEDMODEL_PREC_BITS),
        warpValid is set equal to 0.
      • Otherwise, warpValid is set equal to 1.

```

<a id="s-7-13-3-22"></a>

##### § 7.13.3.22 Resolve divisor process

```text
§   7.13.3.22. Resolve divisor process

    The input to this process is a variable d.

    The outputs of this process are variables divShift and divFactor that can be used to perform an
    approximate division by d via multiplying by divFactor and shifting right by divShift.




    AV2 Specification                                                                                Page 495 of 1169
    The variable n (representing the location of the most significant bit in Abs(d) ) is set equal to
    FloorLog2( Abs(d) ).

    The variable e is set equal to Abs( d ) - ( 1 << n ).

    The variable f is set as follows:

      • If n is greater than DIV_LUT_BITS, f is set equal to Round2( e, n - DIV_LUT_BITS ).
      • Otherwise, f is set equal to e << ( DIV_LUT_BITS - n ).

    The output variable divShift is set equal to ( n + DIV_LUT_PREC_BITS ).

    The output variable divFactor is set as follows:

      • If d is less than 0, divFactor is set equal to -Div_Lut[ f ].
      • Otherwise, divFactor is set equal to Div_Lut[ f ].

    The lookup table Div_Lut is specified as:

     Div_Lut[ DIV_LUT_NUM ] = {
         512, 508, 504, 500, 496, 493, 489, 485, 482, 478, 475, 471, 468, 465, 462,
         458, 455, 452, 449, 446, 443, 440, 437, 434, 431, 428, 426, 423, 420, 417,
         415, 412, 410, 407, 405, 402, 400, 397, 395, 392, 390, 388, 386, 383, 381,
         379, 377, 374, 372, 370, 368, 366, 364, 362, 360, 358, 356, 354, 352, 350,
         349, 347, 345, 343, 341, 340, 338, 336, 334, 333, 331, 329, 328, 326, 324,
         323, 321, 320, 318, 317, 315, 314, 312, 311, 309, 308, 306, 305, 303, 302,
         301, 299, 298, 297, 295, 294, 293, 291, 290, 289, 287, 286, 285, 284, 282,
         281, 280, 279, 278, 277, 275, 274, 273, 272, 271, 270, 269, 267, 266, 265,
         264, 263, 262, 261, 260, 259, 258, 257, 256
     }


    The function call to resolve_divisor() indicates that the process defined in this sub-section is invoked.

```

<a id="s-7-13-3-23"></a>

##### § 7.13.3.23 Warp estimation process

```text
§   7.13.3.23. Warp estimation process

    The input to this process is a variable ref specifying which set of candidate motion vectors to prepare.

    This process produces the array LocalWarpParams based on NumSamples candidates in CandList by
    performing a least squares fit.

    The find warp samples process in § 7.12.3 Find warp samples process is invoked with ref as input.

    A 2x2 matrix A, and two length 2 arrays Bx and By are constructed as follows:

     for ( i = 0; i < 2; i++ ) {
         for ( j = 0; j < 2; j++ ) {
             A[i][j] = 0
         }
         Bx[i] = 0
         By[i] = 0
     }
     w4 = Num_4x4_Blocks_Wide[MiSize]
     h4 = Num_4x4_Blocks_High[MiSize]
     midY = MiRow * 4 + h4 * 2 - 1
     midX = MiCol * 4 + w4 * 2 - 1
     suy = midY * 8
     sux = midX * 8
     duy = suy + BlockMvs[ref][0]




    AV2 Specification                                                                               Page 496 of 1169
 dux = sux + BlockMvs[ref][1]
 for ( i = 0; i < NumSamples[ ref ]; i++ ) {
     sy = CandList[ ref ][ i ][ 0 ] - suy
     sx = CandList[ ref ][ i ][ 1 ] - sux
     dy = CandList[ ref ][ i ][ 2 ] - duy
     dx = CandList[ ref ][ i ][ 3 ] - dux
     if ( Abs(sx - dx) < LS_MV_MAX && Abs(sy - dy) < LS_MV_MAX ) {
         A[0][0] += ls_product(sx, sx) + 8
         A[0][1] += ls_product(sx, sy) + 4
         A[1][1] += ls_product(sy, sy) + 8
         Bx[0] += ls_product(sx, dx) + 8
         Bx[1] += ls_product(sy, dx) + 4
         By[0] += ls_product(sx, dy) + 4
         By[1] += ls_product(sy, dy) + 8
     }
 }


where ls_product is specified as:

 ls_product(a, b) {
     return ( (a * b) >> 2) + (a + b)
 }



  NOTE:       The matrix A is symmetric so entry A[1][0] is omitted.


The variable det (containing the determinant of the matrix A) is set equal to A[0][0] * A[1][1] - A[0][1] *
A[0][1].

If det is equal to 0, the local warp parameters in LocalWarpParams are derived as follows:

 if ( det == 0 ) {
     for ( i = 2; i < 6; i++ ) {
         LocalWarpParams[ ref ][ i ] = ( i == 2 || i == 5 ) ?
                                       1 << WARPEDMODEL_PREC_BITS : 0
     }
     (LocalWarpParams[ref][0], LocalWarpParams[ref][1]) =
         get_warp_translation(LocalWarpParams[ref],ref)
 }


If det is equal to 0, this process terminates immediately.

The resolve divisor process specified in § 7.13.3.22 Resolve divisor process is invoked with det as input,
and the outputs are assigned to divShift and divFactor.

The local warp parameters in LocalWarpParams are derived as follows:

 divShift -= WARPEDMODEL_PREC_BITS
 if ( divShift < 0 ) {
     divFactor = divFactor << (-divShift)
     divShift = 0
 }
 LocalWarpParams[ ref ][ 2 ] = diag( A[1][1] * Bx[0] - A[0][1] * Bx[1] )
 LocalWarpParams[ ref ][ 3 ] = diag( -A[0][1] * Bx[0] + A[0][0] * Bx[1] )
 LocalWarpParams[ ref ][ 4 ] = diag( A[1][1] * By[0] - A[0][1] * By[1] )
 LocalWarpParams[ ref ][ 5 ] = diag( -A[0][1] * By[0] + A[0][0] * By[1] )
 LocalWarpParams[ ref ] = reduce_warp_model(LocalWarpParams[ ref ])
 (LocalWarpParams[ ref ][ 0 ], LocalWarpParams[ ref ][ 1 ]) =
     get_warp_translation( LocalWarpParams[ ref ], ref )




AV2 Specification                                                                              Page 497 of 1169
    where diag is specified to divide and clamp using divFactor and divShift as follows:

     diag(v) {
         return Clip3( INT32MIN, INT32MAX, Round2Signed(v * divFactor, divShift) )
     }


    The function get_warp_translation (which works out the required translation for the block) is specified as:

     get_warp_translation(params, refList) {
         w4 = Num_4x4_Blocks_Wide[ MiSize ]
         h4 = Num_4x4_Blocks_High[ MiSize ]
         midY = MiRow * 4 + h4 * 2 - 1
         midX = MiCol * 4 + w4 * 2 - 1
         mvx = BlockMvs[ refList ][ 1 ]
         mvy = BlockMvs[ refList ][ 0 ]
         vx = mvx * (1 << (WARPEDMODEL_PREC_BITS - 3)) -
             (midX * (params[2] - (1 << WARPEDMODEL_PREC_BITS)) + midY * params[3])
         vy = mvy * (1 << (WARPEDMODEL_PREC_BITS - 3)) -
             (midX * params[4] + midY * (params[5] - (1 << WARPEDMODEL_PREC_BITS)))
         cx = Clip3( -WARPEDMODEL_TRANS_CLAMP,
                     WARPEDMODEL_TRANS_CLAMP - (1 << WARP_PARAM_REDUCE_BITS), vx )
         cy = Clip3( -WARPEDMODEL_TRANS_CLAMP,
                     WARPEDMODEL_TRANS_CLAMP - (1 << WARP_PARAM_REDUCE_BITS), vy )
         return (cx, cy)
     }


    The function reduce_warp_model (which clamps and reduces the precision of a warp model to be ready
    for use in the warp filter) is specified as:

     reduce_warp_model( params ) {
         maxValue = (1 << (WARPEDMODEL_PREC_BITS - 1)) -
                    (1 << WARP_PARAM_REDUCE_BITS)
         minValue = -maxValue
         reducedParams[0] = params[0]
         reducedParams[1] = params[1]
         for (i = 2; i < 6; i++) {
             offset = (i == 2 || i == 5) ? (1 << WARPEDMODEL_PREC_BITS) : 0
             original = params[i] - offset
             clamped = Clip3(minValue, maxValue, original)
             rounded = Round2Signed(clamped, WARP_PARAM_REDUCE_BITS) <<
                           WARP_PARAM_REDUCE_BITS
             reducedParams[ i ] = rounded + offset
         }
         return reducedParams
     }


```

<a id="s-7-13-3-24"></a>

##### § 7.13.3.24 Extend warp estimation process

```text
§   7.13.3.24. Extend warp estimation process

    This process produces the array LocalWarpParams based on extending the warp parameters from a
    neighboring block with the motion vector for the current block.

    The input to this process is the motion vector mv for the current block.

    The extended warp parameters are computed in LocalWarpParams as follows:

     deltaRow = RefStackRowOffset[RefMvIdx]
     deltaCol = RefStackColOffset[RefMvIdx]
     if ( deltaRow != -1 && deltaCol != -1 ) {
         deltaRow = ExtendDeltaRow
         deltaCol = ExtendDeltaCol




    AV2 Specification                                                                            Page 498 of 1169
 }
 mvRow = MiRow + deltaRow
 mvCol = MiCol + deltaCol
 ref = RefFrame[ 0 ]
 neighborRef = RefFrames[ mvRow ][ mvCol ][ 0 ] == ref ? 0 : 1
 if ( MotionModes[ mvRow ][ mvCol ] >= LOCALWARP ) {
     params = WarpParams[ mvRow ][ mvCol ][ 0 ]
 } else if ( is_global_mv_block( mvRow, mvCol, neighborRef ) ) {
     params = gm_params[RefFrames[ mvRow ][ mvCol ][ neighborRef ]]
 } else {
     for( i = 0; i < 6; i++) {
          params[ i ] = Default_Warp_Params[ i ]
     }
     params[0] = Mvs[ mvRow ][ mvCol ][ neighborRef ][ 1 ] <<
                      (WARPEDMODEL_PREC_BITS - 3)
     params[1] = Mvs[ mvRow ][ mvCol ][ neighborRef ][ 0 ] <<
                      (WARPEDMODEL_PREC_BITS - 3)
 }
 w4 = Num_4x4_Blocks_Wide[MiSize]
 h4 = Num_4x4_Blocks_High[MiSize]
 midY = MiRow * 4 + h4 * 2 - 1
 midX = MiCol * 4 + w4 * 2 - 1
 mvx = mv[ 1 ]
 mvy = mv[ 0 ]
 projMidX = (midX << WARPEDMODEL_PREC_BITS) +
             (mvx << (WARPEDMODEL_PREC_BITS - 3))
 projMidY = (midY << WARPEDMODEL_PREC_BITS) +
             (mvy << (WARPEDMODEL_PREC_BITS - 3) )

 neighborIsAbove = deltaRow == -1 && deltaCol >= 0
 extendWarpParams[0] = 0
 extendWarpParams[1] = 0
 if (neighborIsAbove) {
     extendWarpParams[ 2 ] = params[ 2 ]
     extendWarpParams[ 4 ] = params[ 4 ]
     aboveX = midX
     aboveY = MiRow * 4 - 1
     projAboveX = params[ 2 ] * aboveX + params[ 3 ] * aboveY + params[ 0 ]
     projAboveY = params[ 4 ] * aboveX + params[ 5 ] * aboveY + params[ 1 ]
     extendWarpParams[ 3 ] = Round2( projMidX - projAboveX,
                  Mi_Height_Log2[MiSize] + MI_SIZE_LOG2 - 1)
     extendWarpParams[ 5 ] = Round2( projMidY - projAboveY,
                  Mi_Height_Log2[MiSize] + MI_SIZE_LOG2 - 1)
 } else {
     extendWarpParams[ 3 ] = params[ 3 ]
     extendWarpParams[ 5 ] = params[ 5 ]
     leftX = MiCol * 4 - 1
     leftY = midY
     projLeftX = params[ 2 ] * leftX + params [3 ] * leftY + params[ 0 ]
     projLeftY = params[ 4 ] * leftX + params[ 5 ] * leftY + params[ 1 ]
     extendWarpParams[2] = Round2( projMidX - projLeftX,
                  Mi_Width_Log2[MiSize] + MI_SIZE_LOG2 - 1)
     extendWarpParams[4] = Round2( projMidY - projLeftY,
                  Mi_Width_Log2[MiSize] + MI_SIZE_LOG2 - 1)
 }
 LocalWarpParams[ 0 ] = reduce_warp_model( extendWarpParams )
 (LocalWarpParams[ 0 ][ 0 ], LocalWarpParams[ 0 ][ 1 ]) =
     get_warp_translation( LocalWarpParams[ 0 ], 0 )


The function is_global_mv_block (which works out if a block used global warp) is specified as:

 is_global_mv_block(mvRow, mvCol, mvList) {
     candMode = YModes[ mvRow ][ mvCol ]
     candSize = MiSizes[ PlaneStart ][ mvRow ][ mvCol ]
     return is_global_mv_cand( candMode, candSize,
                               RefFrames[ mvRow ][ mvCol ][ mvList ] )
 }




AV2 Specification                                                                           Page 499 of 1169
    The function is_global_mv_cand (which works out if a given candidate block used global warp) is specified
    as:

     is_global_mv_cand( candMode, candSize, candRef ) {
         large = ( Min( Block_Width[ candSize ],Block_Height[ candSize ] ) >= 8 )
         return ( candMode == GLOBALMV || candMode == GLOBAL_GLOBALMV ) &&
                 GmType[ candRef ] > IDENTITY &&
                 large
     }


```

<a id="s-7-13-3-25"></a>

##### § 7.13.3.25 Block adaptive weighted prediction process

```text
§   7.13.3.25. Block adaptive weighted prediction process

    The inputs to this process are:

      • a variable plane specifying which plane is being predicted,
      • variables x and y specifying the location of the top left sample in the CurrFrame[ 0 ] array of the
        region to be adapted,
      • variables w and h specifying the width and height of the block,
      • an array mv specifying the motion vector for the block,
      • a variable morphPred specifying whether to use the current frame as the reference.

    The outputs of this process are modified inter predicted samples in the current frame CurrFrame.

    This process adjusts the inter predicted samples for the current block to try and match adjustments
    required for the surrounding samples.

    Variables describing the location of the block (refX and refY) in the reference frame and the size of the
    block that is within planeWidth and planeHeight (bw and bh) are derived as:

     if ( plane == 0 ) {
         plane = 0
         subX = 0
         subY = 0
     } else {
         subX = SubsamplingX
         subY = SubsamplingY
     }
     planeWidth = MiCols * MI_SIZE >> subX
     planeHeight = MiRows * MI_SIZE >> subY
     bw = Min(planeWidth - x, w)
     bh = Min(planeHeight - y, h)
     dy = to_fullmv( mv[0] )
     dx = to_fullmv( mv[1] )
     refY = ( MiRow * MI_SIZE + dy ) >> subY
     refX = ( MiCol * MI_SIZE + dx ) >> subX


    The reference prevFrame (specifying which frame to use for the reference template) is set as follows:

     if ( morphPred ) {
         prevFrame = CurrFrame
     } else {
         refIdx = ref_frame_idx[ RefFrame[ 0 ] ]
         prevFrame = FrameStore[ refIdx ]
     }




    AV2 Specification                                                                             Page 500 of 1169
It is a requirement of bitstream conformance that all the following are true whenever this process is
invoked:

  • refX is greater than or equal to 1.
  • refY is greater than or equal to 1.
  • refX + bw is less than or equal to planeWidth.
  • refY + bh is less than or equal to planeHeight.


  NOTE:       This ensures that the samples needed from the reference block are within the frame.


The adaptation parameters are set as follows:

 shift = 8
 alpha = 1 << 8
 beta = -(1 << 7)
 sumX = 0
 sumY = 0
 sumXX = 0
 sumXY = 0
 count = 0
 if (plane == 0) {
     bw2 = Min(16,bw)
     bh2 = Min(16,bh)
 } else {
     bw2 = Min(8,bw)
     bh2 = Min(8,bh)
 }
 width = bw2 == 12 ? 8 : bw2
 height = bh2 == 12 ? 8 : bh2
 numUp = 0
 numLeft = 0
 if (AvailU && AvailL) {
     if (width == 16 && height == 16) {
          numUp = 16
          numLeft = 16
     } else if (width > 4 && height > 4) {
          numUp = 8
          numLeft = 8
     } else if (width < 16 && height < 16) {
          numUp = 4
          numLeft = 4
     } else if (width == 16) {
          numUp = 16
     } else {
          numLeft = 16
     }
 } else if (AvailU) {
     numUp = width
 } else if (AvailL) {
     numLeft = height
 }
 if (numUp > 0) {
     upStep = width / numUp
     for( i = upStep >> 1; i < width; i += upStep ) {
          recon = CurrFrame[plane][y - 1][x + i]
          ref = prevFrame[ plane ][refY - 1][refX + i]
          sumX += ref
          sumY += recon
          sumXY += ref * recon
          sumXX += ref * ref
     }
     count += numUp
 }



AV2 Specification                                                                           Page 501 of 1169
 if (numLeft > 0) {
     leftStep = height / numLeft
     for( i = leftStep >> 1; i < height; i+= leftStep ) {
         recon = CurrFrame[plane][y + i][x - 1]
         ref = prevFrame[ plane ][refY + i][refX - 1]
         sumX += ref
         sumY += recon
         sumXY += ref * recon
         sumXX += ref * ref
     }
     count += numLeft
 }
 if ( plane > 0 ) {
     alpha = BawpAlpha
     if ( count == 0 ) {
         alpha = 1 << 8
     }
 } else if ( explicit_bawp && !morphPred ) {
     firstRefDist = Abs( get_relative_dist( OrderHints[ RefFrame[ 0 ] ],
                                             OrderHint ) )
     listIndex = (YMode == NEARMV) ? 0 :
                                     ( (YMode == NEWMV && use_amvd) ? 1 : 2 )
     scale = listIndex + 1
     if (firstRefDist > 4) {
         scale += 1
     }
     if (!explicit_bawp_scale) {
         scale = -scale
     }
     alpha = 256 + 16 * scale
 } else if ( count > 0 ) {
     nor = sumXY - sumX * sumY / count
     der = sumXX - sumX * sumX / count
     if ( der != 0 && nor != 0 ) {
         alpha = resolve_division(nor, der, shift)
         if (alpha == 0) {
              alpha = 1 << shift
         }
     } else {
         alpha = 1 << shift
     }
 }
 if ( count > 0 ) {
     beta = ( (sumY << shift) - sumX * alpha ) / count
 }
 if ( plane == 0 && !morphPred ) {
     BawpAlpha = alpha
 }


where the function resolve_division(N, D, shift) approximates the division (N << shift) / D and is defined
as:

 resolve_division(N, D, shift) {
     signN = N < 0
     N = Abs(N)
     shiftN = FloorLog2(N)
     shiftD = FloorLog2(D)
     eD = D - (1 << shiftD)
     if (shiftD > DIV_LUT_BITS)
          fD = Round2(eD, shiftD - DIV_LUT_BITS)
     else
          fD = eD << (DIV_LUT_BITS - shiftD)
     if (shiftN > DIV_LUT_BITS)
          fN = Round2(N, shiftN - DIV_LUT_BITS)
     else
          fN = N << (DIV_LUT_BITS - shiftN)
     shiftAdd = shiftD - shiftN - shift
     if (shiftAdd <= 1) {



AV2 Specification                                                                             Page 502 of 1169
              shift0 = (DIV_LUT_PREC_BITS + DIV_LUT_BITS + shiftAdd)
              if ( shift0 >= 0 ) {
                   ret = (Div_Lut[fD] * fN) >> shift0
              } else {
                   ret = (2 << shift) - 1
              }
          } else {
              ret = 0
          }
          ret = Min( (2 << shift) - 1, ret)
          if (signN) ret = -ret
          return ret
     }


    Finally the samples in the block are adjusted as follows:

     for( i = 0 ; i < h ; i++ ) {
         for( j = 0; j < w; j++ ) {
             orig = CurrFrame[ plane ][ y + i ][ x + j ]
             CurrFrame[ plane ][ y + i ][ x + j ] =
                 Clip1( (orig * alpha + beta) >> shift )
         }
     }



      NOTE: This adjusts all the samples in the block, not just the samples within planeWidth and
      planeHeight.


      NOTE: The default parameters of alpha equal to 256 and beta equal to -128 (used if the current
      block is at the top-left of a tile) will subtract 1 off every sample value.

```

<a id="s-7-13-3-26"></a>

##### § 7.13.3.26 Build morphological prediction process

```text
§   7.13.3.26. Build morphological prediction process

    The inputs to this process are:

      • variables x and y specifying the location of the top left sample in the CurrFrame[ 0 ] array of the
        region to be adapted,
      • variables w and h specifying the width and height of the block,
      • an array mv specifying the motion vector used for intra block copy.

    The block adaptive weighted prediction process specified in § 7.13.3.25 Block adaptive weighted
    prediction process is invoked with plane set equal to 0, x, y, w, h, mv, and morphPred set equal to 1 as
    inputs.

```

<a id="s-7-13-3-27"></a>

##### § 7.13.3.27 Wedge mask process

```text
§   7.13.3.27. Wedge mask process

    The input to this process is:

      • variables w and h specifying the width and height of the region to be predicted.

    This process sets up a mask array for the luma samples.




    AV2 Specification                                                                             Page 503 of 1169
The mask is specified as:

 for ( i = 0; i < h; i++ ) {
     for ( j = 0; j < w; j++ ) {
         Mask[ i ][ j ] =
             WedgeMasks[ MiSize ][ wedge_sign ][ WedgeIndex ][ i ][ j ]
     }
 }


where WedgeMasks is a fixed lookup table that is generated by the following function:

 initialise_wedge_mask_table( ) {
     w = MASK_MASTER_SIZE
     h = MASK_MASTER_SIZE
     for( boundary = 0; boundary < WEDGE_BOUNDARY_TYPES; boundary++ ) {
         for( angle = 0; angle < WEDGE_ANGLES; angle++ ) {
             for( n = 0; n < h; n++ ) {
                 y = ((n << 1) - h + 1) * Wedge_Sin_Lut[ angle ]
                 for( m = 0; m < w; m++ ) {
                     d = ((m << 1) - w + 1) * Wedge_Cos_Lut[ angle ] + y
                     if ( boundary == WEDGE_BOUNDARY_SHARP ) {
                          d = d * 2
                     }
                     clamp_d = Clip3( -31, 31, d )
                     MasterMask[ boundary ][ angle ][ n ][ m ] =
                     (clamp_d >= 0 ? Pos_Dist_2_Bld_Weight[ clamp_d ]
                                    : Neg_Dist_2_Bld_Weight[ -clamp_d ]) << 2

                    }
                }
          }
      }
      for ( bsize = BLOCK_8X8; bsize < BLOCK_SIZES; bsize++ ) {
          if ( Wedge_Bits[ bsize ] > 0 ) {
              w = Block_Width[ bsize ]
              h = Block_Height[ bsize ]
              boundary = bsize <= BLOCK_16X16 ? WEDGE_BOUNDARY_SHARP
                                              : WEDGE_BOUNDARY_SMOOTH
              for( wedge = 0; wedge < WEDGE_TYPES; wedge++ ) {
                  dir = Wedge_Codebook[ wedge ][ 0 ]
                  xoff = MASK_MASTER_SIZE / 2 -
                         ((Wedge_Codebook[ wedge ][ 1 ] * w) >> 3)
                  yoff = MASK_MASTER_SIZE / 2 -
                         ((Wedge_Codebook[ wedge ][ 2 ] * h) >> 3)
                  flipSign = 0
                  for ( i = 0; i < h; i++ ) {
                      for ( j = 0; j < w; j++ ) {
                        WedgeMasks[ bsize ][ flipSign ][ wedge ][ i ][ j ] =
                            MasterMask[ boundary ][ dir ][ yoff+i ][ xoff+j ]
                        WedgeMasks[ bsize ][ !flipSign ][ wedge ][ i ][ j ] =
                            64 - MasterMask[ boundary ][ dir ][ yoff+i ][ xoff+j ]
                      }
                  }
              }
          }
      }
 }


The lookup tables are defined as:

 Wedge_Cos_Lut[WEDGE_ANGLES] = {
     4, 4, 4, 2, 2,
     0,-2,-2,-4,-4,
     -4,-4,-4,-2,-2,
     0, 2, 2, 4, 4



AV2 Specification                                                                       Page 504 of 1169
     }

     Wedge_Sin_Lut[WEDGE_ANGLES] = {
         0, -1,-2,-2,-4,
         -4,-4,-2,-2, -1,
         0, 1, 2, 2, 4,
         4, 4, 2, 2, 1
     }

     Pos_Dist_2_Bld_Weight[WEDGE_BLD_LUT_SIZE] = {
          8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 12, 13, 13, 13, 14, 14,
         14, 14, 14, 15, 15, 15, 15, 15, 15, 15, 15, 15, 16, 16, 16, 16
     }

     Neg_Dist_2_Bld_Weight[WEDGE_BLD_LUT_SIZE] = {
         8, 8, 7, 7, 6, 6, 5, 5, 4, 4, 4, 3, 3, 3, 2, 2,
         2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0
     }


    The Wedge_Codebook (which gives the direction and offset to the wedge for each wedge index) is defined
    as:

     Wedge_Codebook[WEDGE_TYPES][3] = {
         { WEDGE_0, 5, 4 },   { WEDGE_0, 6, 4 },   { WEDGE_0, 7, 4 },
         { WEDGE_14, 4, 4 }, { WEDGE_14, 5, 4 }, { WEDGE_14, 6, 4 },
         { WEDGE_14, 7, 4 }, { WEDGE_27, 4, 4 }, { WEDGE_27, 5, 4 },
         { WEDGE_27, 6, 4 }, { WEDGE_27, 7, 4 }, { WEDGE_45, 4, 4 },
         { WEDGE_45, 5, 4 }, { WEDGE_45, 6, 4 }, { WEDGE_45, 7, 4 },
         { WEDGE_63, 4, 4 }, { WEDGE_63, 4, 3 }, { WEDGE_63, 4, 2 },
         { WEDGE_63, 4, 1 }, { WEDGE_90, 4, 3 }, { WEDGE_90, 4, 2 },
         { WEDGE_90, 4, 1 }, { WEDGE_117, 4, 4 }, { WEDGE_117, 4, 3 },
         { WEDGE_117, 4, 2 }, { WEDGE_117, 4, 1 }, { WEDGE_135, 4, 4 },
         { WEDGE_135, 3, 4 }, { WEDGE_135, 2, 4 }, { WEDGE_135, 1, 4 },
         { WEDGE_153, 4, 4 }, { WEDGE_153, 3, 4 }, { WEDGE_153, 2, 4 },
         { WEDGE_153, 1, 4 }, { WEDGE_166, 4, 4 }, { WEDGE_166, 3, 4 },
         { WEDGE_166, 2, 4 }, { WEDGE_166, 1, 4 }, { WEDGE_180, 3, 4 },
         { WEDGE_180, 2, 4 }, { WEDGE_180, 1, 4 }, { WEDGE_194, 3, 4 },
         { WEDGE_194, 2, 4 }, { WEDGE_194, 1, 4 }, { WEDGE_207, 3, 4 },
         { WEDGE_207, 2, 4 }, { WEDGE_207, 1, 4 }, { WEDGE_225, 3, 4 },
         { WEDGE_225, 2, 4 }, { WEDGE_225, 1, 4 }, { WEDGE_243, 4, 5 },
         { WEDGE_243, 4, 6 }, { WEDGE_243, 4, 7 }, { WEDGE_270, 4, 5 },
         { WEDGE_270, 4, 6 }, { WEDGE_270, 4, 7 }, { WEDGE_297, 4, 5 },
         { WEDGE_297, 4, 6 }, { WEDGE_297, 4, 7 }, { WEDGE_315, 5, 4 },
         { WEDGE_315, 6, 4 }, { WEDGE_315, 7, 4 }, { WEDGE_333, 5, 4 },
         { WEDGE_333, 6, 4 }, { WEDGE_333, 7, 4 }, { WEDGE_346, 5, 4 },
         { WEDGE_346, 6, 4 }, { WEDGE_346, 7, 4 }
     }


```

<a id="s-7-13-3-28"></a>

##### § 7.13.3.28 Difference weight mask process

```text
§   7.13.3.28. Difference weight mask process

    The inputs to this process are variables w and h specifying the width and height of the region to be
    predicted.

    This process prepares an array Mask containing the blending weights for the luma samples.

    The process sets the array based on the difference between the two predictions as follows:

     for ( i = 0; i < h; i++ ) {
         for ( j = 0; j < w; j++ ) {
             diff = Abs(Preds[ 0 ][ i ][ j ] - Preds[ 1 ][ i ][ j ])
             diff = Round2(diff, (BitDepth - 8) + InterPostRound)
             m = Clip3(0, 64, 38 + diff / 16)
             if ( mask_type )
                 Mask[ i ][ j ] = 64 - m



    AV2 Specification                                                                            Page 505 of 1169
               else
                      Mask[ i ][ j ] = m

          }
     }


```

<a id="s-7-13-3-29"></a>

##### § 7.13.3.29 Intra mode variant mask process

```text
§   7.13.3.29. Intra mode variant mask process

    The input to this process is:

      • variables w and h specifying the width and height of the region to be predicted.

    This process prepares an array Mask containing the blending weights for the luma samples.

    The process sets the array based on the mode used for intra prediction as follows:

     sizeScale = 128 / Max( h, w )
     for ( i = 0; i < h; i++ ) {
         for ( j = 0; j < w; j++ ) {
             if ( interintra_mode == II_V_PRED ) {
                 Mask[ i ][ j ] = Ii_Weights_1d[ i * sizeScale ]
             } else if ( interintra_mode == II_H_PRED ) {
                 Mask[ i ][ j ] = Ii_Weights_1d[ j * sizeScale ]
             } else if ( interintra_mode == II_SMOOTH_PRED ) {
                 Mask[ i ][ j ] = Ii_Weights_1d[ Min(i, j) * sizeScale ]
             } else {
                 Mask[ i ][ j ] = 32
             }
         }
     }


    where the table Ii_Weights_1d is defined as:

     Ii_Weights_1d[ 128 ] = {
       60, 58, 56, 54, 52, 50, 48, 47, 45, 44, 42, 41, 39, 38, 37, 35, 34, 33, 32,
       31, 30, 29, 28, 27, 26, 25, 24, 23, 22, 22, 21, 20, 19, 19, 18, 18, 17, 16,
       16, 15, 15, 14, 14, 13, 13, 12, 12, 12, 11, 11, 10, 10, 10, 9, 9, 9, 8,
       8, 8, 8, 7, 7, 7, 7, 6, 6, 6, 6, 6, 5, 5, 5, 5, 5, 4, 4,
       4, 4, 4, 4, 4, 4, 3, 3, 3, 3, 3, 3, 3, 3, 3, 2, 2, 2, 2,
       2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1,
       1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1
     }


```

<a id="s-7-13-3-30"></a>

##### § 7.13.3.30 Mask blend process

```text
§   7.13.3.30. Mask blend process

    The inputs to this process are:

      • a variable plane specifying which plane is being predicted,
      • variables dstX and dstY specifying the location of the top left sample in the CurrFrame[ plane ] array
        of the region to be predicted,
      • variables w and h specifying the width and height of the region to be predicted.

    The process combines two predictions according to the mask. It makes use of an array Mask containing
    the blending weights to apply (the weights are defined for the current plane samples if compound_type is
    equal to COMPOUND_INTRA, or the luma plane otherwise).




    AV2 Specification                                                                            Page 506 of 1169
    The variables subX and subY describing the subsampling of the current plane are derived as follows:

      • If plane is equal to 0, subX and subY are set equal to 0.
      • Otherwise (plane is not equal to 0), subX is set equal to SubsamplingX and subY is set equal to
        SubsamplingY.

    The process is specified as follows:

     for ( y = 0; y < h; y++ ) {
         for ( x = 0; x < w; x++ ) {
             if ( ( !subX ) ||
                  (inter_intra && !wedge_interintra) ) {
                 m = Mask[ y ][ x ]
             } else if ( !subY ) {
                 m = Round2( Mask[ y ][ 2*x ] + Mask[ y ][ 2*x+1 ], 1 )
             } else {
                 m = Round2( Mask[ 2*y ][ 2*x ] + Mask[ 2*y ][ 2*x+1 ] +
                      Mask[ 2*y+1 ][ 2*x ] + Mask[ 2*y+1 ][ 2*x+1 ], 2 )
             }
             if ( inter_intra ) {
                 pred0 = IntraPred[ y ][ x ]
                 pred1 = CurrFrame[ plane ][ y + dstY ][ x + dstX ]
                 CurrFrame[ plane ][ y + dstY ][ x + dstX ] =
                      Round2( m * pred0 + (64 - m) * pred1, 6 )
             } else {
                 pred0 = Preds[ 0 ][ y ][ x ]
                 pred1 = Preds[ 1 ][ y ][ x ]
                 CurrFrame[ plane ][ y + dstY ][ x + dstX ] =
                      Clip1(Round2(m * pred0 + (64 - m) * pred1, 6 + InterPostRound))
             }
         }
     }


```

<a id="s-7-13-4"></a>

#### § 7.13.4 Palette prediction process

```text
§   7.13.4. Palette prediction process

    The palette prediction process is invoked for palette coded intra blocks to predict a part of the block
    using the limited palette.

    The inputs to this process are:

      • variables startX and startY specifying the location of the top left sample in the CurrFrame[ plane ]
        array of the current transform block,
      • variables x and y specifying the location in 4x4 units relative to the top left sample of the current
        transform block,
      • a variable txSz specifying the size of the current transform block.

    The outputs of this process are palette predicted samples in the current frame CurrFrame.

    The variable w specifying the width of the transform block is set equal to Tx_Width[ txSz ].

    The variable h specifying the height of the transform block is set equal to Tx_Height[ txSz ].

    The current frame is updated as follows:

      • CurrFrame[ 0 ][ startY + i ][ startX + j ] is set equal to palette_colors_y[ ColorMapY[ y * 4 + i ][ x * 4
        + j ] ] for i = 0..h-1 and j = 0..w-1.




    AV2 Specification                                                                                Page 507 of 1169
```

<a id="s-7-13-5"></a>

#### § 7.13.5 Predict chroma from luma process

```text
§   7.13.5. Predict chroma from luma process

    The chroma from luma process uses reconstructed luma samples to form a prediction for the chroma
    samples. The high frequencies are taken from the reconstructed luma samples and combined with DC
    predicted chroma samples.

    The inputs to this process are:

      • a variable plane (greater than zero) specifying which plane is being predicted,
      • variables startX and startY specifying the location of the top left sample in the CurrFrame[ plane ]
        array of the current transform block,
      • a variable txSz specifying the size of the current transform block.

    The outputs of this process are modified chroma predicted samples in the current frame CurrFrame.

    If cfl_index is equal to CFL_MULTI, the mhccp process specified in § 7.13.6 MHCCP process is invoked
    with plane, startX, startY, and txSz as inputs, and then this process immediately terminates.

    The variable w specifying the width of the transform block is set equal to Tx_Width[ txSz ].

    The variable h specifying the height of the transform block is set equal to Tx_Height[ txSz ].

    The variable subX is set equal to SubsamplingX.

    The variable subY is set equal to SubsamplingY.

    The variable lumaAvg (with an estimate of the average luma value) is prepared as follows:

     stepW = w > 32 ? 2 : 1
     stepH = h > 32 ? 2 : 1
     x = startX << subX
     y = startY << subY
     filterIdx = cfl_ds_filter_index
     if (filterIdx == 3) {
         filterIdx = 0
     }
     lumaSum = 0
     lumaCount = 0
     for (i = 0; i < w; i++) {
         lumaAbove[ i ] = 0
     }
     for (j = 0; j < h; j++) {
         lumaLeft[ j ] = 0
     }
     if ( AvailUChroma ) {
         prevSample = (1 << (BitDepth - 1))
         log2SbH = Mi_Height_Log2[ SbSize ]
         sbTop = (MiRow >> log2SbH) << (log2SbH + MI_SIZE_LOG2)
         for (i = 0; i < w; i++) {
             t = 0
             if (i >= (MiCols * MI_SIZE - x) >> subX) {
                 t = prevSample
             } else {
                 for (dy = -subY; dy <= subY; dy++) {
                      for (dx = -subX; dx <= subX; dx++) {
                          v = CurrFrame[ 0 ]
                                       [ Max(sbTop - 1,y - 1 - subY + dy) ]
                                       [ x + Max(0, (i << subX) + dx) ]
                          if (subX && subY) {
                              t += Cfl_Filters_420[filterIdx][dy + subY][dx + subX]*v
                          } else if (subX) {



    AV2 Specification                                                                                Page 508 of 1169
                            t += Cfl_Filters_422[filterIdx][dx + subX] * v
                        } else {
                            t += 8 * v
                        }
                    }
                }
           }
           if ( i % stepW == 0 ) {
               lumaSum += t
               lumaAbove[ i ] = t >> 3
           }
           prevSample = t
      }
      lumaCount += w / stepW
 }
 if ( AvailLChroma ) {
     prevSample = (1 << (BitDepth - 1))
     for( j = 0; j < h; j++ ) {
         t = 0
         if ( j >= (MiRows * MI_SIZE - y) >> subY ) {
             t = prevSample
         } else {
             for( dy = -subY; dy <= subY; dy++ ) {
                  for( dx = -subX; dx <= subX; dx++ ) {
                      v = CurrFrame[ 0 ]
                                   [ y + Max(0, (j << subY) + dy) ]
                                   [ x - 1 - subX + dx ]
                      if (subX && subY) {
                          t += Cfl_Filters_420[filterIdx][dy + subY][dx + subX]*v
                      } else if (subX) {
                          t += Cfl_Filters_422[filterIdx][dx + subX] * v
                      } else {
                          t += 8 * v
                      }
                  }
             }
         }
         if ( j % stepH == 0 ) {
             lumaSum += t
             lumaLeft[ j ] = t >> 3
         }
         prevSample = t
     }
     lumaCount += h / stepH
 }
 lumaAvg = 8 << (BitDepth - 1)
 if (lumaCount > 0) {
     lumaAvg = Min( (8 << BitDepth) - 1, approx_divide( lumaSum, lumaCount ) )
 }


where the constant tables Cfl_Filters_420 and Cfl_Filters_422 are defined as follows:


 Cfl_Filters_420[ 3 ][ 3 ][ 3 ] = {
     {{0, 0, 0},
      {0, 2, 2},
      {0, 2, 2}},
     {{0, 0, 0},
      {1, 2, 1},
      {1, 2, 1}},
     {{0, 1, 0},
      {1, 4, 1},
      {0, 1, 0}}
 }
 Cfl_Filters_422[ 3 ][ 3 ] = {
     {0, 4, 4},




AV2 Specification                                                                       Page 509 of 1169
      {2, 4, 2},
      {0, 8, 0}
 }


The variable implicitAlpha is prepared based on correlations between luma and chroma as follows:

 implicitAlpha = 0
 if (cfl_index == CFL_DERIVED_ALPHA) {
     count = 0
     sumX = 0
     sumY = 0
     sumXY = 0
     sumXX = 0
     if ( AvailUChroma && AvailLChroma ) {
         if (w > h * 2) {
              numLeft = 0
              numAbove = NUM_REF_SAM_CFL
         } else if (h > w * 2) {
              numAbove = 0
              numLeft = NUM_REF_SAM_CFL
         } else {
              numAbove = NUM_REF_SAM_CFL >> 1
              numLeft = NUM_REF_SAM_CFL >> 1
         }
     } else {
         numAbove = AvailUChroma ? NUM_REF_SAM_CFL : 0
         numLeft = AvailLChroma ? NUM_REF_SAM_CFL : 0
     }
     numAbove = Min(numAbove, w)
     numLeft = Min(numLeft, h)
     if (numAbove > 0) {
         step = w / numAbove
         prevSample = 1 << (BitDepth - 1)
         for ( i = 0; i < w; i++ ) {
              sample = prevSample
              if (startX + i < (MiCols * MI_SIZE >> subX) ) {
                  sample = CurrFrame[ plane ][ startY - 1 ][ startX + i ]
              }
              samples[ i ] = sample
              prevSample = sample
         }
         for (i = step >> 1; i < w; i += step) {
              sample = samples[ i ]
              sumX += lumaAbove [ i ]
              sumY += sample
              sumXY += lumaAbove[ i ] * sample
              sumXX += lumaAbove[ i ] * lumaAbove[ i ]
              count++
         }
     }
     if (numLeft > 0) {
         step = h / numLeft
         prevSample = 1 << (BitDepth - 1)
         for ( j = 0; j < h; j++ ) {
              sample = prevSample
              if (j + startY < (MiRows * MI_SIZE >> subY) ) {
                  sample = CurrFrame[ plane ][ startY + j ][ startX - 1 ]
              }
              samples[ j ] = sample
              prevSample = sample
         }
         for ( j = step >> 1; j < h; j += step ) {
              sample = samples[ j ]
              sumX += lumaLeft[ j ]
              sumY += sample
              sumXY += lumaLeft[ j ] * sample
              sumXX += lumaLeft[ j ] * lumaLeft[ j ]
              count++
         }



AV2 Specification                                                                        Page 510 of 1169
          }
          if (count > 0) {
              der = sumXX - (sumX * sumX) / count
              nor = sumXY - (sumX * sumY) / count
              shift = 8
              if ( der != 0 && nor != 0 ) {
                  implicitAlpha = resolve_division(nor, der, shift)
              }
          }
     }


    An array L (containing subsampled reconstructed luma samples with 3 fractional bits of precision) and
    lumaAvg (representing the average reconstructed luma intensity with 3 fractional bits of precision) is
    specified as:

     for ( i = 0; i < h; i++ ) {
         lumaY = (startY + i) << subY
         clampY = i == 0 || lumaY % 64 == 0
         for ( j = 0; j < w; j++ ) {
             lumaX = (startX + j) << subX
             clampX = j == 0 || lumaX % 64 == 0
             t = 0
             for (dy = -subY; dy <= subY; dy++) {
                 for (dx = -subX; dx <= subX; dx++) {
                     v = CurrFrame[ 0 ]
                                   [ lumaY + (clampY ? Max(dy, 0) : dy) ]
                                   [ lumaX + (clampX ? Max(dx, 0) : dx) ]
                     if (subX && subY) {
                         t += Cfl_Filters_420[filterIdx][dy + subY][dx + subX] * v
                     } else if (subX) {
                         t += Cfl_Filters_422[filterIdx][dx + subX] * v
                     } else {
                         t = 8 * v
                     }
                 }
             }
             L[ i ][ j ] = t
         }
     }


    The variable alpha is prepared depending on cfl_index as follows:

     if ( cfl_index == CFL_DERIVED_ALPHA ) alpha = implicitAlpha
     else if ( plane == 1 ) alpha = CflAlphaU * 32
     else alpha = CflAlphaV * 32


    The predicted chroma samples are specified as:

     for ( i = 0; i < h; i++ ) {
         for ( j = 0; j < w; j++ ) {
             dc = CurrFrame[ plane ][ startY + i ][ startX + j ]
             scaledLuma = Round2Signed( alpha * ( L[ i ][ j ] - lumaAvg ), 11 )
             CurrFrame[ plane ][ startY + i ][ startX + j ] = Clip1(dc + scaledLuma)
         }
     }


```

<a id="s-7-13-6"></a>

#### § 7.13.6 MHCCP process

```text
§   7.13.6. MHCCP process

    The inputs to this process are:

      • a variable plane (greater than zero) specifying which plane is being predicted,


    AV2 Specification                                                                          Page 511 of 1169
      • variables startX and startY specifying the location of the top left sample in the CurrFrame[ plane ]
        array of the current transform block,
      • a variable txSz specifying the size of the current transform block.

    The outputs of this process are modified chroma predicted samples in the current frame CurrFrame.

    The variable w specifying the width of the transform block is set equal to Tx_Width[ txSz ].

    The variable h specifying the height of the transform block is set equal to Tx_Height[ txSz ].

    The derive multi param process specified in § 7.13.7 Derive multi param process is invoked and the
    output is assigned to multiParams.

    The samples are predicted as follows:

     vec[2] = 1 << (BitDepth - 1)
     for ( i = 0; i < h; i++ ) {
         for ( j = 0; j < w; j++ ) {
             a = CflRef[0][CflAbove + i][CflLeft + j]
             if ( cfl_mh_dir == 0 ) {
                 vec[0] = a
             } else if ( cfl_mh_dir == 1 ) {
                 vec[0] = CflRef[0][Max(0,CflAbove + i - 1)][CflLeft + j]
             } else {
                 vec[0] = CflRef[0][CflAbove + i][Max(0,CflLeft + j - 1)]
             }
             vec[1] = Round2( a * a,BitDepth )
             t = 0
             for( k = 0; k < 3; k++ ) {
                 t += mul_fixed32_adapt(multiParams[k], vec[k], MHCCP_BITS)
             }
             CurrFrame[ plane ][ startY + i ][ startX + j ] = Clip1( t )
         }
     }


    where the function mul_fixed32_adapt (which performs multiplication and right shift with adjustments
    made to ensure arithmetic can work with 32 bit signed integers) is specified as:

     mul_fixed32_adapt(a, b, shift) {
         bitsA = GetMsb( Abs( a ) ) + 1
         bitsB = GetMsb( Abs( b ) ) + 1
         need = Max( 0, bitsA + bitsB - 29 )
         s1 = need >> 1
         s2 = need - s1
         adj = shift - (s1 + s2)
         prod = ( a >> s1 ) * ( b >> s2 )
         if ( adj <= 0 ) {
             return prod
         } else if ( adj > 29 ) {
             return 0
         } else {
             return Round2Signed( prod, adj )
         }
     }


```

<a id="s-7-13-7"></a>

#### § 7.13.7 Derive multi param process

```text
§   7.13.7. Derive multi param process

    This process works out the best (in a least squares sense) parameters to use to predict the chroma
    samples from luma samples.



    AV2 Specification                                                                                Page 512 of 1169
All elements of a 1d array b of length 3 are set equal to 0.

All elements of a 2d array ata of size 3 by 3 are set equal to 0.

Statistics about the reference samples are collected as follows:

 v[2] = 1 << (BitDepth - 1)
 count = 0
 for (i = 1; i < (CflRefHeight >> SubsamplingY) - 1; i++) {
     for (j = 1; j < (CflRefWidth >> SubsamplingX) - 1; j++) {
         if ( i < CflAbove || j < CflLeft ) {
             if (cfl_mh_dir == 0) {
                 v[0] = CflRef[0][i][j]
             } else if (cfl_mh_dir == 1) {
                 v[0] = CflRef[0][i - 1][j]
             } else {
                 v[0] = CflRef[0][i][j - 1]
             }
             v[1] = Round2( CflRef[0][i][j] * CflRef[0][i][j], BitDepth )
             target = CflRef[1][i][j]
             for (i0 = 0; i0 < 3; i0++) {
                 for (i1 = i0; i1 < 3; i1++) {
                      ata[i0][i1] += v[i0] * v[i1]
                 }
                 b[i0] += v[i0] * target
             }
             count++
         }
     }
 }


where cfl_ref_luma_avail (which decides if a reference sample is available) is specified as:

 cfl_ref_luma_avail(i, j, w, h) {
     return (i < CflAbove || j < CflLeft + w) &&
            (i < CflAbove + h || j < CflLeft)
 }


The array newParams is initialized as follows:

 for( i = 0; i < 2; i++ ) {
     newParams[ i ] = 0
 }
 newParams[ 2 ] = 1 << MHCCP_BITS


If count is equal to 0, the output of the process is the array newParams and the process immediately
terminates.

Otherwise (count is greater than 0), the ata and b are normalized as follows:

 matrixShift = MHCCP_BITS + 6 - 2 * BitDepth - CeilLog2(count)
 if (matrixShift > 0) {
     leftShift = matrixShift
     rightShift = 0
 } else {
     leftShift = 0
     rightShift = -matrixShift
 }
 for (i0 = 0; i0 < 3; i0++) {
     for (i1 = i0; i1 < 3; i1++) {
          ata[i0][i1] = (ata[i0][i1] << leftShift) >> rightShift



AV2 Specification                                                                              Page 513 of 1169
          }
          b[i0] = (b[i0] << leftShift) >> rightShift
     }


    The Gaussian elimination process specified in § 7.13.8 Gaussian elimination process is invoked with ata
    and b as inputs, and the output is assigned to newParams.

    The output of this process is the array newParams.

```

<a id="s-7-13-8"></a>

#### § 7.13.8 Gaussian elimination process

```text
§   7.13.8. Gaussian elimination process

    The inputs to this process are:

      • a 3x3 array ata,
      • a length 3 array b.

    The output of this process is the array params.

    This process solves a matrix equation via Gaussian elimination (without pivoting) as follows:

     for (i = 0; i < 3; i++) {
         for (j = 0; j < 3; j++) {
             c[i][j] = j >= i ? ata[i][j] : ata[j][i];
         }
         c[i][i] += 2 << (BitDepth - 8)
         c[i][3] = b[i]
     }
     for ( i = 0; i < 3 ; i++) {
         diag = Max(1, Abs(c[i][i]))
         (scale, shift) = get_division_scale_shift( diag )
         for ( j = i + 1; j < 4; j++) {
             c[i][j] = mul_fixed32_adapt( c[i][j], scale, shift )
         }
         for ( j = i + 1; j < 3; j++) {
             scaleFactor = c[j][i];
             for (k = i + 1; k < 4; k++) {
                 c[j][k] -= mul_fixed32_adapt(scaleFactor, c[i][k], MHCCP_BITS)
             }
         }
     }
     for( i = 0; i < 2; i++ ) {
         params[ i ] = 0
     }
     params[ 2 ] = c[ 2 ][ 3 ]
     for( i = 2 ; i >= 0 ; i--) {
         params[ i ] = c[ i ][ 3 ]
         for( j = i + 1 ; j < 3; j++ ) {
             params[ i ] -= mul_fixed32_adapt(c[ i ][ j ], params[ j ], MHCCP_BITS)
         }
     }


    where the function get_division_scale_shift (which returns a scale and shift that can be used to
    approximate division by the input) is defined as:

     get_division_scale_shift( denom ) {
         shift = FloorLog2(denom)
         normDiff = Clip3( 1, 32767,
             Round2(denom << DIV_PREC_BITS, shift) ) & ((1 << DIV_PREC_BITS) - 1)
         index = normDiff >> (DIV_PREC_BITS - DIV_SLOT_BITS)
         normDiff2 = normDiff - Division_Pow2_O[index]
         scale = ((Division_Pow2_W[index] *




    AV2 Specification                                                                               Page 514 of 1169
                    ((normDiff2*normDiff2) >> DIV_PREC_BITS)) >> DIV_PREC_BITS_POW2) -
                  (normDiff2 >> 1) + Division_Pow2_B[index]
          scale = scale << (MHCCP_BITS - DIV_PREC_BITS)
          return (scale, shift)
     }


    where the constant tables Division_Pow2_O, Division_Pow2_B, and Division_Pow2_W are defined as:

     Division_Pow2_W[DIV_PREC_BITS_POW2] = {
         214, 153, 113, 86, 67, 53, 43, 35
     }

     Division_Pow2_O[DIV_PREC_BITS_POW2] = {
         4822, 5952, 6624, 6792, 6408, 5424, 3792, 1466
     }

     Division_Pow2_B[DIV_PREC_BITS_POW2] = {
         12784, 12054, 11670, 11583, 11764, 12195, 12870, 13782
     }


```

<a id="s-7-14"></a>

### § 7.14 Reconstruction and dequantization

```text
§   7.14. Reconstruction and dequantization
```

<a id="s-7-14-1"></a>

#### § 7.14.1 General

```text
§   7.14.1. General

    This section details the process of reconstructing a block of coefficients using dequantization and inverse
    transforms.

```

<a id="s-7-14-2"></a>

#### § 7.14.2 Dequantization functions

```text
§   7.14.2. Dequantization functions

    This section defines the functions get_dc_quant and get_ac_quant that are needed by the dequantization
    process.

    The quantization parameters are derived from lookup tables.

    The function qlookup( q ) is specified as:

     qlookup( q ) {
         if (q < 25) {
             return Ac_Qlookup[q]
         } else {
             return Ac_Qlookup[((q - 1) % 24) + 1] << ((q - 1) / 24)
         }
     }


    where Ac_Qlookup is defined as follows:

     Ac_Qlookup[25] = {
         64,    40,     41,    43,    44,     45,   47,    48,     49,   51,    52,
         54,    55,     57,    59,    60,     62,   64,    66,     68,   70,    72,
         74,    76,     78
     }


    The function get_q( qindex, delta ) is specified as:

     get_q( qindex, delta ) {
         if ((qindex == 0) && (delta <= 0)) {
             return Ac_Qlookup[0]
         }



    AV2 Specification                                                                            Page 515 of 1169
          qClamped = Clip3(1, MaxQ, qindex + delta)
          return qlookup(qClamped)
     }


    The function get_qindex( ignoreDeltaQ, segmentId ) returns the quantizer index for the current block and
    is specified by the following:

      • If seg_feature_active_idx( segmentId, SEG_LVL_ALT_Q ) is equal to 1, the following ordered steps
        apply:

          1. Set the variable data equal to FeatureData[ segmentId ][ SEG_LVL_ALT_Q ].
          2. Set qindex equal to base_q_idx + data.
          3. If ignoreDeltaQ is equal to 0 and delta_q_present is equal to 1, set qindex equal to CurrentQIndex
             + data.
          4. Return Clip3( 0, MaxQ, qindex ).
      • Otherwise, if ignoreDeltaQ is equal to 0 and delta_q_present is equal to 1, return CurrentQIndex.
      • Otherwise, return base_q_idx.


      NOTE: When using both delta quantization and lossless segments, care should be taken that
      get_qindex returns 0 for the lossless segments. One approach is to set FeatureData[ segmentId ]
      [ SEG_LVL_ALT_Q ] to -255 for the lossless segments.


    The function get_dc_quant( plane ) returns the quantizer value for the dc coefficient for a particular plane
    and is derived as follows:

      • If plane is equal to 0, return get_q( get_qindex( 0, segment_id ), DeltaQYDc + BaseYDcDeltaQ ).
      • Otherwise, if plane is equal to 1, return get_q( get_qindex( 0, segment_id ), DeltaQUDc +
        BaseUVDcDeltaQ ).
      • Otherwise (plane is equal to 2), return get_q( get_qindex( 0, segment_id ), DeltaQVDc +
        BaseUVDcDeltaQ ).

    The function get_ac_quant( plane ) returns the quantizer value for the ac coefficient for a particular plane
    and is derived as follows:

      • If plane is equal to 0, return get_q( get_qindex( 0, segment_id ), 0 ).
      • Otherwise, if plane is equal to 1, return get_q( get_qindex( 0, segment_id ), DeltaQUAc +
        BaseUVAcDeltaQ ).
      • Otherwise (plane is equal to 2), return get_q( get_qindex( 0, segment_id ), DeltaQVAc +
        BaseUVAcDeltaQ ).

```

<a id="s-7-14-3"></a>

#### § 7.14.3 Reconstruct process

```text
§   7.14.3. Reconstruct process

    The reconstruct process is invoked to perform dequantization, inverse transform and reconstruction. This
    process is triggered at a point defined by a function call to reconstruct in the transform block syntax
    table described in § 5.20.7.24 Transform block syntax.




    AV2 Specification                                                                             Page 516 of 1169
    The inputs to this process are:

      • a variable plane specifying which plane is being reconstructed,
      • variables x and y specifying the location of the top left sample in the CurrFrame[ plane ] array of the
        current transform block,
      • a variable txSz specifying the size of the transform block.

    The outputs of this process are reconstructed samples in the current frame CurrFrame.

    The variable log2W (specifying the base 2 logarithm of the width of the transform block) is set equal to
    Tx_Width_Log2[ txSz ].

    The variable log2H (specifying the base 2 logarithm of the height of the transform block) is set equal to
    Tx_Height_Log2[ txSz ].

    The variable w (specifying the width of the transform block) is set equal to 1 << log2W.

    The variable h (specifying the height of the transform block) is set equal to 1 << log2H.

    The following ordered steps apply:

     1. If plane is equal to 0 and sec_tx_type is not equal to 0, the secondary transform process as specified
        in § 7.15.3 Secondary transform process is invoked with the variable txSz as input. This modifies the
        values in Dequant.
     2. The 2D inverse transform block process as specified in § 7.15.4 2D inverse transform process is
        invoked with the variables plane and txSz as inputs. The inverse transform outputs are stored in the
        Residual buffer.
     3. For i = 0..(h-1), for j = 0..(w-1), CurrFrame[ plane ][ y + i ][ x + j ] is set equal to
        Clip1( CurrFrame[ plane ][ y + i ][ x + j ] + Residual[ i ][ j ] ).

    If Lossless is equal to 1, it is a requirement of bitstream conformance that the values written into the
    Residual array in step 2 are representable by a signed integer with 1 + BitDepth bits.


      NOTE: This requirement applies to the final values written to the Residual array, i.e., after any
      DPCM adjustment.

```

<a id="s-7-14-4"></a>

#### § 7.14.4 Dequantization process

```text
§   7.14.4. Dequantization process

    The dequantization process is triggered at a point defined by a function call to dequant in the transform
    block syntax table described in § 5.20.7.24 Transform block syntax.

    The inputs to this process are:

      • a variable plane specifying which plane is being reconstructed,
      • a variable txSz specifying the size of the transform block.

    The process dequantizes coefficients from the Quant array and places the results in the Dequant array.

    The variable tw is set equal to Min( 32, Tx_Width[ txSz ] ).

    The variable th is set equal to Min( 32, Tx_Height[ txSz ] ).


    AV2 Specification                                                                              Page 517 of 1169
The variables dqDenom, shift, useQm, segLvl, useUserQm, and useFsc are derived as follows:

 pels = Tx_Width[ txSz ] * Tx_Height[ txSz ]
 shift = (pels > 256) + (pels > 1024)
 useFsc = enable_fsc && PlaneTxType == IDTX && plane == 0 &&
          (fsc_mode || is_inter)
 if ( allow_tcq && plane == 0 && !Lossless &&
      get_tx_class(PlaneTxType) == TX_CLASS_2D && !useFsc ) {
     shift += 1
 }
 dqDenom = 1 << shift

 if ( tw > 8 || th > 8 ) {
     if ( plane == 0 ) {
          segLvl = qm_y[ 0 ]
     } else if ( plane == 1 ) {
          segLvl = qm_u[ 0 ]
     } else {
          segLvl = qm_v[ 0 ]
     }
 } else {
     segLvl = SegQMLevel[ plane ][ segment_id ]
 }
 useQm = using_qmatrix == 1 && PlaneTxType < IDTX && segLvl < NUM_CUSTOM_QMS
 useUserQm = useQm && tw <= 8 && th <= 8 && QmDataPresent[ segLvl ]


For i = 0..(th-1), for j = 0..(tw-1), the following ordered steps apply:

 1. The variable q is derived as follows:

       ◦ If i is equal to 0 and j is equal to 0, the variable q is set equal to get_dc_quant( plane ).
       ◦ Otherwise (i, j or both are not equal to 0), the variable q is set equal to get_ac_quant( plane ).
 2. The variable q2 is derived as follows:

       ◦ If useQm is equal to 1, q2 is set as follows:

           if ( useUserQm ) {
               if ( tw < th ) {
                    m = UserQm[ segLvl ][ 2 ][ plane ][ i ][ j ]
               } else if ( tw > th ) {
                    m = UserQm[ segLvl ][ 1 ][ plane ][ i ][ j ]
               } else {
                    qi = i * 8 / th
                    qj = j * 8 / tw
                    m = UserQm[ segLvl ][ 0 ][ plane ][ qi ][ qj ]
               }
           } else {
               m = Quantizer_Matrix[ segLvl ][ plane > 0 ]
                                    [ Qm_Offset[ txSz ] + i * tw + j ]
           }
           q2 = Round2( q * m, 5 )


       ◦ Otherwise, q2 is set equal to q.
 3. The variable qc is set equal to Quant[ i * tw + j ].
 4. The variable sign is set equal to ( qc < 0 ) ? -1 : 1.
 5. The variable dqHigh is set equal to Abs(qc) * q2.
 6. The variable dq is set equal to Round2(dqHigh & 0xFFFFFF, QUANT_TABLE_BITS).
 7. The variable dq2 is set equal to sign * ( dq / dqDenom ).


AV2 Specification                                                                                  Page 518 of 1169
     8. Dequant[ i ][ j ] is set equal to Clip3( - ( 1 << ( 7 + BitDepth ) ), ( 1 << ( 7 + BitDepth ) ) - 1, dq2 ).

```

<a id="s-7-14-5"></a>

#### § 7.14.5 Save dequant process

```text
§   7.14.5. Save dequant process

    The save dequant process is triggered at a point defined by a function call to save_dequant in the
    transform block syntax table described in § 5.20.7.24 Transform block syntax.

    The inputs to this process are:

      • a variable plane specifying which plane is being reconstructed,
      • a variable txSz specifying the size of the transform block.

    The process saves the dequantized coefficients as follows:

     tw = Min(32,Tx_Width[ txSz ])
     th = Min(32,Tx_Height[ txSz ])
     for( i = 0; i < th; i++ ) {
         for( j = 0; j < tw; j++ ) {
             SaveDequant[ plane ][ i ][ j ] = Dequant[ i ][ j ]
         }
     }


```

<a id="s-7-14-6"></a>

#### § 7.14.6 Get dequant process

```text
§   7.14.6. Get dequant process

    The get dequant process is triggered at a point defined by a function call to get_dequant in the transform
    block syntax table described in § 5.20.7.24 Transform block syntax.

    The inputs to this process are:

      • a variable plane specifying which plane is being reconstructed,
      • a variable txSz specifying the size of the transform block,
      • a variable cctxType specifying the type of cross component transform.

    The process computes the dequantized coefficients as follows:


     tw = Min( 32, Tx_Width[ txSz ] )
     th = Min( 32, Tx_Height[ txSz ] )
     for( i = 0; i < th; i++ ) {
         for( j = 0; j < tw; j++ ) {
             if (cctxType == CCTX_NONE) {
                 v = SaveDequant[ plane ][ i ][ j ]
             } else {
                 angle = cctxType - 1
                 if (plane == 1) {
                      cU = Cctx_Mtx[ angle ][ 0 ]
                      cV = -Cctx_Mtx[ angle ][ 1 ]
                 } else {
                      cU = Cctx_Mtx[ angle ][ 1 ]
                      cV = Cctx_Mtx[ angle ][ 0 ]
                 }
                 u = SaveDequant[ 1 ][ i ][ j ]
                 v = SaveDequant[ 2 ][ i ][ j ]
                 v = Round2Signed(u * cU + v * cV, CCTX_PREC_BITS)
                 v = Clip3( -(1 << (BitDepth + 7)), (1 << (BitDepth + 7)) - 1, v)
             }




    AV2 Specification                                                                                  Page 519 of 1169
               Dequant[i][j] = v
          }
     }


    where the constant table Cctx_Mtx (which stores the cosine and sine of the rotation angle shifted up by
    CCTX_PREC_BITS bits) is defined as follows:

     Cctx_Mtx[CCTX_TYPES - 1][2] = {
         { 181, 181 },
         { 222, 128 },
         { 128, 222 },
         { 181, -181 },
         { 222, -128 },
         { 128, -222 }
     }


```

<a id="s-7-15"></a>

### § 7.15 Inverse transform process

```text
§   7.15. Inverse transform process
```

<a id="s-7-15-1"></a>

#### § 7.15.1 General

```text
§   7.15.1. General

    This section details the inverse transforms used during the reconstruction processes detailed in § 7.14
    Reconstruction and dequantization.

```

<a id="s-7-15-2"></a>

#### § 7.15.2 1D transforms

```text
§   7.15.2. 1D transforms

```

<a id="s-7-15-2-1"></a>

##### § 7.15.2.1 1d inverse transform process

```text
§   7.15.2.1. 1d inverse transform process

    The inputs to this process are:

      • an array src with the input coefficients,
      • a variable txType1D that specifies the type of transform to apply,
      • a variable sz specifying the length of the src array,
      • a variable shift specifying the amount of down-shifting to apply,
      • a variable colTx specifying whether the current 1-D inverse transform is applied to the columns of the
        input coefficients.

    The process transforms the input coefficients using a matrix multiplication as follows for i = 0..(sz-1):

     s = 0
     if (sz == 4) {
         for (j = 0; j < 4; j++) {
             if (txType1D == DCT) {
                 s += Dct_Kernel4[ j ][ i ] * src[ j ]
             } else if (txType1D == ADST) {
                 s += Adst_Kernel4[ j ][ i ] * src[ j ]
             } else {
                 s += Fdst_Kernel4[ j ][ i ] * src[ j ]
             }
         }
     } else if (sz == 8) {
         for (j = 0; j < 8; j++) {
             if (txType1D == DCT) {
                 s += Dct_Kernel8[ j ][ i ] * src[ j ]
             } else if (txType1D == ADST) {
                 s += Adst_Kernel8[ j ][ i ] * src[ j ]
             } else if (txType1D == FDST) {
                 s += Fdst_Kernel8[ j ][ i ] * src[ j ]



    AV2 Specification                                                                              Page 520 of 1169
               } else if (txType1D == DDTX) {
                   s += Ddtx_Kernel8[ j ][ i ] * src[ j ]
               } else {
                   s += Ddtx_Kernel8[ j ][ 7 - i ] * src[ j ]
               }
         }
     } else if (sz == 16) {
         for (j = 0; j < 16; j++) {
              if (txType1D == DCT) {
                  s += Dct_Kernel16[ j ][ i ] * src[ j ]
              } else if (txType1D == ADST) {
                  s += Adst_Kernel16[ j ][ i ] * src[ j ]
              } else if (txType1D == FDST) {
                  s += Fdst_Kernel16[ j ][ i ] * src[ j ]
              } else if (txType1D == DDTX) {
                  s += Ddtx_Kernel16[ j ][ i ] * src[ j ]
              } else {
                  s += Ddtx_Kernel16[ j ][ 15 - i ] * src[ j ]
              }
         }
     } else {
         for (j = 0; j < 32; j++) {
              s += Dct_Kernel32[ j ][ i ] * src[ j ]
         }
     }
     result[i] = Clip3( -( 1 << ( BitDepth + ( colTx ? 0 : 7 ) ) ),
                          ( 1 << ( BitDepth + ( colTx ? 0 : 7 ) ) ) - 1,
                          Round2(s, shift) )


    The output of the process is the array result.

```

<a id="s-7-15-2-2"></a>

##### § 7.15.2.2 Inverse Walsh-Hadamard transform process

```text
§   7.15.2.2. Inverse Walsh-Hadamard transform process

    The inputs to this process are:

      • an array src (of length 4) with the input coefficients,
      • a variable shift that specifies the amount of pre-scaling.

    This process does an in-place transform of the array src as follows:

     a = src[ 0 ] >> shift
     c = src[ 1 ] >> shift
     d = src[ 2 ] >> shift
     b = src[ 3 ] >> shift
     a += c
     d -= b
     e = (a - d) >> 1
     b = e - b
     c = e - c
     a -= b
     d += c
     result[ 0 ] = a
     result[ 1 ] = b
     result[ 2 ] = c
     result[ 3 ] = d


    The output of this process is the array result.

```

<a id="s-7-15-2-3"></a>

##### § 7.15.2.3 Inverse identity transform process

```text
§   7.15.2.3. Inverse identity transform process

    The inputs to this process are:

      • an array src with the input coefficients,


    AV2 Specification                                                      Page 521 of 1169
      • a variable scale that specifies the amount of scaling to apply,
      • a variable sz specifying the length of the src array,
      • a variable shift specifying the amount of down-shifting to apply,
      • a variable colTx specifying whether the current 1-D inverse transform is applied to the columns of the
        input coefficients.

    The process does a scaling of the array src by the following calculation for i = 0..(sz-1):

     result[i] = Clip3( - ( 1 << ( BitDepth + ( colTx ? 0 : 7 ) ) ),
                          ( 1 << ( BitDepth + ( colTx ? 0 : 7 ) ) ) - 1,
                          Round2(src[i] * scale, shift) )


    The output of the process is the array result.


      NOTE: This section defines the inverse identity transform used for lossy segments. For lossless
      segments, the inverse identity transform is specially handled using a bit-shift operation as shown in
      § 7.15.4 2D inverse transform process.

```

<a id="s-7-15-3"></a>

#### § 7.15.3 Secondary transform process

```text
§   7.15.3. Secondary transform process

    This process performs a matrix based transform for coefficients stored in the 2D array Dequant. The
    output is placed back into the array Dequant.

    The input to this process is a variable txSz that specifies the transform size.

    The variables w, h, bwl, large, and n (related to the size of the transform block) are derived as follows:

     w = Min(32, Tx_Width[ txSz ])
     h = Min(32, Tx_Height[ txSz ])
     bwl = Min(5, Tx_Width_Log2[ txSz ])
     large = w >= 8 && h >= 8
     if ( !large ) {
         n = IST_4X4_HEIGHT
     } else if ( txSz == TX_8X8 || PlaneTxType == ADST_ADST ) {
         n = IST_8X8_HEIGHT_RED
     } else {
         n = IST_8X8_HEIGHT
     }


    The variables kernel and transpose (describing the type of transform to apply) are derived as follows:

     mode = YMode
     if ( is_directional_mode( mode ) ) {
         pAngle = Mode_To_Angle[ mode ] + AngleDeltaY * ANGLE_STEP +
                   Mrl_Index_To_Delta[ MrlIndex ]
         (mode,unusedAngle) = wide_angle_mapping( mode, Tx_Width[txSz],
                                                  Tx_Height[txSz], pAngle )
     }
     if ( is_inter ) {
         kernel = 0
     } else if ( PlaneTxType == ADST_ADST && Tx_Width[ txSz ] >= 8 &&
                  Tx_Height[ txSz ] >= 8 ) {
         kernel = Inv_Most_Probable_Stx_Mapping_Adst[ mode ][ most_probable_stx_set ]
     } else {
         kernel = Inv_Most_Probable_Stx_Mapping[ mode ][ most_probable_stx_set ]
     }



    AV2 Specification                                                                              Page 522 of 1169
 if (PlaneTxType == ADST_ADST) {
     kernel += 7
 }
 transpose = (mode == H_PRED || mode == D157_PRED ||
              mode == D67_PRED || mode == SMOOTH_H_PRED)


where the constant tables Inv_Most_Probable_Stx_Mapping and Inv_Most_Probable_Stx_Mapping_Adst
are defined as:

 Inv_Most_Probable_Stx_Mapping[ INTRA_MODES - 1 ][ IST_DIR_SIZE ] = {
     { 6, 1, 0, 5, 4, 3, 2 },
     { 1, 6, 0, 4, 2, 5, 3 },
     { 1, 6, 0, 4, 2, 5, 3 },
     { 2, 6, 0, 5, 1, 4, 3 },
     { 3, 4, 6, 1, 0, 2, 5 },
     { 4, 1, 3, 6, 0, 5, 2 },
     { 4, 1, 3, 6, 0, 5, 2 },
     { 5, 0, 6, 2, 1, 4, 3 },
     { 5, 0, 6, 2, 1, 4, 3 },
     { 6, 1, 0, 5, 4, 3, 2 },
     { 1, 6, 0, 4, 2, 5, 3 },
     { 1, 6, 0, 4, 2, 5, 3 }
 }

 Inv_Most_Probable_Stx_Mapping_Adst[INTRA_MODES - 1]
                                   [IST_REDUCE_SET_SIZE_ADST_ADST] = {
     { 3, 1, 0, 2 },
     { 1, 3, 0, 2 },
     { 1, 3, 0, 2 },
     { 1, 3, 0, 2 },
     { 0, 2, 3, 1 },
     { 2, 1, 0, 3 },
     { 2, 1, 0, 3 },
     { 1, 0, 3, 2 },
     { 1, 0, 3, 2 },
     { 3, 1, 0, 2 },
     { 1, 3, 0, 2 },
     { 1, 3, 0, 2 }
 }


The coefficients are placed in scan order into the array coefs as follows:

 scanIn = get_scan(txSz, TX_CLASS_2D)
 for( i = 0 ; i < n ; i++) {
     pos = scanIn[ i ]
     x = pos & (w - 1)
     y = pos >> bwl
     coefs[ i ] = Dequant[ y ][ x ]
     Dequant[ y ][ x ] = 0
 }


The coefficients are transformed by a matrix multiplication and placed back into Dequant as follows:

 scanBwl = large ? 3 : 2
 scanW = 1 << scanBwl
 scanOut = large ? Stx_Scan_Order_8x8 : Stx_Scan_Order_4x4
 if ( large ) {
     scanMap = Stx_Scan_Map[ kernel ][ sec_tx_type - 1]
 }
 n2 = large ? IST_8X8_WIDTH : IST_4X4_WIDTH
 for( i = 0; i < n2; i++ ) {
     t = 0
     for( j = 0 ; j < n ; j++ ) {
         t += coefs[ j ] *



AV2 Specification                                                                          Page 523 of 1169
                        (large ? Ist_8x8_Kernel[ kernel ][ sec_tx_type-1 ][ j ][ i ] :
                                 Ist_4x4_Kernel[ kernel ][ sec_tx_type-1 ][ j ][ i ] )
          }
          v = Round2Signed( t, 7 )
          v = Clip3( -(1 << (BitDepth + 7)), (1 << (BitDepth + 7)) - 1, v)
          if ( large ) {
              pos = scanOut[scanMap[i]]
          } else {
              pos = scanOut[i]
          }
          x = pos & (scanW - 1)
          y = pos >> scanBwl
          if (transpose) {
              Dequant[x][y] = v
          } else {
              Dequant[y][x] = v
          }
     }


    where constant tables Stx_Scan_Order_4x4 and Stx_Scan_Order_8x8 are defined as:

     Stx_Scan_Order_4x4[IST_4X4_WIDTH] = {
         0, 1, 4, 8, 5, 2, 3, 6, 9, 12, 13, 10, 7, 11, 14, 15
     }
     Stx_Scan_Order_8x8[64] = {
         0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5,
         12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28,
         35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51,
         58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63
     }



      NOTE:       The scanOut tables are not the inverse of the scanIn tables.

```

<a id="s-7-15-4"></a>

#### § 7.15.4 2D inverse transform process

```text
§   7.15.4. 2D inverse transform process

    This process performs a 2D inverse transform for an array of coefficients stored in the 2D array Dequant.
    The output is placed in the 2D array Residual.

    The inputs to this process are:

      • a variable plane specifying whether the process is filtering Y, U, or V samples,
      • a variable txSz that specifies the transform size.

    Set the variable adjTxSz equal to Adjusted_Tx_Size[ txSz ].

    Set the variable log2W equal to Tx_Width_Log2[ txSz ].

    Set the variable log2H equal to Tx_Height_Log2[ txSz ].

    Set the variable adjLog2W equal to Tx_Width_Log2[ adjTxSz ].

    Set the variable adjLog2H equal to Tx_Height_Log2[ adjTxSz ].

    Set the variable w equal to 1 << adjLog2W.

    Set the variable h equal to 1 << adjLog2H.

    The variable pels is set equal to w * h.



    AV2 Specification                                                                          Page 524 of 1169
    The variable shift is set equal to (pels > 256) + (pels > 1024).

    If Lossless is equal to 1 and PlaneTxType is equal to IDTX, set Residual[ i ][ j ] equal to Dequant[ i ][ j ]
    >> (3 - shift) for i = 0..h-1, for j = 0..w-1.


    Otherwise, the 2d matrix transform process specified in § 7.15.4.1 2D matrix transform process is invoked
    with adjTxSz and txSz as inputs.

    The variable useDpcm is set equal to (plane == 0 ? use_dpcm_y : use_dpcm_uv).

    The variable mode is set equal to (plane == 0 ? YMode : UVMode).

    If useDpcm is equal to 1 a cumulative sum is applied to Residual as follows:

     if ( mode == V_PRED ) {
         for (j = 0; j < w; j++) {
              for (i = 1; i < h; i++) {
                  Residual[ i ][ j ] += Residual[ i - 1 ][ j ]
              }
         }
     } else {
         for (j = 1; j < w; j++) {
              for (i = 0; i < h; i++) {
                  Residual[ i ][ j ] += Residual[ i ][ j - 1 ]
              }
         }
     }


    If adjTxSz is not equal to txSz, the residual is expanded by sample duplication as follows:

     w2 = Tx_Width[ txSz ]
     if ( w != w2 ) {
         for( i = 0; i < h; i++ ) {
             for( j = w - 1; j >= 0; j-- ) {
                 r = Residual[ i ][ j ]
                 Residual[ i ][ 2 * j ] = r
                 Residual[ i ][ 2 * j + 1 ] = r
             }
         }
     }
     h2 = Tx_Height[ txSz ]
     if ( h != h2 ) {
         for( i = h - 1; i >= 0; i-- ) {
             for( j = 0; j < w2; j++ ) {
                 r = Residual[ i ][ j ]
                 Residual[ 2 * i ][ j ] = r
                 Residual[ 2 * i + 1 ][ j ] = r
             }
         }
     }


```

<a id="s-7-15-4-1"></a>

##### § 7.15.4.1 2D matrix transform process

```text
§   7.15.4.1. 2D matrix transform process

    This process performs a 2D matrix transform for an array of coefficients stored in the 2D array Dequant.
    The output is placed in the 2D array Residual.

    The inputs to this process are:

      • a variable adjTxSz that specifies the adjusted transform size,
      • a variable txSz that specifies the transform size.


    AV2 Specification                                                                                Page 525 of 1169
Set the variable log2W equal to Tx_Width_Log2[ txSz ].

Set the variable log2H equal to Tx_Height_Log2[ txSz ].

Set the variable adjLog2W equal to Tx_Width_Log2[ adjTxSz ].

Set the variable adjLog2H equal to Tx_Height_Log2[ adjTxSz ].

Set the variable w equal to 1 << adjLog2W.

Set the variable h equal to 1 << adjLog2H.

The constant table Transform_Shift is specified as:

 Transform_Shift[ TX_SIZES_ALL ][ 2 ] = {
   { 7, 10 },
   { 7, 11 },
   { 6, 13 },
   { 6, 13 },
   { 6, 13 },
   { 7, 10 },
   { 7, 10 },
   { 7, 11 },
   { 7, 11 },
   { 6, 12 },
   { 6, 12 },
   { 6, 12 },
   { 6, 12 },
   { 6, 12 },
   { 6, 12 },
   { 6, 13 },
   { 6, 13 },
   { 6, 13 },
   { 6, 13 },
   { 7, 11 },
   { 7, 11 },
   { 6, 12 },
   { 6, 12 },
   { 6, 13 },
   { 6, 13 },
 }


Set the variable rowShift equal to Transform_Shift[ txSz ][ 0 ].

Set the variable colShift equal to Transform_Shift[ txSz ][ 1 ].

The function get_transform_1d_type is specified as:

 get_transform_1d_type( dir, sz ) {
     useDdt = enable_inter_ddt && !use_intrabc && is_inter
     t = Transform_1d_Type[ PlaneTxType ][ dir ]
     if ( useDdt && (t == ADST || t == FDST) && sz != 4 ) {
         return (t == ADST) ? DDTX : FDDT
     }
     return t
 }


The 1d transform types returned from this function are specified as specified in Table 7.1:

                             Table 7.1: 1D transform type values and names



AV2 Specification                                                                             Page 526 of 1169
                    Value of 1d transform type                           Name of 1d transform type

 0                                                        DCT

 1                                                        IDT

 2                                                        ADST

 3                                                        FDST

 4                                                        DDTX

 5                                                        FDDT


where the constant table Transform_1d_Type is specified as:

 Transform_1d_Type[ TX_TYPES ][ 2 ] = {
     { DCT, DCT },
     { DCT, ADST },
     { ADST, DCT },
     { ADST, ADST },
     { DCT, FDST },
     { FDST, DCT },
     { FDST, FDST },
     { FDST, ADST },
     { ADST, FDST },
     { IDT, IDT },
     { IDT, DCT },
     { DCT, IDT },
     { IDT, ADST },
     { ADST, IDT },
     { IDT, FDST },
     { FDST, IDT }
 }


Set the variable rowType equal to get_transform_1d_type( 0, w ).

Set the variable colType equal to get_transform_1d_type( 1, h ).

txRowIn[ j ] is set equal to 0 for j = 0..w-1.

txColIn[ i ] is set equal to 0 for i = 0..h-1.

intermediate[ i ][ j ] is set equal to 0 for i = 0..Min(h,32)-1, for j = 0..w-1.

The following applies for i = 0..(Min(h,32)-1):

  • txRowIn[ j ] is derived as follows for j = 0..(Min(w,32)-1):

       ◦ If Abs( log2W - log2H ) is odd, txRowIn[ j ] is set equal to Round2( Dequant[ i ][ j ] * 2896, 12 ).
       ◦ Otherwise, txRowIn[ j ] is set equal to Dequant[ i ][ j ].
  • The row transform is applied as follows:

       ◦ If Lossless is equal to 1, the Inverse WHT process as specified in § 7.15.2.2 Inverse Walsh-
         Hadamard transform process is invoked with txRowIn and the input variable shift equal to 3 as
         inputs, and the output is assigned to txRowOut.
       ◦ Otherwise, if rowType is equal to IDT, the inverse identity transform process as specified in
         § 7.15.2.3 Inverse identity transform process is invoked with txRowIn, get_identity_scale( log2W ),
         w, rowShift, and 0 as inputs, and the output is assigned to txRowOut.



AV2 Specification                                                                                    Page 527 of 1169
           ◦ Otherwise, the 1d matrix transform process specified in § 7.15.2.1 1d inverse transform process is
             invoked with txRowIn, rowType, w, rowShift, and 0 as inputs, and the output is assigned to
             txRowOut.
      • Set intermediate[ i ][ j ] equal to txRowOut[ j ] for j = 0..(w-1).

    The following applies for j = 0..(w-1):

      • Set txColIn[ i ] equal to intermediate[ i ][ j ] for i = 0..(Min(h,32)-1).
      • The column transform is applied as follows:

           ◦ If Lossless is equal to 1, the Inverse WHT process as specified in § 7.15.2.2 Inverse Walsh-
             Hadamard transform process is invoked with txColIn and the input variable shift equal to 0 as
             inputs, and the output is assigned to txColOut.
           ◦ Otherwise, if colType is equal to IDT, the inverse identity transform process as specified in
             § 7.15.2.3 Inverse identity transform process is invoked with txColIn, get_identity_scale( log2H ),
             h, colShift, and 1 as inputs, and the output is assigned to txColOut.
           ◦ Otherwise, the 1d matrix transform process specified in § 7.15.2.1 1d inverse transform process is
             invoked with txColIn, colType, h, colShift, and 1 as inputs, and the output is assigned to txColOut.
      • Residual[ i ][ j ] is set equal to txColOut[ i ] for i = 0..(h-1).

    where the function get_identity_scale is specified as:

     get_identity_scale( log2Sz ) {
         if (log2Sz == 2) {
             return 128
         } else if (log2Sz == 3) {
             return 181
         } else if (log2Sz == 4) {
             return 256
         }
         return 362
     }


```

<a id="s-7-16"></a>

### § 7.16 Deblocking filter for TIP process

```text
§   7.16. Deblocking filter for TIP process
    Input to this process is the array CurrFrame of reconstructed samples.

    Output from this process is a modified array CurrFrame containing deblocked samples.

    The filtering is applied as follows:

     tipSize = ( enable_tip_refinemv &&
                 TipInterpFilter == EIGHTTAP_SHARP ) ? BLOCK_8X8 : BLOCK_16X16
     for ( plane = 0; plane < NumPlanes; plane++ ) {
         baseFilterLevel = base_q_idx
         if (plane == 1) {
             baseFilterLevel += DeltaQUAc + BaseUVAcDeltaQ
         } else if (plane == 2) {
             baseFilterLevel += DeltaQVAc + BaseUVAcDeltaQ
         }
         qThr = Round2(get_q(baseFilterLevel,0),QUANT_TABLE_BITS) >> 6
         qInd = Clip3(0, MAX_SIDE_TABLE - 1, baseFilterLevel - 24 * (BitDepth - 8))
         side = Max( Side_Thresholds[qInd] + (1 << (12 - BitDepth)),
                     0 ) >> ( 13 - BitDepth)
         subX = plane == 0 ? 0 : SubsamplingX




    AV2 Specification                                                                              Page 528 of 1169
      subY = plane == 0 ? 0 : SubsamplingY
      sw = Block_Width[tipSize] >> subX
      sh = Block_Height[tipSize] >> subY
      h = (MiRows * MI_SIZE >> subY)
      w = (MiCols * MI_SIZE >> subX)
      for ( y = 0; y < h; y += 4 ) {
          for ( x = 0; x < w; x += sw ) {
              if ( x > 0 ) {
                  vertTileEdge = is_vert_tile_edge( x, subX )
                  (maxWidthPos, maxWidthNeg) = filter_maximum_width( plane,
                                                                filterSize = sw,
                                                                vertTileEdge)
                  if ( !disable_loopfilters_across_tiles || !vertTileEdge ) {
                      width = filter_choice( x, y, plane, qThr, side, dx=1, dy=0,
                                           maxWidthNeg, maxWidthPos, MI_SIZE)
                      if (width > 0) {
                          for (i = 0; i < 4; i++) {
                              sample_filtering( x, y + i, plane, qThr, dx=1, dy=0,
                                               Min(width,maxWidthNeg),
                                               Min(width,maxWidthPos), 0, 0 )
                          }
                      }
                  }
              }
          }
      }
      for ( x = 0; x < w; x += 4 ) {
          for ( y = 0; y < h; y += sh ) {
              if ( y > 0 ) {
                  horzTileEdge = is_horz_tile_edge( y, subY )
                  if ( !disable_loopfilters_across_tiles || !horzTileEdge ) {
                      horz64Edge = ( (y << subY) % 64 ) == 0
                      (maxWidthPos, maxWidthNeg) = filter_maximum_width( plane,
                                                                   filterSize = sh,
                                                                   horz64Edge)
                      width = filter_choice( x, y, plane, qThr, side, dx=0, dy=1,
                                           maxWidthNeg, maxWidthPos, MI_SIZE)
                      if (width > 0) {
                          for (i = 0; i < MI_SIZE; i++) {
                              sample_filtering( x + i, y, plane, qThr, dx=0, dy=1,
                                               Min(width,maxWidthNeg),
                                               Min(width,maxWidthPos), 0, 0 )
                          }
                      }
                  }
              }
          }
      }
 }


The function call of filter_maximum_width indicates that the filter maximum width process specified in
§ 7.17.3 Filter maximum width process is invoked.

The function call of filter_choice indicates that the filter choice process specified in § 7.17.7.2 Filter
choice process is invoked.

The function call of sample_filtering indicates that the sample filtering process specified in § 7.17.7
Sample filtering process is invoked.

The function is_vert_tile_edge (which determines if the filter crosses a vertical tile edge) is specified as:

 is_vert_tile_edge( x, subX ) {
     lumaX = x << subX
     col = lumaX >> MI_SIZE_LOG2
     for( t = 0; t < TileCols; t++ ) {
         if ( col == MiColStarts[ t ] )



AV2 Specification                                                                                 Page 529 of 1169
                     return 1
          }
          return 0
     }


    The function is_horz_tile_edge (which determines if the filter crosses a horizontal tile edge) is specified
    as:

     is_horz_tile_edge( y, subY ) {
         lumaY = y << subY
         row = lumaY >> MI_SIZE_LOG2
         for( t = 0; t < TileRows; t++ ) {
             if ( row == MiRowStarts[ t ] )
                  return 1
         }
         return 0
     }


```

<a id="s-7-17"></a>

### § 7.17 Deblocking filter process

```text
§   7.17. Deblocking filter process
```

<a id="s-7-17-1"></a>

#### § 7.17.1 General

```text
§   7.17.1. General

    Input to this process is the array CurrFrame of reconstructed samples.

    Output from this process is a modified array CurrFrame containing deblocked samples.

    The purpose of the deblocking filter is to eliminate (or at least reduce) visually objectionable artifacts
    associated with the semi-independence of the coding of super blocks and their constituent sub-blocks.

    The deblocking filter is applied on all vertical boundaries followed by all horizontal boundaries as follows:

     for ( plane = 0; plane < NumPlanes; plane++ ) {
         for ( pass = 0; pass < 2; pass++ ) {
             if (apply_deblocking_filter[plane==0 ? pass : plane + 1]) {
                 rowStep = ( plane == 0 ) ? 1 : ( 1 << SubsamplingY )
                 colStep = ( plane == 0 ) ? 1 : ( 1 << SubsamplingX )
                 for ( row = 0; row < MiRows; row += rowStep )
                     for ( col = 0; col < MiCols; col += colStep )
                         deblocking_filter_edge( plane, pass, row, col )
             }
         }
     }


    When the function deblocking_filter_edge is called, the edge deblocking filter process specified in § 7.17.2
    Edge deblocking filter process is invoked with the variables plane, pass, row, and col as inputs.


      NOTE: The deblocking filter is an integral part of the decoding process, in that the results of
      deblocking filtering are used in the prediction of subsequent frames.


      NOTE: The deblocking filtering is designed so that any order of filtering for the edges will give
      identical results, provided that the vertical boundaries are filtered before the horizontal boundaries.




    AV2 Specification                                                                              Page 530 of 1169
```

<a id="s-7-17-2"></a>

#### § 7.17.2 Edge deblocking filter process

```text
§   7.17.2. Edge deblocking filter process

    The inputs to this process are:

      • a variable plane specifying whether the process is filtering Y, U, or V samples,
      • a variable pass specifying the direction of the edges. pass equal to 0 means the process is filtering
        vertical block boundaries, and pass equal to 1 means the process is filtering horizontal block
        boundaries,
      • variables row and col specifying the location of the edge in units of 4x4 blocks in the luma plane.

    The outputs of this process are modified values in the array CurrFrame.

    The variable sbShift is set equal to Mi_Width_Log2[SbSize].

    The variable sbX (the superblock X position) is set equal to (col >> sbShift).

    The variable sbY (the superblock Y position) is set equal to (row >> sbShift).

    If use_bru is equal to 1 and BruModes[sbY << sbShift][sbX << sbShift] is not equal to BRU_ACTIVE, this
    process terminates immediately.

    The variables subX and subY describing the subsampling of the current plane are derived as follows:

      • If plane is equal to 0, subX and subY are set equal to 0.
      • Otherwise (plane is not equal to 0), subX is set equal to SubsamplingX and subY is set equal to
        SubsamplingY.

    The variables dx and dy are derived as follows:

      • If pass is equal to 0, then dx is set equal to 1, dy is set equal to 0.
      • Otherwise (pass is equal to 1), dy is set equal to 1, dx is set equal to 0.

    dx and dy specify the offset between the samples to be filtered.

    The variable x is set equal to col * MI_SIZE.

    The variable y is set equal to row * MI_SIZE.

    x and y contain the location in luma coordinates.

    The variable sbEdge (equal to 1 if this is a horizontal edge on the 64x64 grid or a vertical tile edge) is
    computed as follows:

     tileVertEdge = (pass == 0 && MiColStartGrid[ row ][ col ] == col)
     tileHorzEdge = (pass == 1 && MiRowStartGrid[ row ][ col ] == row)
     horz64Edge = (pass == 1 && ( y % 64 ) == 0)
     sbEdge = horz64Edge || tileVertEdge


    If disable_loopfilters_across_tiles is equal to 1 and tileVertEdge is equal to 1, then this process
    immediately returns and no filtering is applied to this edge.

    If disable_loopfilters_across_tiles is equal to 1 and tileHorzEdge is equal to 1, then this process
    immediately returns and no filtering is applied to this edge.


    AV2 Specification                                                                               Page 531 of 1169
The variable onScreen is derived as follows:

  • If pass is equal to 0 and x is equal to 0, onScreen is set equal to 0.
  • Otherwise, if pass is equal to 1 and y is equal to 0, onScreen is set equal to 0.
  • Otherwise, onScreen is set equal to 1.

If onScreen is equal to 0, then this process immediately returns and no filtering is applied to this edge.

The variables xP and yP (containing the location in the current plane) are derived as follows:

  • Set xP equal to x >> subX.
  • Set yP equal to y >> subY.

The variables prevRow and prevCol (containing the location of the mode info block on the other side of
the boundary) are derived as follows:

  • Set prevRow equal to row - ( dy << subY ).
  • Set prevCol equal to col - ( dx << subX ).

The variable isSubPuEdge (equal to 1 if the edge is treated as a subblock edge) is computed by
comparing the locations of the subblock as follows:

 subPuColBase = SubPuColBase[ plane > 0 ][ row ][ col ]
 subPuRowBase = SubPuRowBase[ plane > 0 ][ row ][ col ]
 prevSubPuColBase = SubPuColBase[ plane > 0 ][ prevRow ][ prevCol ]
 prevSubPuRowBase = SubPuRowBase[ plane > 0 ][ prevRow ][ prevCol ]
 isSubPuEdge = allow_df_sub_pu && ( subPuColBase != prevSubPuColBase ||
                                    subPuRowBase != prevSubPuRowBase )


Set the variable subPuSize (giving the size of the subblocks used in this block) equal to SubPuSize[ plane
> 0 ][ row ][ col ].

Set the variable currLossless equal to LosslessArray[ ( plane > 0 ) ? ChromaSegmentIds[ row ][ col ] :
SegmentIds[ row ][ col ] ].


Set the variable prevLossless equal to
LosslessArray[ ( plane > 0 ) ? ChromaSegmentIds[ prevRow ][ prevCol ] : SegmentIds[ prevRow ][ prevCol ] ].


Set the variable MiSize equal to MiSizes[ plane > 0 ][ row ][ col ].

Set the variable baseRow equal to MiRowBase[ plane > 0 ][ row ][ col ].

Set the variable baseCol equal to MiColBase[ plane > 0 ][ row ][ col ].

Set the variable baseY equal to (baseRow * MI_SIZE) >> subY.

Set the variable baseX equal to (baseCol * MI_SIZE) >> subX.

Set the variable txSz equal to DeblockingTxSizes[ plane ][ row >> subY ][ col >> subX ].

Set the variable prevSubPuSize equal to SubPuSize[ plane > 0 ][ prevRow ][ prevCol ].

Set the variable prevTxSz equal to DeblockingTxSizes[ plane ][ prevRow >> subY ][ prevCol >> subX ].



AV2 Specification                                                                                Page 532 of 1169
Set the variable txColBase equal to TxColBase[ plane ][ row >> subY ][ col >> subX ].

Set the variable txRowBase equal to TxRowBase[ plane ][ row >> subY ][ col >> subX ].

Set the variable prevTxColBase equal to TxColBase[ plane ][ prevRow >> subY ][ prevCol >> subX ].

Set the variable prevTxRowBase equal to TxRowBase[ plane ][ prevRow >> subY ][ prevCol >> subX ].

If plane is greater than 0, the chroma information is held in the bottom-right mode info so the variables
are adjusted as follows:

  • row is set equal to baseRow + Num_4x4_Blocks_High[ MiSize ] - 1.
  • col is set equal to baseCol + Num_4x4_Blocks_Wide[ MiSize ] - 1.

Set the variable skip equal to Skips[ row ][ col ].

If plane is greater than 0, the variables are modified as follows:

 if ( RegionTypes[ row ][ col ] == INTRA_REGION ||
       (FrameIsIntra && enable_sdp)) {
     skip = 0
 }


The variable xR is set equal to xP - baseX.

The variable yR is set equal to yP - baseY.

The variable isBlockEdge (equal to 1 if the samples cross a prediction block edge) is derived as follows:

  • If pass is equal to 0 and xR is equal to 0, isBlockEdge is set equal to 1.
  • Otherwise, if pass is equal to 1 and yR is equal to 0, isBlockEdge is set equal to 1.
  • Otherwise, isBlockEdge is set equal to 0.

The variable isTxEdge (equal to 1 if the samples cross a transform block edge) is derived as follows:

  • If txColBase is not equal to prevTxColBase, isTxEdge is set equal to 1.
  • Otherwise, if txRowBase is not equal to prevTxRowBase, isTxEdge is set equal to 1.
  • Otherwise, isTxEdge is set equal to 0.

If isSubPuEdge is equal to 1, the variables txSz, prevTxSz, and isSubPuEdge are modified as follows:

 (txSz,isSubPuEdge) = filt_max_size( pass, isTxEdge, txSz, subPuSize )
 (prevTxSz,_) = filt_max_size( pass, isTxEdge, prevTxSz, prevSubPuSize )
 if ( isBlockEdge ) {
     isSubPuEdge = 0
 }


where the function filt_max_size is specified as:

 filt_max_size( pass, isTxEdge, txSz, subPuSize ) {
     isSubPuEdge = 1
     if ( pass == 0 ) {
         if ( Tx_Width[ txSz ] < Tx_Width[ subPuSize ] ) {



AV2 Specification                                                                             Page 533 of 1169
               isSubPuEdge = 0
           } else if ( !isTxEdge && Tx_Width[ txSz ] == 8 ) {
               txSz = TX_4X4
           } else if ( !isTxEdge && Tx_Width[ txSz ] == 16 &&
                                    Tx_Width[ subPuSize ] == 16 ) {
               txSz = TX_8X8
           } else {
               txSz = subPuSize
           }
      }
      if ( pass == 1 ) {
          if ( Tx_Height[ txSz ] < Tx_Height[ subPuSize ] ) {
              isSubPuEdge = 0
          } else if ( !isTxEdge && Tx_Height[ txSz ] == 8 ) {
              txSz = TX_4X4
          } else if ( !isTxEdge && Tx_Height[ txSz ] == 16 &&
                                   Tx_Height[ subPuSize ] == 16 ) {
              txSz = TX_8X8
          } else {
              txSz = subPuSize
          }
      }
      return (txSz, isSubPuEdge)
 }


The adaptive filter strength process specified in § 7.17.5 Adaptive filter strength process is invoked with
the inputs row, col, plane, and pass, and the output assigned to the variables currQ and currSide.

The adaptive filter strength process specified in § 7.17.5 Adaptive filter strength process is invoked with
the inputs prevRow, prevCol, plane, and pass, and the output assigned to the variables prevQ and
prevSide.

The variable applyFilter (equal to 1 if the samples are filtered) is derived as follows:

  • If isTxEdge is equal to 0 and isSubPuEdge is equal to 0, applyFilter is set equal to 0.
  • Otherwise, if (currQ != 0 && currSide != 0) is equal to 0 and (prevQ != 0 && prevSide != 0) is equal
    to 0, applyFilter is set equal to 0.
  • Otherwise, if isBlockEdge is equal to 1 or skip is equal to 0 or isSubPuEdge is equal to 1, applyFilter
    is set equal to 1.
  • Otherwise, applyFilter is set equal to 0.

If applyFilter is equal to 0, this process terminates immediately.

The filter size process specified in § 7.17.4 Filter size process is invoked with the inputs txSz, prevTxSz,
and pass, and the output assigned to the variable filterSize (containing the maximum filter size that can
be used).

The variable filterSize is clipped at the edge of the screen as follows:

 planeWidth = MiCols * MI_SIZE >> subX
 planeHeight = MiRows * MI_SIZE >> subY
 if ( plane == 0 ) {
     if (xP + dx * 16 > planeWidth || yP + dy * 16 > planeHeight) {
          filterSize = Min(filterSize, 16)
     }
 } else {
     if (xP + dx * 8 > planeWidth || yP + dy * 8 > planeHeight) {




AV2 Specification                                                                               Page 534 of 1169
               filterSize = Min(filterSize, 8)
          }
     }


    The variables qThr and side are set as follows:

     if ( currQ && prevQ ) {
         qThr = (currQ + prevQ + 1) >> 1
     } else {
         qThr = Max( currQ, prevQ )
     }
     if ( currSide && prevSide ) {
         side = (currSide + prevSide + 1) >> 1
     } else {
         side = Max( currSide, prevSide )
     }
     if ( isSubPuEdge && !isTxEdge ) {
         qThr = qThr >> 3
         side = side >> 3
     }


    If prevLossless is equal to 1 and currLossless is equal to 1, this process terminates immediately.

    The filter maximum width process specified in § 7.17.3 Filter maximum width process is invoked with
    plane, filterSize, and sbEdge as inputs, and the outputs are assigned to maxWidthPos and maxWidthNeg.

    The filter choice process specified in § 7.17.7.2 Filter choice process is invoked with xP, yP, plane, qThr,
    side, dx, dy, maxWidthNeg, maxWidthPos, MI_SIZE as inputs, and the output is assigned to width.

    If width is equal to 0, this process terminates immediately.

    For the variable i taking values from 0 to MI_SIZE - 1, the sample filtering process specified in § 7.17.7
    Sample filtering process is invoked with the input variable x set equal to xP + dy * i, the input variable y
    set equal to yP + dx * i, and the variables plane, qThr, dx, dy, Min(width,maxWidthNeg),
    Min(width,maxWidthPos), prevLossless, and currLossless as inputs.


      NOTE: the vector (dx,dy) represents the direction of the filter, while (dy,dx) represents the direction
      of the boundary.

```

<a id="s-7-17-3"></a>

#### § 7.17.3 Filter maximum width process

```text
§   7.17.3. Filter maximum width process

    The inputs to this process are:

      • a variable plane specifying whether the process is filtering Y, U or V samples,
      • a variable filterSize specifying the maximum filter size that can be used,
      • a variable sbEdge specifying if the edge is at a block boundary.

    The variables maxWidthPos and maxWidthNeg are computed as follows:

     if (filterSize <= 4) {
         maxWidthPos = 1
     } else if (filterSize == 8) {
         maxWidthPos = 3
     } else if (filterSize == 16) {
         maxWidthPos = plane != 0 ? 4 : 6
     } else {



    AV2 Specification                                                                               Page 535 of 1169
         maxWidthPos = plane != 0 ? 4 : 8
     }
     if ( sbEdge ) {
         maxWidthNeg = Min( maxWidthPos, plane != 0 ? 2 : 6)
     } else {
         maxWidthNeg = maxWidthPos
     }


    The outputs of this process are the variables maxWidthPos and maxWidthNeg.

```

<a id="s-7-17-4"></a>

#### § 7.17.4 Filter size process

```text
§   7.17.4. Filter size process

    The inputs to this process are:

      • a variable txSz specifying the size of the transform block,
      • a variable prevTxSz specifying the size of the transform block on the other side of the boundary,
      • a variable pass specifying the direction of the edges.

    The output of this process is the variable filterSize containing the maximum filter size that can be used in
    samples.

    The output variable filterSize is derived as follows:

      • If pass is equal to 0, filterSize is set equal to Min( Tx_Width[ prevTxSz ], Tx_Width[ txSz ] ).
      • Otherwise (pass is equal to 1), filterSize is set equal to Min( Tx_Height[ prevTxSz ],
        Tx_Height[ txSz ] ).

```

<a id="s-7-17-5"></a>

#### § 7.17.5 Adaptive filter strength process

```text
§   7.17.5. Adaptive filter strength process

    The inputs to this process are:

      • variables row and col specifying the luma location in units of 4x4 blocks,
      • the variable plane specifying whether the process is filtering Y, U or V samples,
      • the variable pass specifying the direction of the edge being filtered. pass equal to 0 means the
        process is filtering vertical block boundaries, and pass equal to 1 means the process is filtering
        horizontal block boundaries.

    The outputs of this process are the variables qThr and side.

    The variable segment is set as follows:

      • If plane is equal to 0, segment is set equal to SegmentIds[ row ][ col ].
      • Otherwise (plane is greater than 0), segment is set equal to ChromaSegmentIds[ row ][ col ].

    The variable qindex is set as follows:

      • If plane is equal to 0, qindex is set equal to LumaQIndex[ row ][ col ].
      • Otherwise (plane is greater than 0), qindex is set equal to ChromaQIndex[ row ][ col ].

    The adaptive filter strength selection process specified in § 7.17.6 Adaptive filter strength selection
    process is invoked with qindex, segment, plane, and pass as inputs, and the output is assigned to lvl.




    AV2 Specification                                                                                Page 536 of 1169
    The output variables are derived as follows:

     qThr = Round2( get_q( lvl , 0 ), QUANT_TABLE_BITS ) >> 6
     qInd = Clip3( 0, MAX_SIDE_TABLE - 1, lvl - 24 * (BitDepth - 8) )
     side = Max( Side_Thresholds[ qInd ] + (1 << (12 - BitDepth)), 0 ) >>
            ( 13 - BitDepth )


```

<a id="s-7-17-6"></a>

#### § 7.17.6 Adaptive filter strength selection process

```text
§   7.17.6. Adaptive filter strength selection process

    The inputs to this process are:

      • the variable qindex specifying a value derived from the quantizer used for the block,
      • the variable segment specifying the current segment id,
      • the variable plane specifying whether the process is filtering Y, U or V samples,
      • the variable pass specifying the direction of the edge being filtered. pass equal to 0 means the
        process is filtering vertical block boundaries, and pass equal to 1 means the process is filtering
        horizontal block boundaries.

    The output of this process is the variable lvlSeg containing the filter strength level.

    The variable i is set equal to ( plane == 0 ) ? pass : ( plane + 1 ).

    The variable CurrentQIndex is set equal to qindex (CurrentQIndex is used by the get_qindex function).

    The variable lvlSeg is set as follows:

     qindex2 = get_qindex( 0, segment )
     if (plane == 1) {
         delta = DeltaQUAc + BaseUVAcDeltaQ
     } else if (plane == 2) {
         delta = DeltaQVAc + BaseUVAcDeltaQ
     } else {
         delta = 0
     }
     qc = q_clamped(qindex2, delta)
     lvlSeg = qc + DF_DELTA_SCALE * DfDeltaQ[ i ]


    Where the function q_clamped is defined as:

     q_clamped( qindex, delta ) {
         if ( qindex == 0 && delta <= 0 ) {
             return 0
         }
         return Clip3(1, MaxQ, qindex + delta)
     }


```

<a id="s-7-17-7"></a>

#### § 7.17.7 Sample filtering process

```text
§   7.17.7. Sample filtering process

```

<a id="s-7-17-7-1"></a>

##### § 7.17.7.1 General

```text
§   7.17.7.1. General

    The inputs to this process are:

      • variables x and y specifying the location within CurrFrame[ plane ],
      • a variable plane specifying whether the block is the Y, U or V plane,



    AV2 Specification                                                                              Page 537 of 1169
      • a variable qThr specifying a threshold used during the filtering operation,
      • variables dx and dy specifying the direction perpendicular to the edge being filtered,
      • a variable maxWidthNeg specifying the maximum number of samples allowed to be modified for
        negative offsets,
      • a variable maxWidthPos specifying the maximum number of samples allowed to be modified for
        positive offsets,
      • a variable prevLossless specifying if the previous samples are in a lossless segment,
      • a variable currLossless specifying if the current samples are in a lossless segment.

    The outputs of this process are modified values in the array CurrFrame.

    The width is set equal to Max(maxWidthNeg, maxWidthPos).

    The samples are filtered as follows:

     q0 = CurrFrame[ plane ][ y ][ x ]
     q1 = CurrFrame[ plane ][ y + dy     ][ x + dx     ]
     p0 = CurrFrame[ plane ][ y - dy     ][ x - dx     ]
     p1 = CurrFrame[ plane ][ y - dy * 2 ][ x - dx * 2 ]
     qThrClamp = qThr * Q_Thresh_Mults[width - 1]
     deltaM2 = p1 - q1 + 3 * (q0 - p0)
     deltaM2 *= 4
     deltaM2 = Clip3(-qThrClamp, qThrClamp, deltaM2)
     deltaM2Neg = deltaM2 * W_Mult[maxWidthNeg - 1]
     deltaM2Pos = deltaM2 * W_Mult[maxWidthPos - 1]
     for (i = 0; i < width; i++) {
         diffNeg = Round2(deltaM2Neg * (maxWidthNeg - i), 3 + DF_SHIFT)
         diffPos = Round2(deltaM2Pos * (maxWidthPos - i), 3 + DF_SHIFT)
         qy = y + i * dy
         qx = x + i * dx
         if ( !currLossless ) {
             CurrFrame[ plane ][ qy ][ qx ] =
                 Clip1( CurrFrame[ plane ][ qy ][ qx ] - diffPos )
         }
         if ( i < maxWidthNeg && !prevLossless ) {
             pi = -i - 1
             py = y + pi * dy
             px = x + pi * dx
             CurrFrame[ plane ][ py ][ px ] =
                 Clip1( CurrFrame[ plane ][ py ][ px ] + diffNeg )
         }
     }


```

<a id="s-7-17-7-2"></a>

##### § 7.17.7.2 Filter choice process

```text
§   7.17.7.2. Filter choice process

    The inputs to this process are:

      • variables x and y specifying the location within CurrFrame[ plane ],
      • a variable plane specifying whether the block is the Y, U or V plane,
      • variables qThr and sideThr that specify thresholds used during the filtering operation,
      • variables dx and dy specifying the direction perpendicular to the edge being filtered,
      • a variable maxWidthNeg specifying the maximum number of samples allowed to be modified for
        negative offsets,




    AV2 Specification                                                                             Page 538 of 1169
  • a variable maxWidthPos specifying the maximum number of samples allowed to be modified for
    positive offsets,
  • a variable count specifying the length of the edge.

The output from this process is the chosen filter width.

If qThr is equal to 0 or sideThr is equal to 0, the process terminates immediately with 0 as output.

The variable maxSamplesPos is set equal to Clip3(3, 8, maxWidthPos + 1).

The variable maxSamplesNeg is set equal to Clip3(3, 8, maxWidthNeg + 1).

Arrays s and t containing samples for indices from -maxSamplesNeg to maxSamplesPos - 1 are prepared
as follows:

 x2 = x + (count - 1) * dy
 y2 = y + (count - 1) * dx
 for (dist = 0; dist < maxSamplesPos; dist++) {
     s[dist] = CurrFrame[ plane ][ y + dist * dy ][ x + dist * dx ]
     t[dist] = CurrFrame[ plane ][ y2 + dist * dy ][ x2 + dist * dx ]
 }
 for (dist = 0; dist < maxSamplesNeg; dist++) {
     s[-dist-1] = CurrFrame[ plane ]
                           [ y - (dist + 1) * dy ]
                           [ x - (dist + 1) * dx ]
     t[-dist-1] = CurrFrame[ plane ]
                           [ y2 - (dist + 1) * dy ]
                           [ x2 - (dist + 1) * dx ]
 }


An array secondDeriv containing the estimated second derivative of the samples for indices from -2 to 1
is prepared as follows:

 for (dist = -2; dist < 2; dist++) {
     p0 = s[dist - 1]
     q0 = s[dist]
     q1 = s[dist+1]
     derivS = Abs(p0 - (q0 << 1) + q1)
     p0 = t[dist - 1]
     q0 = t[dist]
     q1 = t[dist+1]
     derivT = Abs(p0 - (q0 << 1) + q1)
     secondDeriv[dist] = (derivS + derivT + 1) >> 1
 }


The width to return is calculated as follows:

 if (secondDeriv[-2] > sideThr || secondDeriv[1] > sideThr) return 0
 if (maxWidthPos == 1) return 1

 sideThr2 = sideThr >> 2
 if (secondDeriv[-2] > sideThr2 || secondDeriv[1] > sideThr2) return 1
 if (secondDeriv[-1] + secondDeriv[0] > qThr * 4) return 1

 sideThr3 = sideThr >> 3
 if (secondDeriv[-2] > sideThr3 || secondDeriv[1] > sideThr3) return 2
 if (secondDeriv[-1] + secondDeriv[0] > qThr * 3) return 2

 endThr = (sideThr * 3) >> 4
 if ( maxWidthNeg > 2 ) {



AV2 Specification                                                                             Page 539 of 1169
          derivS = Abs(s[-1] - s[-4] - 3 * (s[-1] - s[-2]))
          derivT = Abs(t[-1] - t[-4] - 3 * (t[-1] - t[-2]))
          if ( ((derivS + derivT + 1) >> 1) > endThr ) return 2
     }
     derivS = Abs(s[0] - s[3] - 3 * (s[0] - s[1]))
     derivT = Abs(t[0] - t[3] - 3 * (t[0] - t[1]))
     if ( ((derivS + derivT + 1) >> 1) > endThr ) return 2
     if (maxWidthPos == 3) return 3

     transition = (secondDeriv[-1] + secondDeriv[0]) << 4
     prevDist = 3
     for (dist = 4; dist <= maxWidthPos; dist += 2) {
         qThr4 = qThr * Q_First[dist - 4]
         endThr4 = (sideThr * dist) >> 4
         if (transition > qThr4) return prevDist
         dist2 = Min(7,dist)
         if ( maxWidthNeg >= dist2 ) {
             derivS = Abs(s[-1] - s[-dist2 - 1] - dist2 * (s[-1] - s[-2]))
             derivT = Abs(t[-1] - t[-dist2 - 1] - dist2 * (t[-1] - t[-2]))
             if ( ((derivS + derivT + 1) >> 1) > endThr4) return prevDist
         }
         derivS = Abs(s[0] - s[dist2] - dist2 * (s[0] - s[1]))
         derivT = Abs(t[0] - t[dist2] - dist2 * (t[0] - t[1]))
         if ( ((derivS + derivT + 1) >> 1) > endThr4) return prevDist
         prevDist = dist
     }
     return maxWidthPos


```

<a id="s-7-18"></a>

### § 7.18 CDEF process

```text
§   7.18. CDEF process
    Input to this process is the array CurrFrame of reconstructed samples.

    Output from this process is the array CdefFrame containing deringed samples.

    The purpose of CDEF is to perform deringing based on the detected direction of blocks.

    CDEF parameters are stored for each 64 by 64 block of luma samples.

    The CDEF filter is applied on each 8 by 8 block as follows:

     step4 = Num_4x4_Blocks_Wide[ BLOCK_8X8 ]
     cdefSize4 = Num_4x4_Blocks_Wide[ BLOCK_64X64 ]
     cdefMask4 = ~(cdefSize4 - 1)
     for ( r = 0; r < MiRows; r += step4 ) {
         for ( c = 0; c < MiCols; c += step4 ) {
               baseR = r & cdefMask4
               baseC = c & cdefMask4
               idx = cdef_idx[ baseR ][ baseC ]
               cdef_block(r, c, idx)
         }
     }


    When the cdef_block function is called, the CDEF block process specified in § 7.18.1 CDEF block process
    is invoked with r, c, and idx as inputs.

```

<a id="s-7-18-1"></a>

#### § 7.18.1 CDEF block process

```text
§   7.18.1. CDEF block process

    The inputs to this process are:

      • variables r and c specifying the location of an 8x8 block in units of 4x4 blocks in the luma plane,




    AV2 Specification                                                                             Page 540 of 1169
  • a variable idx specifying which set of CDEF parameters to use, or -1 to signal that no filtering is to be
    applied.

The block is first copied to the CdefFrame as follows:

 startY = r * MI_SIZE
 endY = startY + MI_SIZE * 2
 startX = c * MI_SIZE
 endX = startX + MI_SIZE * 2
 for ( y = startY; y < endY; y++ ) {
     for ( x = startX; x < endX; x++ ) {
         CdefFrame[ 0 ][ y ][ x ] = CurrFrame[ 0 ][ y ][ x ]
     }
 }
 if ( NumPlanes > 1 ) {
     startY >>= SubsamplingY
     endY >>= SubsamplingY
     startX >>= SubsamplingX
     endX >>= SubsamplingX
     for ( y = startY; y < endY; y++ ) {
         for ( x = startX; x < endX; x++ ) {
             CdefFrame[ 1 ][ y ][ x ] = CurrFrame[ 1 ][ y ][ x ]
             CdefFrame[ 2 ][ y ][ x ] = CurrFrame[ 2 ][ y ][ x ]
         }
     }
 }



  NOTE: If CDEF filtering turns out to be needed, then the contents of CdefFrame will be overwritten
  later in this process.


If idx is equal to -1, then the process returns immediately after performing this copy.

The variable coeffShift is set equal to BitDepth - 8.

The variable skip is set as follows:

  • If cdef_on_skip_txfm_frame_enable is equal to 0, skip is set equal to ( Skips[ r ][ c ] && Skips[ r + 1 ]
    [ c ] && Skips[ r ][ c + 1 ] && Skips[ r + 1 ][ c + 1 ] ).
  • Otherwise (cdef_on_skip_txfm_frame_enable is equal to 1), skip is set equal to 0.

The variables skip and skipChroma are updated as follows:

 skipChroma = 0
 for( i = 0; i < 2; i++ ) {
     for( j = 0; j < 2; j++ ) {
         s = SegmentIds[ r + i ][ c + j ]
         skip = skip | LosslessArray[ s ]
         if ( NumPlanes > 1 ) {
             s = ChromaSegmentIds[ r + i ][ c + j ]
             skipChroma = skipChroma | LosslessArray[ s ]
         }
     }
 }


If skip is equal to 0, the CDEF direction process specified in § 7.18.2 CDEF direction process is invoked
with r and c as inputs, and the outputs assigned to variables yDir and var.




AV2 Specification                                                                               Page 541 of 1169
    If skip is equal to 0, the following ordered steps apply:

     1. The variable priStr is set equal to cdef_y_pri_strength[ idx ] << coeffShift.
     2. The variable secStr is set equal to cdef_y_sec_strength[ idx ] << coeffShift.
     3. The variable dir is set equal to ( priStr == 0 ) ? 0 : yDir.
     4. The variable varStr is set equal to ( var >> 6 ) ? Min( FloorLog2( var >> 6 ), 12) : 0.
     5. The variable priStr is set equal to ( var ? ( priStr * ( 4 + varStr ) + 8 ) >> 4 : 0 ).
     6. The variable damping is set equal to CdefDamping + coeffShift.
     7. The CDEF filter process specified in § 7.18.3 CDEF filter process is invoked with plane equal to 0, r, c,
        priStr, secStr, damping, and dir as input.
     8. If NumPlanes is equal to 1 or skipChroma is equal to 1, the process terminates at this point (i.e.,
        filtering is not done for the U and V planes).
     9. The variable priStr is set equal to cdef_uv_pri_strength[ idx ] << coeffShift.
    10. The variable secStr is set equal to cdef_uv_sec_strength[ idx ] << coeffShift.
    11. The variable dir is set equal to ( priStr == 0 ) ? 0 : Cdef_Uv_Dir[ SubsamplingX ][ SubsamplingY ]
        [ yDir ].
    12. The variable damping is set equal to CdefDamping + coeffShift - 1.
    13. The CDEF filter process specified in § 7.18.3 CDEF filter process is invoked with plane equal to 1, r, c,
        priStr, secStr, damping, and dir as input.
    14. The CDEF filter process specified in § 7.18.3 CDEF filter process is invoked with plane equal to 2, r, c,
        priStr, secStr, damping, and dir as input.

    Cdef_Uv_Dir is a constant lookup table defined as:

     Cdef_Uv_Dir[ 2 ][ 2 ][ 8 ] = {
       { {0, 1, 2, 3, 4, 5, 6, 7},
         {1, 2, 2, 2, 3, 4, 6, 0} },
       { {7, 0, 2, 4, 5, 6, 6, 6},
         {0, 1, 2, 3, 4, 5, 6, 7} }
     }


```

<a id="s-7-18-2"></a>

#### § 7.18.2 CDEF direction process

```text
§   7.18.2. CDEF direction process

    The inputs to this process are variables r and c specifying the location of an 8x8 block in units of 4x4
    blocks in the luma plane.

    The outputs of this process are:

      • a variable yDir containing the direction of this block,
      • a variable var containing the variance for this block.

    This block uses luma samples to measure the direction and variance of a block.




    AV2 Specification                                                                              Page 542 of 1169
The process is specified as:

 for ( i = 0; i < 8; i++ ) {
     cost[i] = 0
     for ( j = 0; j < 15; j++ )
          partial[i][j] = 0
 }
 bestCost = 0
 yDir = 0
 x0 = c << MI_SIZE_LOG2
 y0 = r << MI_SIZE_LOG2
 for ( i = 0; i < 8; i++ ) {
     for ( j = 0; j < 8; j++ ) {
          x = (CurrFrame[ 0 ][y0 + i][x0 + j] >> (BitDepth - 8)) - 128
          partial[0][i + j] += x
          partial[1][i + j / 2] += x
          partial[2][i] += x
          partial[3][3 + i - j / 2] += x
          partial[4][7 + i - j] += x
          partial[5][3 - i / 2 + j] += x
          partial[6][j] += x
          partial[7][i / 2 + j] += x
     }
 }
 for ( i = 0; i < 8; i++ ) {
     cost[2] += partial[2][i] * partial[2][i]
     cost[6] += partial[6][i] * partial[6][i]
 }
 cost[2] *= Div_Table[8]
 cost[6] *= Div_Table[8]
 for ( i = 0; i < 7; i++ ) {
     cost[0] += (partial[0][i] * partial[0][i] +
                  partial[0][14 - i] * partial[0][14 - i]) *
                 Div_Table[i + 1]
     cost[4] += (partial[4][i] * partial[4][i] +
                  partial[4][14 - i] * partial[4][14 - i]) *
                 Div_Table[i + 1]
 }
 cost[0] += partial[0][7] * partial[0][7] * Div_Table[8]
 cost[4] += partial[4][7] * partial[4][7] * Div_Table[8]
 for ( i = 1; i < 8; i += 2 ) {
     for ( j = 0; j < 4 + 1; j++ ) {
       cost[i] += partial[i][3 + j] * partial[i][3 + j]
     }
     cost[i] *= Div_Table[8]
     for ( j = 0; j < 4 - 1; j++ ) {
          cost[i] += (partial[i][j] * partial[i][j] +
                    partial[i][10 - j] * partial[i][10 - j]) *
                    Div_Table[2 * j + 2]
     }
 }
 for ( i = 0; i < 8; i++ ) {
     if ( cost[i] > bestCost ) {
       bestCost = cost[i]
       yDir = i
     }
 }
 var = (bestCost - cost[(yDir + 4) & 7]) >> 10


where the Div_Table is a constant lookup table specified as:

 Div_Table[9] = {
     0, 840, 420, 280, 210, 168, 140, 120, 105
 }




AV2 Specification                                                        Page 543 of 1169
```

<a id="s-7-18-3"></a>

#### § 7.18.3 CDEF filter process

```text
§   7.18.3. CDEF filter process

    The inputs to this process are:

      • a variable plane specifying which plane is being predicted,
      • variables r and c specifying the location of an 8x8 block in units of 4x4 blocks in the luma plane,
      • a variable priStr specifying the primary filter strength,
      • a variable secStr specifying the secondary filter strength,
      • a variable damping specifying a shift used for damping,
      • a variable dir specifying the detected direction of the block.

    The process modifies samples in CdefFrame based on filtering samples from CurrFrame.

    The variable coeffShift is set equal to BitDepth - 8.

    The filtering is applied as follows:

     MiColStart = MiColStartGrid[ r ][ c ]
     MiRowStart = MiRowStartGrid[ r ][ c ]
     MiColEnd = MiColEndGrid[ r ][ c ]
     MiRowEnd = MiRowEndGrid[ r ][ c ]
     subX = (plane > 0) ? SubsamplingX : 0
     subY = (plane > 0) ? SubsamplingY : 0
     x0 = (c * MI_SIZE ) >> subX
     y0 = (r * MI_SIZE ) >> subY
     w = 8 >> subX
     h = 8 >> subY
     for ( i = 0; i < h; i++ ) {
         for ( j = 0; j < w; j++ ) {
             sum = 0
             x = CurrFrame[plane][y0 + i][x0 + j]
             max = x
             min = x
             for ( k = 0; k < 2; k++ ) {
                 for ( sign = -1; sign <= 1; sign += 2 ) {
                     p = cdef_get_at(plane, x0, y0, i, j, dir, k, sign, subX, subY)
                     if ( CdefAvailable ) {
                         sum += Cdef_Pri_Taps[(priStr >> coeffShift) & 1][k] *
                                 constrain(p - x, priStr, damping )
                         max = Max(p, max)
                         min = Min(p, min)
                     }
                     for ( dirOff = -2; dirOff <= 2; dirOff += 4) {
                         s = cdef_get_at( plane, x0, y0, i, j, (dir + dirOff) & 7, k,
                                           sign, subX, subY)
                         if ( CdefAvailable ) {
                             sum += Cdef_Sec_Taps[(priStr >> coeffShift) & 1][k] *
                                     constrain(s - x, secStr, damping )
                             max = Max(s, max)
                             min = Min(s, min)
                         }
                     }
                 }
             }
             CdefFrame[plane][y0 + i][x0 + j] =
                 Clip3(min, max, x + ((8 + sum - (sum < 0)) >> 4) )
         }
     }




    AV2 Specification                                                                             Page 544 of 1169
    where Cdef_Pri_Taps and Cdef_Sec_Taps are constant lookup tables specified as:

     Cdef_Pri_Taps[2][2] = {
         { 4, 2 }, { 3, 3 }
     }

     Cdef_Sec_Taps[2][2] = {
         { 2, 1 }, { 2, 1 }
     }


    constrain is specified as:

     constrain(diff, threshold, damping) {
         if ( !threshold )
           return 0
         dampingAdj = Max(0, damping - FloorLog2( threshold ) )
         sign = (diff < 0) ? -1 : 1
         return sign * Clip3(0, Abs(diff), threshold - (Abs(diff) >> dampingAdj) )
     }


    cdef_get_at fetches a sample from CurrFrame and sets CdefAvailable according to whether the sample is
    available. cdef_get_at is specified as:

     cdef_get_at(plane, x0, y0, i, j, dir, k, sign, subX, subY) {
         y = y0 + i + sign * Cdef_Directions[dir][k][0]
         x = x0 + j + sign * Cdef_Directions[dir][k][1]
         candidateR = (y << subY) >> MI_SIZE_LOG2
         candidateC = (x << subX) >> MI_SIZE_LOG2
         if ( is_inside_filter_region( candidateR, candidateC ) ) {
             CdefAvailable = 1
             return CurrFrame[ plane ][ y ][ x ]
         } else {
             CdefAvailable = 0
             return 0
         }
     }


    where Cdef_Directions is a constant lookup table defined as:

     Cdef_Directions[8][2][2] = {
       { { -1, 1 }, { -2, 2 } },
       { { 0, 1 }, { -1, 2 } },
       { { 0, 1 }, { 0, 2 } },
       { { 0, 1 }, { 1, 2 } },
       { { 1, 1 }, { 2, 2 } },
       { { 1, 0 }, { 2, 1 } },
       { { 1, 0 }, { 2, 0 } },
       { { 1, 0 }, { 2, -1 } }
     }


```

<a id="s-7-19"></a>

### § 7.19 CCSO process

```text
§   7.19. CCSO process
    Input to this process is the array CurrFrame of reconstructed samples and the array CdefFrame of
    samples that have had CDEF applied.

    This process modifies the samples in CdefFrame.




    AV2 Specification                                                                        Page 545 of 1169
    A CCSO enable bit is stored for each plane for each (1<<CcsoLumaSizeLog2) by (1<<CcsoLumaSizeLog2) block of
    luma samples.

    The following applies for plane=0..NumPlanes-1:

      • If ccso_planes[plane] is equal to 1, the apply CCSO filter process in § 7.19.1 Apply CCSO filter process
        is invoked with plane as input.

```

<a id="s-7-19-1"></a>

#### § 7.19.1 Apply CCSO filter process

```text
§   7.19.1. Apply CCSO filter process

    The input to this process is a variable plane specifying which plane is being modified.

    This process modifies the samples in CdefFrame[plane].

    Variables subX and subY are prepared as follows:

     if ( plane == 0 ) {
         subX = 0
         subY = 0
     } else {
         subX = SubsamplingX
         subY = SubsamplingY
     }


    Variables blkW, blkH representing the CCSO block size in units of samples in the current plane are
    derived as follows:

     shiftY = CcsoLumaSizeLog2 - subY
     shiftX = CcsoLumaSizeLog2 - subX
     blkH = 1 << shiftY
     blkW = 1 << shiftX


    The filtering is applied as follows:

     planeWidth = MiCols * MI_SIZE >> subX
     planeHeight = MiRows * MI_SIZE >> subY
     maxBandLog2 = ccso_max_band_log2[plane]
     extFilter = ccso_ext_filter[plane]
     quantStep = CCSO_Quant_Sz[ ccso_scale_idx[plane] ][ ccso_quant_idx[plane] ]
     dy = Ccso_Pos[extFilter][0]
     dx = Ccso_Pos[extFilter][1]
     for(y = 0; y < planeHeight; y += blkH) {
         for(x = 0; x < planeWidth; x += blkW) {
             unitRow = y >> shiftY
             unitCol = x >> shiftX
             useCcso = CcsoBlks[ plane ][ unitRow ][ unitCol ]
             sbBlkW = Block_Width[ SbSize ] >> subX
             sbBlkH = Block_Height[ SbSize ] >> subY
             for (y2 = y; y2 < Min(planeHeight, y + blkH); y2 += sbBlkH) {
                 for (x2 = x; x2 < Min(planeWidth, x + blkW); x2 += sbBlkW) {
                     if ( useCcso && BruModes[y2 >> (MI_SIZE_LOG2 - subY)]
                                              [x2 >> (MI_SIZE_LOG2 - subX)] ==
                                                  BRU_ACTIVE ) {
                         shift = BitDepth - maxBandLog2
                         for(y3 = y2; y3 < Min(planeHeight, y2 + sbBlkH); y3++) {
                             for(x3 = x2; x3 < Min(planeWidth, x2 + sbBlkW); x3++) {
                                 yLuma = y3 << subY
                                 xLuma = x3 << subX
                                 row = yLuma >> MI_SIZE_LOG2
                                 col = xLuma >> MI_SIZE_LOG2
                                 s = ( plane > 0 ) ? ChromaSegmentIds[row][col] :



    AV2 Specification                                                                             Page 546 of 1169
                                                    SegmentIds[ row ][ col ]
                                if ( !LosslessArray[ s ] ) {
                                    if ( disable_loopfilters_across_tiles ) {
                                        miColStart = MiColStartGrid[row][col]
                                        miColEnd = MiColEndGrid[row][col]
                                        miRowStart = MiRowStartGrid[row][col]
                                        miRowEnd = MiRowEndGrid[row][col]
                                    } else {
                                        miColStart = 0
                                        miRowStart = 0
                                        miColEnd = MiCols
                                        miRowEnd = MiRows
                                    }
                                    LumaStartX = miColStart * MI_SIZE
                                    LumaStartY = miRowStart * MI_SIZE
                                    LumaEndX = miColEnd * MI_SIZE - 1
                                    LumaEndY = miRowEnd * MI_SIZE - 1

                                    c = get_ccso_luma(xLuma,yLuma)
                                    band = c >> shift
                                    if ( ccso_bo_only[plane] ) {
                                        cls0 = 0
                                        cls1 = 0
                                    } else {
                                        cls0 = ccso_score( get_ccso_luma(xLuma+dx,
                                                                 yLuma+dy) - c,
                                                        quantStep,
                                                        ccso_edge_clf[plane])
                                        cls1 = ccso_score( get_ccso_luma(xLuma-dx,
                                                                 yLuma-dy) - c,
                                                        quantStep,
                                                        ccso_edge_clf[plane])
                                    }
                                    CdefFrame[plane][y3][x3] =
                                      Clip1( CdefFrame[plane][y3][x3] +
                                        CcsoFilterOffset[plane][band][cls0][cls1] )
                                }
                            }
                        }
                    }
                }
           }
      }
 }


where get_ccso_luma gets luma samples from CurrFrame (before CDEF filtering) and is defined as:

 get_ccso_luma(x,y) {
     return CurrFrame[ 0 ]
                      [ Clip3( LumaStartY, LumaEndY, y ) ]
                      [ Clip3( LumaStartX, LumaEndX, x ) ]
 }


and ccso_score is defined as:

 ccso_score( diff, quantStep, edgeClassifier ) {
     if ( diff > quantStep && edgeClassifier == 0 )
          return 2
     else if (diff < -quantStep)
          return 0
     else
          return 1
 }




AV2 Specification                                                                       Page 547 of 1169
    and CCSO_Quant_Sz is defined as:

     CCSO_Quant_Sz[4][4] = {
         { 16, 8, 32, 0 },
         { 56, 40, 64, 128 },
         { 48, 24, 96, 192 },
         { 80, 112, 160, 256 }
     }



      NOTE: If edgeClassifier is 0, different classes are used for positive and negative significant
      differences. If edgeClassifier is 1, positive significant differences are treated the same as there being
      no difference.


    The table Ccso_Pos is defined as:

     Ccso_Pos[7][2] = {
       {-1, 0},
       {0, -1},
       {-1, -1},
       {-1, 1},
       {-1, -2},
       {1, -2},
       {0, 2}
     }


```

<a id="s-7-20"></a>

### § 7.20 Loop restoration process

```text
§   7.20. Loop restoration process
    Input to this process are the arrays CurrFrame (of reconstructed samples) and CdefFrame (of deringed
    samples).

    Output from this process is the array LrFrame of loop restored samples.


      NOTE: Although this process loops over 4x4 blocks, loop restoration is designed to work in stripes
      64 luma samples high without needing additional line buffers. Samples within the current stripe are
      fetched from CdefFrame. Samples outside the current stripe are fetched from CurrFrame (these
      samples will be deblocked, but will not have CDEF and CCSO filtering applied).


    The array LrFrame is set equal to a copy of CdefFrame. (The contents of LrFrame will later be
    overwritten for blocks that require restoration filtering.)

    If UsesLr is equal to 0 and gdf_frame_enable is equal to 0, then the process returns immediately after
    performing this copy.

    Otherwise, loop restoration is applied as follows:

     for ( plane = 0; plane < NumPlanes; plane++ ) {
       for ( y = 0; y < MiRows * MI_SIZE; y += MI_SIZE ) {
         for ( x = 0; x < MiCols * MI_SIZE; x += MI_SIZE ) {
           if ( FrameRestorationType[ plane ] != RESTORE_NONE ||
                ( plane==0 && gdf_frame_enable ) ) {
             row = y >> MI_SIZE_LOG2
             col = x >> MI_SIZE_LOG2
             loop_restore_block( plane, row, col )
           }




    AV2 Specification                                                                              Page 548 of 1169
             }
         }
     }


    When loop_restore_block is called, the loop restore block process in § 7.20.1 Loop restore block process is
    invoked with plane, row, and col as inputs.

```

<a id="s-7-20-1"></a>

#### § 7.20.1 Loop restore block process

```text
§   7.20.1. Loop restore block process

    The inputs to this process are:

      • a variable plane specifying whether the process is filtering Y, U, or V samples,
      • variables row and col specifying the location of the block in units of 4x4 blocks in the upscaled luma
        plane.

    The output of this process are samples in LrFrame[ plane ].

    The variable unitSize (specifying the size of restoration units in units of samples in the current plane) is
    set as follows:

      • If FrameRestorationType[ plane ] is equal to RESTORE_NONE, unitSize is set equal to
        RESTORATION_TILESIZE_MAX.
      • Otherwise (FrameRestorationType[ plane ] is not equal to RESTORE_NONE), unitSize is set equal to
        LoopRestorationSize[ plane ].

    The variables subX and subY are set equal to the subsampling for the current plane as follows:

      • If plane is equal to 0, subX is set equal to 0 and subY is set equal to 0.
      • Otherwise, subX is set equal to SubsamplingX and subY is set equal to SubsamplingY.

    If plane is equal to 0 and LosslessArray[SegmentIds[ row ][ col ]] is equal to 1, this process terminates
    immediately.

    If plane is greater than 0 and LosslessArray[ChromaSegmentIds[ row ][ col ]] is equal to 1, this process
    terminates immediately.

    The variable x is set equal to col * MI_SIZE >> subX.

    The variable y is set equal to row * MI_SIZE >> subY.

    (Variables x and y represent the position of the block in samples relative to the top-left corner of the
    current plane.)

    The variable MiColStart is set equal to MiColStartGrid[ row ][ col ].

    The variable MiColEnd is set equal to MiColEndGrid[ row ][ col ].

    The variable MiRowStart is set equal to MiRowStartGrid[ row ][ col ].

    The variable MiRowEnd is set equal to MiRowEndGrid[ row ][ col ].

    The variable lrRowOffset is set equal to (MiRowStart * MI_SIZE >> subY) / unitSize.




    AV2 Specification                                                                               Page 549 of 1169
The variable lrColOffset is set equal to (MiColStart * MI_SIZE >> subX) / unitSize.

The variable sbShift is set equal to Mi_Width_Log2[ SbSize ].

The variable stripeRow (specifying the row of the start of the stripe in units of 4x4 blocks) is set equal to
Min( MiRowEnd - 1, ((row + 2) >> 4) << 4 ).


If use_bru is equal to 1 and BruModes[ (stripeRow >> sbShift) << sbShift ][ (col >> sbShift) << sbShift ] is not
equal to BRU_ACTIVE, this process terminates immediately.

The variable col is set equal to col - MiColStart.

The variable row is set equal to row - MiRowStart.

The variable miCols is set equal to MiColEnd - MiColStart.

The variable miRows is set equal to MiRowEnd - MiRowStart.

The variable lumaY is set equal to row * MI_SIZE.

The variable stripeNum (specifying the zero-based index of the current stripe) is set equal to (lumaY +
8) / 64.


  NOTE: The stripes are offset upwards by 8 luma samples to make pipelined implementations more
  efficient. When a row of superblocks has been received, enough rows of deblocked output can be
  produced to allow loop restoration of the corresponding stripes.


The variable unitRows (specifying the number of restoration units down the frame) is set equal to
count_units_in_frame( unitSize, miRows * MI_SIZE >> subY ).


The variable unitCols (specifying the number of restoration units across the frame) is set equal to
count_units_in_frame( unitSize, miCols * MI_SIZE >> subX ).


  NOTE:       The number of restoration units in a frame can be different for chroma and luma.


The variable unitRow (specifying the vertical index of the current loop restoration unit) is set equal to
lrRowOffset + Min( unitRows - 1, ( ( row * MI_SIZE + 8) >> subY ) / unitSize ).


The variable unitCol (specifying the horizontal index of the current loop restoration unit) is set equal to
lrColOffset + Min( unitCols - 1, ( col * MI_SIZE >> subX ) / unitSize ).


The horizontal extent of the space allowed for filtering is specified as follows:

The variable w is set equal to MI_SIZE >> subX.

The variable h is set equal to MI_SIZE >> subY.

(Variables w and h represent the size of the block in samples.)


  NOTE: Although the filter is described as operating on small blocks, the output will be the same if
  larger blocks are used - provided all contained samples belong to the same loop restoration unit.




AV2 Specification                                                                                Page 550 of 1169
The variable unclippedStripeStartY is set equal to MiRowStart * MI_SIZE + stripeNum * 64 - 8.

The variable unclippedStripeEndY is set equal to unclippedStripeStartY + 64.

The variables representing which luma pixels are allowed to be accessed are set as follows:

 if ( disable_loopfilters_across_tiles ) {
     LumaStartX = MiColStart * MI_SIZE
     LumaEndX = MiColEnd * MI_SIZE - 1
     LumaStartY = MiRowStart * MI_SIZE
     LumaEndY = MiRowEnd * MI_SIZE - 1
 } else {
     LumaStartX = 0
     LumaEndX = MiCols * MI_SIZE - 1
     LumaStartY = 0
     LumaEndY = MiRows * MI_SIZE - 1
 }
 LumaStripeStartY = Max( LumaStartY, unclippedStripeStartY)
 LumaStripeEndY = Min( LumaEndY, unclippedStripeEndY - 1)


The variable rType (specifying the loop restoration type) is set as follows:

  • If FrameRestorationType[ plane ] is equal to RESTORE_NONE, rType is set equal to
    RESTORE_NONE.
  • Otherwise (FrameRestorationType[ plane ] is not equal to RESTORE_NONE), rType is set equal to
    LrType[ plane ][ unitRow ][ unitCol ].

The filter to be used depends on rType as follows:

  • If rType is equal to RESTORE_WIENER_NONSEP, the following ordered steps apply:

      1. If frame_filters_on[ plane ] is equal to 1 and plane is equal to 0 and NumFilterClasses is greater
         than 1, the pixel-classified Wiener filter process specified in § 7.20.4 Pixel classified Wiener filter
         process is invoked with x, y, w, h, 1 as inputs.
      2. The non-separable Wiener filter process specified in § 7.20.3 Non-separable Wiener filter process
         is invoked with plane, unitRow, unitCol, x, y, w, and h as inputs.
  • Otherwise, if rType is equal to RESTORE_PC_WIENER, the pixel-classified Wiener filter process
    specified in § 7.20.4 Pixel classified Wiener filter process is invoked with x, y, w, h, 0 as inputs.
  • Otherwise (rType is equal to RESTORE_NONE), no filtering is applied.

The guided detail filter is conditionally applied on this block as follows:

 if ( plane == 0 && gdf_frame_enable &&
      ( gdf_per_block == 0 ||
        GdfBlks[stripeRow * MI_SIZE / GdfBlkSize][x / GdfBlkSize] ) ) {
     qpBase = FrameIsIntra ? 85 : 110
     qpDiff = base_q_idx - qpBase - 24 * (BitDepth - 8)
     qpIdx = Clip3( 0, 2, (qpDiff - 37)/25 ) + gdf_pic_qc_idx
     if (FrameIsIntra) {
         refDstIdx = 0
     } else {
         maxDist = 0
         for(i = 0; i < Min( NumTotalRefs, 2); i++ ) {
              if ( OrderHints[ i ] != RESTRICTED_OH ) {
                  maxDist = Max( Abs(FrameDistance[i]), maxDist)
              }
         }



AV2 Specification                                                                                 Page 551 of 1169
               if (maxDist == 0)
                    refDstIdx = 5
               else if (maxDist < 2)
                    refDstIdx = 1
               else if (maxDist < 3)
                    refDstIdx = 2
               else if (maxDist < 6)
                    refDstIdx = 3
               else if (maxDist < 11)
                    refDstIdx = 4
               else
                    refDstIdx = 5
          }
          apply_gdf_filter(x,y,qpIdx,refDstIdx,4,4,unclippedStripeEndY)
     }


    The function call to apply_gdf_filter indicates that the apply GDF filter process specified in § 7.20.5 Apply
    GDF filter process is invoked.

```

<a id="s-7-20-2"></a>

#### § 7.20.2 Get source sample process

```text
§   7.20.2. Get source sample process

    The inputs to this process are:

      • a variable plane specifying whether the process is filtering Y, U, or V samples,
      • variables x and y specifying the location in the current plane in units of samples.

    This process makes sure samples are taken from within the allowed extent for loop restoration filtering.

    Samples within the current stripe are taken after CDEF and CCSO filtering has been applied, samples
    outside the current stripe are taken before CDEF and CCSO filtering.

    The sample to return is specified as follows:

     subX = (plane == 0) ? 0 : SubsamplingX
     subY = (plane == 0) ? 0 : SubsamplingY
     x = Clip3( LumaStartX >> subX, LumaEndX >> subX, x )
     y = Clip3( LumaStartY >> subY, LumaEndY >> subY, y )
     stripeStartY = LumaStripeStartY >> subY
     stripeEndY = LumaStripeEndY >> subY
     if (y < stripeStartY) {
         y = Max(stripeStartY - 2,y)
         return CurrFrame[ plane ][ y ][ x ]
     } else if (y > stripeEndY) {
         y = Min(stripeEndY + 2,y)
         return CurrFrame[ plane ][ y ][ x ]
     } else {
         return CdefFrame[ plane ][ y ][ x ]
     }



      NOTE: This process can be called for samples on the lines above and lines below the current stripe.
      However, the coordinates are cropped such that only two lines above and below the stripe need to be
      fetched. In other words, requests for the third line (above or below) are given a copy of the second
      line.




    AV2 Specification                                                                              Page 552 of 1169
```

<a id="s-7-20-3"></a>

#### § 7.20.3 Non-separable Wiener filter process

```text
§   7.20.3. Non-separable Wiener filter process

    The inputs to this process are:

      • a variable plane specifying whether the process is filtering Y, U, or V samples,
      • variables unitRow and unitCol specifying the position of the loop restoration unit,
      • variables x and y specifying the position of the block in samples relative to the top-left corner of the
        current plane,
      • variables w and h specifying the size of the block in samples.

    The output from this process are modified samples in LrFrame.

    For luma this process applies a non-separable filter to the luma samples.

    For chroma this process applies a non-separable filter to the chroma samples that includes taps from both
    chroma and luma samples.

    The filtering is applied as follows:

     if (plane==0) {
         nTaps = WIENER_NS_TAPS_Y
         config = Wiener_Ns_Config_Y
     } else {
         nTaps = WIENER_NS_TAPS_UV
         config = Wiener_Ns_Config_Uv
     }
     for ( r = 0; r < h; r++ ) {
         for ( c = 0; c < w; c++ ) {
              m = get_source_sample(plane, x + c, y + r)
              s = m << WIENER_NS_PREC_BITS
              if ( plane == 0 && frame_filters_on[ plane ] && NumFilterClasses > 1 ) {
                  cls = FilterClass[ (y + r) >> 2 ][ (x + c) >> 2 ]
                  subcls = SubclassLookup[ cls ]
              } else {
                  subcls = 0
              }
              for ( i = 0; i < nTaps; i++ ) {
                  dy = config[ i ][ 0 ]
                  dx = config[ i ][ 1 ]
                  idx = config[ i ][ 2 ]
                  diff = get_source_sample( plane, x + c + dx, y + r + dy ) - m
                  if ( frame_filters_on[ plane ] ) {
                       coeff = FrameLrWienerNs[ plane ][ subcls ][ idx ]
                  } else {
                       coeff = LrWienerNs[ plane ][ unitRow ][ unitCol ][ idx ]
                  }
                  s += diff * coeff
              }
              if (plane > 0) {
                  mLuma = get_luma_sample(x + c, y + r)
                  for ( i = 0; i < nTaps; i++ ) {
                       if ( frame_filters_on[ plane ] ) {
                           coeff = FrameLrWienerNs[ plane ][ 0 ][ i + 6 ]
                       } else {
                           coeff = LrWienerNs[ plane ][ unitRow ][ unitCol ][ i + 6 ]
                       }
                       if ( coeff != 0 ) {
                           dy = config[ i ][ 0 ]
                           dx = config[ i ][ 1 ]
                           lumaDiff = get_luma_sample( x + c + dx, y + r + dy ) - mLuma
                           s += lumaDiff * coeff
                       }
                  }



    AV2 Specification                                                                               Page 553 of 1169
           }
           v = Round2( s, WIENER_NS_PREC_BITS )
           LrFrame[ plane ][ y + r ][ x + c ] = Clip1( v )
      }
 }


The function calls to get_source_sample indicate that the get source sample process specified in § 7.20.2
Get source sample process is invoked.

The constant tables Wiener_Ns_Config_Y and Wiener_Ns_Config_Uv are defined as:

 Wiener_Ns_Config_Y[WIENER_NS_TAPS_Y][3] = {
     { 1, 0, 0 }, { -1, 0, 0 }, { 0, 1, 1 },      { 0, -1, 1 }, { 2, 0, 2 },
     { -2, 0, 2 }, { 0, 2, 3 },   { 0, -2, 3 }, { 1, 1, 4 },     { -1, -1, 4 },
     { -1, 1, 5 }, { 1, -1, 5 }, { 2, 1, 6 },     { -2, -1, 6 }, { 2, -1, 7 },
     { -2, 1, 7 }, { 1, 2, 8 },   { -1, -2, 8 }, { 1, -2, 9 }, { -1, 2, 9 },
     { 3, 0, 10 }, { -3, 0, 10 }, { 0, 3, 11 }, { 0, -3, 11 },
     { 4, 0, 12 }, { -4, 0, 12 }, { 0, 4, 13 }, { 0, -4, 13 }, { 3, 3, 14 },
     { -3, -3, 14 }, { 3, -3, 15 }, { -3, 3, 15 }
 }

 Wiener_Ns_Config_Uv[WIENER_NS_TAPS_UV][3] = {
     { 1, 0, 0 }, { -1, 0, 0 }, { 0, 1, 1 }, { 0, -1, 1 },
     { 1, 1, 2 }, { -1, -1, 2 }, { -1, 1, 3 }, { 1, -1, 3 },
     { 2, 0, 4 }, { -2, 0, 4 }, { 0, 2, 5 }, { 0, -2, 5 }
 }


The function get_luma_sample gets a filtered sample from luma as follows:

 get_luma_sample(x, y) {
     subX = SubsamplingX
     subY = SubsamplingY
     lastY = MiRows * MI_SIZE - 1 - subY
     lastX = LumaEndX - subX
     x = x << subX
     y = y << subY
     y = Clip3( 0, lastY, y )
     x = Clip3(LumaStartX, lastX, x)
     filterIdx = cfl_ds_filter_index
     if (filterIdx == 3) {
         filterIdx = 0
     }
     if (subX && subY && filterIdx <= 1) {
         t = 0
         for (dy = 0; dy < 2; dy++) {
              for (dx = 0; dx < 2; dx++) {
                  v = get_luma_source_sample(x + dx, y + dy)
                  t += Wiener_Filters_420[filterIdx][dy][dx] * v
              }
         }
         return t >> 2
     } else {
         return get_luma_source_sample(x, y)
     }
 }


The constant table Wiener_Filters_420 is specified as:

 Wiener_Filters_420[2][2][2] = {
     {
         {1, 1},
         {1, 1}
     },




AV2 Specification                                                                           Page 554 of 1169
           {
               {2, 0},
               {2, 0}
           }
     }


    The function get_luma_source_sample gets a sample from the luma stripe as follows:

     get_luma_source_sample( x, y ) {
         return get_source_sample( 0, x, y )
     }


```

<a id="s-7-20-4"></a>

#### § 7.20.4 Pixel classified Wiener filter process

```text
§   7.20.4. Pixel classified Wiener filter process

    The inputs to this process are:

         • variables x and y specifying the position of the block in samples relative to the top-left corner of the
           current plane,
         • variables w and h specifying the size of the block in samples,
         • a variable skipFilter specifying whether to only apply the pixel classification.

    The output from this process are modified luma samples in LrFrame.

    The variable BlockStartX (containing the start x location rounded to units of 64 by 64 luma samples) is
    set equal to (x >> 6) << 6.

    The variable BlockEndX (containing the last on-screen x location in the current 64x64) is set equal to
    Min(MiColEnd * MI_SIZE - 1, BlockStartX + 63).


    The variable qindex is set equal to base_q_idx.

    The variable index is set equal to get_filter_set_index(qindex).

    The variable cls (representing the pixel class) is computed as follows:

     (f, tskip) = get_box_features( x, y )
     lutInput = 0
     for (i = 0; i < PC_WIENER_NUM_FEATURES; i++) {
         qval = Round2Signed( f[i] + get_qval_given_tskip(qindex, tskip, i),
                              PC_WIENER_PREC_FEATURE)
         qval = Clip3(0,255,qval) >> 5
         lutInput += qval << (3 * (3-i))
     }
     cls = Pc_Wiener_Lut_To_Class[ lutInput ]


    If skipFilter is equal to 1, the class is saved by setting FilterClass[ y >> 2 ][ x >> 2 ] equal to cls, and the
    process immediately terminates.

    Otherwise (skipFilter is equal to 0), the filtering is applied as follows:

     filt = Pc_Wiener_Sub_Classify[ index ][ cls ]
     for ( r2 = 0; r2 < h; r2++ ) {
         for ( c2 =0; c2 < w; c2++ ) {
             m = get_source_sample(0, x + c2, y + r2)
             s = m << PC_WIENER_PREC_BITS
             for ( i = 0; i < PC_WIENER_TAPS; i++ ) {



    AV2 Specification                                                                                  Page 555 of 1169
                coeff = Pc_Wiener_Filters[ index ][ filt ][ i >> 1 ]
                s += get_pc_wiener_sample( x + c2 , y + r2, i ) * coeff
           }
           v = Round2( s, PC_WIENER_PREC_BITS )
           LrFrame[ 0 ][ y + r2 ][ x + c2 ] = Clip1( v )
      }
 }


The functions get_pc_wiener_sample, get_box_features, and get_qval_given_tskip are defined as follows:

 Pc_Wiener_Config[ PC_WIENER_TAPS ][ 2 ] = {
     { 1, 0 }, { -1, 0 }, { 0, 1 }, { 0, -1 }, { 2, 0 },
     { -2, 0 }, { 0, 2 }, { 0, -2 }, { 1, 1 }, { -1, -1 },
     { -1, 1 }, { 1, -1 }, { 2, 1 }, { -2, -1 }, { 2, -1 },
     { -2, 1 }, { 1, 2 }, { -1, -2 }, { 1, -2 }, { -1, 2 },
     { 3, 0 }, { -3, 0 }, { 0, 3 }, { 0, -3 }, { 0, 0 }
 }

 get_pc_wiener_sample( x, y, i ) {
     dy = Pc_Wiener_Config[i][0]
     dx = Pc_Wiener_Config[i][1]
     return get_source_sample(0, x + dx, y + dy)
 }

 Pc_Wiener_Normalizer[ PC_WIENER_NUM_FEATURES + 1 ] = {
     0,3739,3273,3074,7
 }

 get_box_features(x, y) {
     for( i = 0; i < PC_WIENER_NUM_FEATURES; i++) {
         f[i] = 0
     }
     s = 0
     for(dy = -PC_WIENER_LEAD; dy <= PC_WIENER_LAG; dy++) {
         for(dx = -PC_WIENER_LEAD; dx <= PC_WIENER_LAG; dx++) {
             (tf, skip) = get_features(x + dx, y + dy)
             for( i = 0; i < PC_WIENER_NUM_FEATURES; i++ ) {
                  f[i] += tf[i]
             }
             s += skip
         }
     }
     for(i = 0; i < PC_WIENER_NUM_FEATURES; i++) {
         nf[i] = Round2( f[i] * Pc_Wiener_Normalizer[i], BitDepth - 8 )
     }
     ns = s * Pc_Wiener_Normalizer[ PC_WIENER_NUM_FEATURES ]
     return (nf, ns)
 }

 get_features(x, y) {
     x = Min(BlockEndX + 2, x)

      m = get_source_sample(0, x, y)

      up = get_source_sample(0, x, y - 1)
      down = get_source_sample(0, x, y + 1)
      vert = up - 2 * m + down

      upright = get_source_sample(0, x + 1, y - 1)
      downleft = get_source_sample(0, x - 1, y + 1)
      antiDiag = upright - 2 * m + downleft

      downright = get_source_sample(0, x + 1, y + 1)
      upleft = get_source_sample(0, x - 1, y - 1)
      diag = upleft - 2 * m + downright

      f[0] = 0
      f[1] = Abs(vert)
      f[2] = Abs(antiDiag)



AV2 Specification                                                                         Page 556 of 1169
          f[3] = Abs(diag)
          return (f, get_tx_skip(x, y))
     }

     get_tx_skip( x, y ) {
         x = Min( BlockEndX, x )
         x = Max( BlockStartX, x )
         y = Clip3( LumaStripeStartY, LumaStripeEndY, y )
         tileStartY = MiRowStart * MI_SIZE
         tileEndY = MiRowEnd * MI_SIZE - 1
         y = Clip3( tileStartY, tileEndY, y)
         return LrTxSkip[ y >> 2 ][ x >> 2 ]
     }

     Mode_Weights[ PC_WIENER_NUM_FEATURES ][ 3 ] = {
         { -527, 15325, 321 },
         { 26436, -17705, 17905 },
         { 366, -147, -194 },
         { 202, -267, -179 }
     }

     Mode_Offsets[ PC_WIENER_NUM_FEATURES ] = {
         -547, -21565, -573, -680
     }

     get_qval_given_tskip(qindex, tskip, i) {
         qstep = get_q(qindex, 0)
         qstepShift = QUANT_TABLE_BITS + 10
         qstep = Round2(qstep, BitDepth - 8)
         diffShift = qstepShift - 8
         prod = Round2(tskip * qstep, 8)
         qval = (Mode_Weights[ i ][ 0 ] * (tskip << diffShift)) +
                 (Mode_Weights[ i ][ 1 ] * qstep) +
                 (Mode_Weights[ i ][ 2 ] * prod)
         return 255 * ( Mode_Offsets[ i ] + Round2Signed(qval, qstepShift) )
     }



      NOTE: Pc_Wiener_Normalizer[ 0 ] is equal to 0, so the value of the first feature does not influence
      the decoding process.

```

<a id="s-7-20-5"></a>

#### § 7.20.5 Apply GDF filter process

```text
§   7.20.5. Apply GDF filter process

    The inputs to this process are:

      • variables x and y specifying the location of the top-left luma sample in a GDF unit,
      • variables qpIdx and refDstIdx specifying which set of tables are active,
      • variables w and h specifying the size of the GDF unit,
      • a variable stripeEndY specifying the unclipped end of the current 64 pixel high stripe.

    The curvature in different directions is estimated in the array grad as follows:

     for( i = 0; i < h + 2; i++ ) {
         for( j = 0; j < w + 2; j++ ) {
             for( d = 0; d < 4; d++ ) {
                 if (d == GDF_VER) {
                     dx = 0
                     dy = 1
                 } else if (d == GDF_HOR) {
                     dx = 1
                     dy = 0
                 } else if (d == GDF_DIAG0) {
                     dx = 1



    AV2 Specification                                                                             Page 557 of 1169
                    dy = 1
                } else {
                    dx = 1
                    dy = -1
                }
                a = get_gdf_sample( x - 1 + j - dx, y - 1 + i - dy )
                b = get_gdf_sample( x - 1 + j, y - 1 + i )
                c = get_gdf_sample( x - 1 + j + dx, y - 1 + i + dy )
                grad[d][i][j] = Abs( b * 2 - a - c )
           }
      }
 }


where the function get_gdf_sample (which gets a sample from the current stripe with reflection at the
stripe end) is specified as:

 get_gdf_sample( x, y ) {
     return get_luma_source_sample(x,y)
 }


The array gdfCls (containing the filter class for each sample) is derived as follows:

 for ( i = (h >> 1) - 1; i >= 0; i--) {
     for( j = 0; j < w >> 1 ;j++) {
         for( d = 0; d < 4; d++ ) {
             str[ d ] = grad_sum(grad[d],i*2,j*2,4,4)
         }
         cls = str[GDF_VER] > str[GDF_HOR] ? 0 : 1
         cls |= str[GDF_DIAG0] > str[GDF_DIAG1] ? 0 : 2
         gdfCls[ i ][ j ] = cls
     }
 }


The function grad_sum sums a rectangle of the values in an array as follows:

 grad_sum(grad,i,j,down,across) {
     t = 0
     for( i2 = 0; i2 < down; i2++ ) {
         for( j2 = 0; j2 < across; j2++ ) {
              t += grad[i + i2][j + j2]
         }
     }
     return t
 }



  NOTE:        The array grad contains values representable by an unsigned integer with BitDepth + 1 bits.
  grad_sum sums 16 values within grad. This means grad_sum returns values representable by an unsigned
  integer with BitDepth + 5 bits.


The scaling used for this unit is prepared as follows:

 if ( refDstIdx == 0 ) {
     scale = 8
 } else {
     scale = 5
 }




AV2 Specification                                                                               Page 558 of 1169
The luma samples in LrFrame are modified as follows:

 for( i = 0; i < h; i++ ) {
     y2 = i + y
     for ( j = 0; j < w; j++ ) {
         x2 = x + j
         cls = gdfCls[i >> 1][j >> 1]
         for( idx = 0; idx < 3; idx++ ) {
             gdfIdx[ idx ] = 0
         }
         for( k = 0; k < 18 + 4; k++ ) {
             alpha = Gdf_Alpha[ refDstIdx ][ qpIdx ][ k ][ cls ]
             if ( k < 18 ) {
                  dy = Gdf_Coords[k][0]
                  dx = Gdf_Coords[k][1]
                  x3 = x2 - dx
                  y3 = y2 - dy
                  x4 = x2 + dx
                  y4 = y2 + dy
                  sample2 = get_gdf_sample(x2,y2)
                  sample3 = get_gdf_sample(x3,y3)
                  sample4 = get_gdf_sample(x4,y4)
                  above = Clip3( -alpha, alpha,
                                  ( sample3 - sample2) <<
                                      (10 - Min( 10, BitDepth) ) )
                  below = Clip3( -alpha, alpha,
                                  ( sample4 - sample2 ) <<
                                      (10 - Min( 10, BitDepth) ) )
                  comb = Clip3( -512, 511, above + below )
             } else {
                  d = k - 18
                  v = grad_sum(grad[d],(i>>1)<<1,(j>>1)<<1,4,4)
                  if ( BitDepth == 8 ) {
                      v = v >> 2
                  } else {
                      v = v >> 4
                  }
                  comb = Min( v, alpha )
             }
             for( idx = 0; idx < 3; idx++ ) {
                  gdfIdx[ idx ] +=
                      comb * Gdf_Weight[ refDstIdx ][ qpIdx ][ idx ][ k ][ cls ]
             }
         }
         pos = 0
         for( idx = 0; idx < 3; idx++ ) {
             v = Round2Signed(
                      ( gdfIdx[ idx ] + Gdf_Bias[ refDstIdx ][ qpIdx ][ idx ] ) *
                      scale, 15 )
             pos = pos * scale * 2 + Clip3( -scale, scale - 1, v ) + scale
         }
         if ( refDstIdx == 0 ) {
             err = Gdf_Intra_Error[ qpIdx ][ pos ]
         } else {
             err = Gdf_Inter_Error[ refDstIdx - 1 ][ qpIdx ][ pos ]
         }
         res = Clip1( LrFrame[ 0 ][ y2 ][ x2 ] +
                       Round2Signed( err * GdfPixScale,12 - BitDepth ) )
         LrFrame[ 0 ][ y2 ][ x2 ] = res
     }
 }


where the constant table Gdf_Coords is specified as:

 Gdf_Coords[18][2] = {
                                   { 6,   0},
                                   { 5,   0},



AV2 Specification                                                                   Page 559 of 1169
                                           { 4,   0},
                                           { 3,   0},
                                { 2,   1}, { 2,   0}, { 2, -1},
                     { 1,   2}, { 1,   1}, { 1,   0}, { 1, -1}, { 1, -2},
          { 0,   6}, { 0,   5}, { 0,   4}, { 0,   3}, { 0, 2}, { 0, 1}
     }


```

<a id="s-7-21"></a>

### § 7.21 Output processes

```text
§   7.21. Output processes
```

<a id="s-7-21-1"></a>

#### § 7.21.1 Output process

```text
§   7.21.1. Output process

    The input to this process is a variable frameToShowMapIdx specifying which frame to output. If
    frameToShowMapIdx is equal to -1, the process will output the current frame. Otherwise,
    frameToShowMapIdx indicates which previously decoded frame to output.

    This process is invoked to prepare output frames.

    The variable mixedOutput is set equal to frameToShowMapIdx == -1 && ShowExistingFrame.

    If mixedOutput is equal to 1, frameToShowMapIdx is set equal to frame_to_show_map_idx.

    If scalability is being used (bitstream contains OBUs with different values of obu_xlayer_id,
    obu_mlayer_id, or obu_tlayer_id), an application-specific function is called to decide whether this frame
    will be output. If this function returns a value equal to 0, then this process terminates immediately.

    Applications that are displaying the decoded video should determine which frames to display based on
    the layer properties specified in the LCR OBUs, when present. The decision should consider:

      • lcr_layer_type: Distinguishes between texture layers (TEXTURE_LAYER) and auxiliary layers
        (AUX_LAYER)
      • lcr_auxiliary_type: For auxiliary layers, specifies the type (e.g., ALPHA_AUX, DEPTH_AUX)
      • lcr_global_purpose_id and lcr_xlayer_purpose_id: Indicate the application purpose for the layered
        bitstream (e.g., stereoscopic viewports, immersive multiple viewports)

    Typically, applications displaying decoded video will output texture layers (lcr_layer_type ==
    TEXTURE_LAYER) while using auxiliary layers (lcr_layer_type == AUX_LAYER) for purposes such as
    transparency (alpha) or depth information, according to the indicated purpose. Applications may set their
    own policy about which frames and layers are output based on their specific use case and the LCR layer
    properties.

    The intermediate output preparation process specified in § 7.21.2 Intermediate output preparation
    process is invoked with mixedOutput and frameToShowMapIdx as inputs, and the outputs are assigned to
    bitDepth, w, h, subX, subY, filmGrainPresent, and numPlanes.

    If filmGrainPresent is equal to 1 and apply_grain is equal to 1, then the film grain synthesis process
    specified in § 7.21.7 Film grain synthesis process is invoked with inputs of w, h, subX, subY, bitDepth, and
    numPlanes. (This process modifies the output arrays OutY, OutU, OutV).

    Finally, the frame to be output is defined to be the arrays OutY, OutU, OutV where the bit depth for each
    sample is bitDepth.

    This frame to be output is the overall output of the decoding process and further processing (such as
    color conversion) is outside the scope of this specification.


    AV2 Specification                                                                             Page 560 of 1169
    For example, a real implementation might use these arrays to display the frame to the user, or a test
    system might save the arrays so the output can be verified.


      NOTE:       If numPlanes is equal to 1, then the U and V planes are ignored.

```

<a id="s-7-21-2"></a>

#### § 7.21.2 Intermediate output preparation process

```text
§   7.21.2. Intermediate output preparation process

    The inputs to this process are:

      • a variable mixedOutput specifying the source for the film grain parameters,
      • a variable frameToShowMapIdx specifying which frame to output.

    The outputs of this process are the variables bitDepth, w, h, subX, subY, filmGrainPresent, and numPlanes
    describing the format of the data in arrays OutY, OutU, and OutV.

    If frameToShowMapIdx is greater than or equal to 0, then the decoder sets variables and copies OutY,
    OutU, and OutV from a previously decoded frame as follows:

      • The variable w is set equal to RefCropWidth[ frameToShowMapIdx ].
      • The variable h is set equal to RefCropHeight[ frameToShowMapIdx ].
      • The variable left is set equal to RefCropLeft[ frameToShowMapIdx ].
      • The variable top is set equal to RefCropTop[ frameToShowMapIdx ].
      • The variable subX is set equal to RefSubsamplingX[ frameToShowMapIdx ].
      • The variable subY is set equal to RefSubsamplingY[ frameToShowMapIdx ].
      • The array OutY is w samples across by h samples down and the sample at location x samples across
        and y samples down is given by OutY[ y ][ x ] = FrameStore[ frameToShowMapIdx ][ 0 ][ y + top ][ x +
        left ] with x = 0..w - 1 and y = 0..h - 1.

      • The array OutU is (w + subX) >> subX samples across by (h + subY) >> subY samples down and the
        sample at location x samples across and y samples down is given by OutU[ y ][ x ] =
        FrameStore[ frameToShowMapIdx ][ 1 ][ y + (top >> subY) ][ x + (left >> subX) ] with x = 0..((w + subX) >>
        subX) - 1 and y = 0..((h + subY) >> subY) - 1.

      • The array OutV is (w + subX) >> subX samples across by (h + subY) >> subY samples down and the
        sample at location x samples across and y samples down is given by OutV[ y ][ x ] =
        FrameStore[ frameToShowMapIdx ][ 2 ][ y + (top >> subY) ][ x + (left >> subX) ] with x = 0..((w + subX) >>
        subX) - 1 and y = 0..((h + subY) >> subY) - 1.

      • The variable bitDepth is set equal to RefBitDepth[ frameToShowMapIdx ].
      • The variable numPlanes is set equal to RefNumPlanes[ frameToShowMapIdx ].
      • The variable filmGrainPresent is set equal to RefFilmGrainPresent[ frameToShowMapIdx ].
      • If filmGrainPresent is equal to 1, the function load_grain_params is invoked with mixedOutput ?
        NUM_REF_FRAMES : frameToShowMapIdx as input.


    Otherwise (frameToShowMapIdx is equal to -1), then the decoder sets variables and copies the current
    frame as follows:

      • The variable w is set equal to CropWidth.



    AV2 Specification                                                                                Page 561 of 1169
      • The variable h is set equal to CropHeight.
      • The variable subX is set equal to SubsamplingX.
      • The variable subY is set equal to SubsamplingY.
      • The array OutY is w samples across by h samples down and the sample at location x samples across
        and y samples down is given by OutY[ y ][ x ] = LrFrame[ 0 ][ y + CropTop ][ x + CropLeft ] with x =
        0..w - 1 and y = 0..h - 1.
      • The array OutU is (w + subX) >> subX samples across by (h + subY) >> subY samples down and the
        sample at location x samples across and y samples down is given by OutU[ y ][ x ] = LrFrame[ 1 ][ y +
        (CropTop >> subY) ][ x + (CropLeft >> subX) ] with x = 0..((w + subX) >> subX) - 1 and y = 0..((h + subY)
        >> subY) - 1.

      • The array OutV is (w + subX) >> subX samples across by (h + subY) >> subY samples down and the
        sample at location x samples across and y samples down is given by OutV[ y ][ x ] = LrFrame[ 2 ][ y +
        (CropTop >> subY) ][ x + (CropLeft >> subX) ] with x = 0..((w + subX) >> subX) - 1 and y = 0..((h + subY)
        >> subY) - 1.

      • The variable bitDepth is set equal to BitDepth.
      • The variable numPlanes is set equal to NumPlanes.
      • The variable filmGrainPresent is set equal to film_grain_params_present.
      • If filmGrainPresent is equal to 1, the function load_grain_params is invoked with NUM_REF_FRAMES
        as input.

    The function load_grain_params(idx) indicates that all the syntax elements read in both film_grain_model
    and film_grain_config should be set equal to the values stored in an area of memory indexed by idx.

    The output of this process are the variables bitDepth, w, h, subX, subY, filmGrainPresent, and numPlanes.

```

<a id="s-7-21-3"></a>

#### § 7.21.3 Output successive frames process

```text
§   7.21.3. Output successive frames process

    The input to this process is a variable orderHint specifying the order hint (with additional bits for the
    embedded layer) for the current frame.

    This process outputs additional frame buffers if they have successive order hints.

    The variable k is set equal to 1.

    While k is less than or equal to NumRefFrames, the following ordered steps apply:

     1. The output implicit output frame process specified in § 7.21.4 Output implicit output frame process is
        invoked with orderHint + k as input, and the output is assigned to the variable madeOutput.
     2. If madeOutput is equal to 0, the process immediately terminates.
     3. The variable k is incremented by 1.

```

<a id="s-7-21-4"></a>

#### § 7.21.4 Output implicit output frame process

```text
§   7.21.4. Output implicit output frame process

    The input to this process is the variable targetHint.




    AV2 Specification                                                                               Page 562 of 1169
    The process examines the frames in the frame buffer and outputs any implicit output frames that match
    the target order hint as follows:

     madeOutput = 0
     for( i = 0; i < NumRefFrames; i++ ) {
         if ( output_ordering(i) == targetHint &&
              is_frame_eligible_for_output(i) ) {
             output_process( i )
             madeOutput = 1
         }
     }


    where output_process( i ) denotes an invocation of the output process specified in § 7.21.1 Output process
    with frameToShowMapIdx equal to i.

    The function is_frame_eligible_for_output(refIdx) is specified as follows:

      • RefImplicitOutputFrame[ refIdx ] has been written and is equal to 1 and RefValid[ refIdx ] is equal to
        1 and the frame has not already been output by the output process specified in § 7.21.1 Output
        process and RefOrderHint[ refIdx ] is not equal to RESTRICTED_OH, the function returns 1.
      • Otherwise (RefImplicitOutputFrame[ refIdx ] is equal to 0, or RefValid[ refIdx ] is equal to 0, or the
        frame has already been output, or RefOrderHint[ refIdx ] is equal to RESTRICTED_OH), the function
        returns 0.

    However, when considering whether a frame has been output by the output process, invocations of the
    output process with frameToShowMapIdx less than 0 and ShowExistingFrame equal to 1 are ignored.


      NOTE: This requirement means that a frame can be shown with a specified order hint without
      affecting the normal output of that frame.


      NOTE: The requirement that RefImplicitOutputFrame[ refIdx ] has been written prevents the use of
      uninitialized frame buffers when the first keyframe is decoded. This may also be implemented by
      initializing the array RefImplicitOutputFrame to 0 before decoding starts. However, note that later
      key frames in a video may trigger the output of frames.


      NOTE: Even if a frame is stored into multiple reference frame buffers, it is still only eligible to be
      output once.


    The output of this process is the variable madeOutput indicating if a matching frame was output.

```

<a id="s-7-21-5"></a>

#### § 7.21.5 Flush implicit output frames process

```text
§   7.21.5. Flush implicit output frames process

    The input to this process is a variable olkLimit (that limits the range of flushed frames).

    This process is invoked after all other OBUs have been decoded and outputs all remaining eligible
    frames.




    AV2 Specification                                                                             Page 563 of 1169
    An eligible frame is found as follows:

     outputHint = -1
     outIdx = -1
     outputLayer = -1
     outputOrder = -1
     for (i = 0; i < NUM_REF_FRAMES; i++) {
         if ( is_frame_eligible_for_output( i ) &&
              ( outIdx == -1 || RefOutputOrder[i] <= outputOrder ) &&
              !( olkLimit && RefOrderHint[ i ] >= OlkTUOrderHint ) ) {
             outIdx = i
             outputHint = RefOrderHint[i]
             outputLayer = RefMLayerId[i]
             outputOrder = RefOutputOrder[i]
         }
     }


    If outIdx is equal to -1, this process immediately terminates.

    The output process specified in § 7.21.1 Output process is invoked with outIdx as input.

    This entire process is then repeated until the termination condition is reached.

```

<a id="s-7-21-6"></a>

#### § 7.21.6 Output frame buffers process

```text
§   7.21.6. Output frame buffers process

    The input to this process is a variable refIdx. If refIdx is greater than or equal to 0, refIdx specifies which
    reference frame buffer to output. If refIdx is equal to -1, it indicates that the current frame is output.

    First any eligible frames with lower order hints are output as follows:

     while(1) {
         outputHint = output_ordering( refIdx )
         outIdx = refIdx
         for (i = 0; i < NumRefFrames; i++) {
             if ( is_frame_eligible_for_output(i) &&
                   output_ordering(i) < outputHint ) {
                  outIdx = i
                  outputHint = output_ordering(i)
             }
         }
         if (outIdx == refIdx) {
             break
         } else {
             output_process(outIdx)
         }
     }


    where output_process( outIdx ) denotes an invocation of the output process specified in § 7.21.1 Output
    process with frameToShowMapIdx equal to outIdx.

    The function output_ordering (which returns an order hint with additional bits specifying the embedded
    layer) is specified as:

     output_ordering( i ) {
         if ( i < 0 ) {
             return OrderHint * (max_mlayer_id + 1) + obu_mlayer_id
         }
         return RefOrderHint[i] * (max_mlayer_id + 1) + RefMLayerId[i]
     }




    AV2 Specification                                                                                Page 564 of 1169
    The output process specified in § 7.21.1 Output process is invoked with refIdx as input.

    The output successive frames process specified in § 7.21.3 Output successive frames process is invoked
    with outputHint as input.

```

<a id="s-7-21-7"></a>

#### § 7.21.7 Film grain synthesis process

```text
§   7.21.7. Film grain synthesis process

```

<a id="s-7-21-7-1"></a>

##### § 7.21.7.1 General

```text
§   7.21.7.1. General

    The inputs to this process are:

      • variables w and h specifying the width and height of the frame,
      • variables subX and subY specifying the subsampling parameters of the frame,
      • a variable bitDepth specifying the number of bits per sample,
      • a variable numPlanes specifying the number of planes in the frame.

    The process modifies the arrays OutY, OutU, OutV to add film grain noise by the following ordered steps:

     1. The variable RandomRegister (used for generating pseudo-random numbers) is set equal to
        grain_seed.
     2. The variable GrainMin is set equal to -(1 << (bitDepth - 1)).
     3. The variable GrainMax is set equal to (1 << (bitDepth - 1)) - 1.
     4. The generate grain process specified in § 7.21.7.3 Generate grain process is invoked with subX, subY,
        and bitDepth as input.
     5. The scaling lookup initialization process specified in § 7.21.7.4 Scaling lookup initialization process is
        invoked with numPlanes as input.
     6. The add noise process specified in § 7.21.7.5 Add noise synthesis process is invoked with w, h, subX,
        subY, bitDepth, and numPlanes as inputs.

```

<a id="s-7-21-7-2"></a>

##### § 7.21.7.2 Random number process

```text
§   7.21.7.2. Random number process

    The input to this process is a variable bits specifying the number of random bits to return.

    The output of this process is a pseudo-random number based on the state in RandomRegister.

    The process is specified as follows:

     get_random_number( bits ) {
       r = RandomRegister
       bit = ((r >> 0) ^ (r >> 1) ^ (r >> 3) ^ (r >> 12)) & 1
       r = (r >> 1) | (bit << 15)
       result = (r >> (16 - bits)) & ((1 << bits) - 1)
       RandomRegister = r
       return result
     }


    The output of this process is the variable result.




    AV2 Specification                                                                               Page 565 of 1169
```

<a id="s-7-21-7-3"></a>

##### § 7.21.7.3 Generate grain process

```text
§   7.21.7.3. Generate grain process

    The inputs to this process are:

      • variables subX and subY specifying the subsampling parameters of the frame,
      • a variable bitDepth specifying the number of bits per sample.

    This process generates noise via an auto-regressive filter.

    First an array LumaGrain 82 samples wide and 73 samples high of white noise is generated for luma as
    follows:

     shift = 12 - bitDepth + grain_scale_shift
     for ( y = 0; y < 73; y++ ) {
       for ( x = 0; x < 82; x++ ) {
         if ( num_y_points > 0 ) {
           g = Gaussian_Sequence[ get_random_number( 11 ) ]
         } else {
           g = 0
         }
         LumaGrain[ y ][ x ] = Round2( g, shift )
       }
     }


    where the function call get_random_number invokes the random number process specified in § 7.21.7.2
    Random number process.

    Then an auto-regressive filter is applied to the white noise as follows:

     shift = ar_coeff_shift_minus_6 + 6
     for ( y = 3; y < 73; y++ ) {
       for ( x = 3; x < 82 - 3; x++ ) {
         s = 0
         pos = 0
         for ( deltaRow = -ar_coeff_lag; deltaRow <= 0; deltaRow++ ) {
           for ( deltaCol = -ar_coeff_lag; deltaCol <= ar_coeff_lag; deltaCol++ ) {
             if ( deltaRow == 0 && deltaCol == 0 )
               break
             c = ar_coeffs_y[ pos ]
             s += LumaGrain[ y + deltaRow ][ x + deltaCol ] * c
             pos++
           }
         }
         LumaGrain[ y ][ x ] = Clip3( GrainMin, GrainMax,
                                      LumaGrain[ y ][ x ] + Round2( s, shift ) )
       }
     }


    The variable chromaW (representing the width of the chroma noise array) is set equal to (subX ? 44 : 82).

    The variable chromaH (representing the height of the chroma noise array) is set equal to (subY ? 38 : 73).

    White noise arrays CbGrain and CrGrain chromaW samples wide and chromaH samples high are
    generated as follows:

     shift = 12 - bitDepth + grain_scale_shift
     RandomRegister = grain_seed ^ 0xb524
     for ( y = 0; y < chromaH; y++ ) {
       for ( x = 0; x < chromaW; x++ ) {




    AV2 Specification                                                                           Page 566 of 1169
      if ( num_cb_points > 0 || chroma_scaling_from_luma) {
        g = Gaussian_Sequence[ get_random_number( 11 ) ]
      } else {
        g = 0
      }
      CbGrain[ y ][ x ] = Round2( g, shift )
   }
 }
 RandomRegister = grain_seed ^ 0x49d8
 for ( y = 0; y < chromaH; y++ ) {
   for ( x = 0; x < chromaW; x++ ) {
     if ( num_cr_points > 0 || chroma_scaling_from_luma) {
       g = Gaussian_Sequence[ get_random_number( 11 ) ]
     } else {
       g = 0
     }
     CrGrain[ y ][ x ] = Round2( g, shift )
   }
 }


Then the auto-regressive filter is applied as follows:

 shift = ar_coeff_shift_minus_6 + 6
 for ( y = 3; y < chromaH; y++ ) {
   for ( x = 3; x < chromaW - 3; x++ ) {
     s0 = 0
     s1 = 0
     pos = 0
     for ( deltaRow = -ar_coeff_lag; deltaRow <= 0; deltaRow++ ) {
       for ( deltaCol = -ar_coeff_lag; deltaCol <= ar_coeff_lag; deltaCol++ ) {
         c0 = ar_coeffs_cb[ pos ]
         c1 = ar_coeffs_cr[ pos ]
         if ( deltaRow == 0 && deltaCol == 0 ) {
            if ( num_y_points > 0 ) {
              luma = 0
              lumaX = ( (x - 3) << subX ) + 3
              lumaY = ( (y - 3) << subY ) + 3
              for ( i = 0; i <= subY; i++ )
                for ( j = 0; j <= subX; j++ )
                  luma += LumaGrain[ lumaY + i ][ lumaX + j ]
              luma = Round2( luma, subX + subY )
              s0 += luma * c0
              s1 += luma * c1
            }
            break
         }
         s0 += CbGrain[ y + deltaRow ][ x + deltaCol ] * c0
         s1 += CrGrain[ y + deltaRow ][ x + deltaCol ] * c1
         pos++
       }
     }
     CbGrain[ y ][ x ] = Clip3( GrainMin, GrainMax,
                                 CbGrain[ y ][ x ] + Round2( s0, shift ) )
     CrGrain[ y ][ x ] = Clip3( GrainMin, GrainMax,
                                 CrGrain[ y ][ x ] + Round2( s1, shift ) )
   }
 }



  NOTE: When num_y_points is equal to 0, this process may use uninitialized values within
  ar_coeffs_y to compute LumaGrain. However, LumaGrain will never be read in this case so it does not
  matter what values are constructed. Similarly, when num_cr_points/num_cb_points are equal to 0 and
  chroma_scaling_from_luma is equal to 0, the CbGrain/CrGrain arrays will never be read.




AV2 Specification                                                                        Page 567 of 1169
```

<a id="s-7-21-7-4"></a>

##### § 7.21.7.4 Scaling lookup initialization process

```text
§   7.21.7.4. Scaling lookup initialization process

    The input to this process is a variable numPlanes specifying the number of planes in the frame.

    This process computes 3 lookup tables for the different color components.

    Each lookup table ScalingLut[ plane ] contains 256 entries constructed by a piecewise linear
    interpolation of the given points as follows:

     for ( plane = 0; plane < numPlanes; plane++ ) {
         if ( plane == 0 || chroma_scaling_from_luma )
              numPoints = num_y_points
         else if ( plane == 1 )
              numPoints = num_cb_points
         else
              numPoints = num_cr_points
         if ( numPoints == 0 ) {
              for ( x = 0; x < 256; x++ ) {
                  ScalingLut[ plane ][ x ] = 0
              }
         } else {
              for ( x = 0; x < get_x( plane, 0 ); x++ ) {
                  ScalingLut[ plane ][ x ] = get_y( plane, 0 )
              }
              for ( i = 0; i < numPoints - 1; i++ ) {
                  deltaY = get_y( plane, i + 1 ) - get_y( plane, i )
                  deltaX = get_x( plane, i + 1 ) - get_x( plane, i )
                  delta = deltaY * ( ( 65536 + (deltaX >> 1) ) / deltaX )
                  for ( x = 0; x < deltaX; x++ ) {
                      v = get_y( plane, i ) + ( ( x * delta + 32768 ) >> 16 )
                      ScalingLut[ plane ][ get_x( plane, i ) + x ] = v
                  }
              }
              for ( x = get_x( plane, numPoints - 1 ); x < 256; x++ ) {
                  ScalingLut[ plane ][ x ] = get_y( plane, numPoints - 1 )
              }
         }
     }


    where the functions get_x and get_y return the coordinates for a specific point and are specified as:

     get_x( plane, i ) {
         if ( plane == 0 || chroma_scaling_from_luma )
              return point_y_value[ i ]
         else if ( plane == 1 )
              return point_cb_value[ i ]
         else
              return point_cr_value[ i ]
     }

     get_y( plane, i ) {
         if ( plane == 0 || chroma_scaling_from_luma )
              return point_y_scaling[ i ]
         else if ( plane == 1 )
              return point_cb_scaling[ i ]
         else
              return point_cr_scaling[ i ]
     }




    AV2 Specification                                                                             Page 568 of 1169
```

<a id="s-7-21-7-5"></a>

##### § 7.21.7.5 Add noise synthesis process

```text
§   7.21.7.5. Add noise synthesis process

    The inputs to this process are:

      • variables w and h specifying the width and height of the frame,
      • variables subX and subY specifying the subsampling parameters of the frame,
      • a variable bitDepth specifying the number of bits per sample,
      • a variable numPlanes specifying the number of planes in the frame.

    This process combines the film grain with the image data.

    First an array of noise data noiseStripe is generated for each 32 luma sample high stripe of the image.

    noiseStripe[ lumaNum ][ 0 ] is 34 samples high and w samples wide (a few additional samples across are
    actually written to the array, but these are never read) and contains noise for the luma component.

    noiseStripe[ lumaNum ][ 1 ] and noiseStripe[ lumaNum ][ 2 ] are (34 >> subY) samples high and Round2(w,
    subX) samples wide and contain noise for the chroma components.

    noiseStripe represents the result of constructing square grain blocks and blending horizontally adjacent
    blocks together (although blending is only applied if overlap_flag is equal to 1) and is constructed as
    follows:

     lumaSize = film_grain_block_size ? 32 : 16
     lumaNum = 0
     for ( y = 0; y < (h + 1)/2 ; y += (lumaSize >> 1) ) {
       RandomRegister = grain_seed
       lumaRand = y >> 3
       RandomRegister ^= ((lumaRand * 37 + 178) & 255) << 8
       RandomRegister ^= ((lumaRand * 173 + 105) & 255)
       for ( x = 0; x < (w + 1)/2 ; x += (lumaSize >> 1) ) {
         offsetY = get_random_number( 9 ) * (3 - film_grain_block_size) >> 6
         get_random_number( 1 )
         get_random_number( 1 )
         get_random_number( 1 )
         offsetX = get_random_number( 9 ) * (3 - film_grain_block_size) >> 6
         get_random_number( 1 )
         get_random_number( 1 )
         get_random_number( 1 )
         for ( plane = 0 ; plane < numPlanes; plane++ ) {
           planeSubX = ( plane > 0) ? subX : 0
           planeSubY = ( plane > 0) ? subY : 0
           planeOffsetX = planeSubX ? 6 + offsetX : 9 + offsetX * 2
           planeOffsetY = planeSubY ? 6 + offsetY : 9 + offsetY * 2
           for ( i = 0; i < (lumaSize + 2) >> planeSubY ; i++ ) {
             for ( j = 0; j < (lumaSize + 2) >> planeSubX ; j++ ) {
               if ( plane == 0 )
                 g = LumaGrain[ planeOffsetY + i ][ planeOffsetX + j ]
               else if ( plane == 1 )
                 g = CbGrain[ planeOffsetY + i ][ planeOffsetX + j ]
               else
                 g = CrGrain[ planeOffsetY + i ][ planeOffsetX + j ]
               if ( planeSubX == 0 ) {
                 if ( j < 2 && overlap_flag && x > 0 ) {
                    old = noiseStripe[ lumaNum ][ plane ][ i ][ x * 2 + j ]
                    if ( j == 0 ) {
                      g = old * 27 + g * 17
                    } else {
                      g = old * 17 + g * 27
                    }
                    g = Clip3( GrainMin, GrainMax, Round2(g, 5) )




    AV2 Specification                                                                           Page 569 of 1169
                   }
                   noiseStripe[ lumaNum ][ plane ][ i ][ x * 2 + j ] = g
                 } else {
                   if ( j == 0 && overlap_flag && x > 0 ) {
                     old = noiseStripe[ lumaNum ][ plane ][ i ][ x + j ]
                     g = old * 23 + g * 22
                     g = Clip3( GrainMin, GrainMax, Round2(g, 5) )
                   }
                   noiseStripe[ lumaNum ][ plane ][ i ][ x + j ] = g
                 }
             }
         }
       }
     }
     lumaNum++
 }


Then the noise stripes are blended together to form a noise image noiseImage as follows:

 for ( plane = 0; plane < numPlanes; plane++ ) {
   planeSubX = ( plane > 0) ? subX : 0
   planeSubY = ( plane > 0) ? subY : 0
   for ( y = 0; y < ( (h + planeSubY) >> planeSubY ) ; y++ ) {
     lumaNum = y >> ( 4 + film_grain_block_size - planeSubY )
     i = y - (lumaNum << ( 4 + film_grain_block_size - planeSubY ) )
     for ( x = 0; x < ( (w + planeSubX) >> planeSubX) ; x++ ) {
       g = noiseStripe[ lumaNum ][ plane ][ i ][ x ]
       if ( planeSubY == 0 ) {
         if ( i < 2 && lumaNum > 0 && overlap_flag ) {
           old = noiseStripe[ lumaNum - 1 ][ plane ][ i + lumaSize ][ x ]
           if ( i == 0 ) {
             g = old * 27 + g * 17
           } else {
             g = old * 17 + g * 27
           }
           g = Clip3( GrainMin, GrainMax, Round2(g, 5) )
         }
       } else {
         if ( i < 1 && lumaNum > 0 && overlap_flag ) {
           old = noiseStripe[ lumaNum - 1 ][ plane ][ i + (lumaSize >> 1) ][ x ]
           g = old * 23 + g * 22
           g = Clip3( GrainMin, GrainMax, Round2(g, 5) )
         }
       }
       noiseImage[ plane ][ y ][ x ] = g
     }
   }
 }



  NOTE: Although this process is specified in terms of full size noiseStripe and noiseImage arrays,
  the reference code shows how it is possible to implement the grain synthesis with just 2 line buffers
  for luma, and 1 line buffer for each chroma component.


Finally, the noise is blended with the original image data as follows:

 if ( clip_to_restricted_range ) {
   minValue = 16 << (bitDepth - 8)
   maxLuma = 235 << (bitDepth - 8)
   if ( fg_mc_identity )
     maxChroma = maxLuma
   else
     maxChroma = 240 << (bitDepth - 8)
 } else {
   minValue = 0



AV2 Specification                                                                            Page 570 of 1169
    maxLuma = (256 << (bitDepth - 8)) - 1
    maxChroma = maxLuma
 }
 ScalingShift = grain_scaling_minus_8 + 8
 for ( y = 0; y < ( (h + subY) >> subY) ; y++ ) {
   for ( x = 0; x < ( (w + subX) >> subX) ; x++ ) {
     lumaX = x << subX
     lumaY = y << subY
     lumaNextX = Min( lumaX + 1, w - 1 )
     if ( subX )
       averageLuma =
            Round2( OutY[ lumaY ][ lumaX ] + OutY[ lumaY ][ lumaNextX ], 1 )
     else
       averageLuma = OutY[ lumaY ][ lumaX ]
     if ( num_cb_points > 0 || chroma_scaling_from_luma ) {
       orig = OutU[ y ][ x ]
       if ( chroma_scaling_from_luma ) {
          merged = averageLuma
       } else {
          combined = averageLuma * ( cb_luma_mult - 128 ) +
                     orig * ( cb_mult - 128 )
          merged = Clip3( 0, (1 << bitDepth) - 1,
                          ( combined >> 6 ) +
                          ( (cb_offset - 256 ) << (bitDepth - 8) ) )
       }
       noise = noiseImage[ 1 ][ y ][ x ]
       noise = Round2( scale_lut( 1, merged, bitDepth ) * noise, ScalingShift )
       OutU[ y ][ x ] = Clip3( minValue, maxChroma, orig + noise )
     }

      if ( num_cr_points > 0 || chroma_scaling_from_luma) {
        orig = OutV[ y ][ x ]
        if ( chroma_scaling_from_luma ) {
          merged = averageLuma
        } else {
          combined = averageLuma * ( cr_luma_mult - 128 ) +
                     orig * ( cr_mult - 128 )
          merged = Clip3( 0, (1 << bitDepth) - 1, ( combined >> 6 ) +
                          ( (cr_offset - 256 ) << (bitDepth - 8) ) )
        }
        noise = noiseImage[ 2 ][ y ][ x ]
        noise = Round2( scale_lut( 2, merged, bitDepth ) * noise, ScalingShift )
        OutV[ y ][ x ] = Clip3( minValue, maxChroma, orig + noise )
      }
   }
 }
 for ( y = 0; y < h ; y++ ) {
   for ( x = 0; x < w ; x++ ) {
     orig = OutY[ y ][ x ]
     noise = noiseImage[ 0 ][ y ][ x ]
     noise = Round2( scale_lut( 0, orig, bitDepth ) * noise, ScalingShift )
     if ( num_y_points > 0 ) {
       OutY[ y ][ x ] = Clip3( minValue, maxLuma, orig + noise )
     }
   }
 }


where scale_lut is a function that performs a piecewise linear interpolation into the appropriate scaling
table. The scale_lut function is specified as follows:

 scale_lut( plane, index, bitDepth ) {
   shift = bitDepth - 8
   x = index >> shift
   rem = index - ( x << shift )
   if ( x == 255 ) {
     return ScalingLut[ plane ][ x ]
   } else {
     start = ScalingLut[ plane ][ x ]



AV2 Specification                                                                            Page 571 of 1169
             end = ScalingLut[ plane ][ x + 1 ]
             return start + Round2( (end - start) * rem, shift )
         }
     }


```

<a id="s-7-22"></a>

### § 7.22 Motion field motion vector storage process

```text
§   7.22. Motion field motion vector storage process
    The inputs to this process are:

      • variables r and c specifying the location of the block in units of 4x4 blocks in the luma plane,
      • a variable bSize specifying the size of the block,
      • a variable mvMethod that affects how the motion vector to be stored is computed.

    This process applies some filtering and reordering to the motion vectors to prepare them for storage as
    part of the reference frame update process.

    If enable_ref_frame_mvs is equal to 0, this process immediately terminates.

    The variables bw4, bh4 (describing the size of the block in units of 4x4 blocks in the luma plane), and n
    (specifying the size of the optical flow blocks within the block) are computed as follows:

     bw4 = Num_4x4_Blocks_Wide[ bSize ]
     bh4 = Num_4x4_Blocks_High[ bSize ]
     n = (bw4 <= 2 && bh4 <= 2 && TipFrameMode != TIP_FRAME_AS_OUTPUT) ? 4 : 8
     bw4 = Min(MiCols - c, bw4)
     bh4 = Min(MiRows - r, bh4)


    The variables isWedge (specifying if the block uses a wedge compound mode of two inter frames),
    refIdx0, refIdx1, and tipPred are computed as follows:

     refIdx0 = RefFrames[ r ][ c ][ 0 ]
     refIdx1 = RefFrames[ r ][ c ][ 1 ]
     isWedge = is_inter_ref_frame(refIdx0) && is_inter_ref_frame(refIdx1) &&
               refIdx0 != TIP_FRAME && compound_type == COMPOUND_WEDGE
     tipPred = refIdx0 == TIP_FRAME
     if (tipPred) {
         refIdx0 = ClosestPast
         refIdx1 = ClosestFuture
     }
     if ( (tipPred || TipFrameMode == TIP_FRAME_AS_OUTPUT) &&
          Tip_Weighting_Factor[ tip_global_wtd_index ] == 16 ) {
         refIdx1 = NONE
     }


    The following applies for i8 = 0..Round2(bh4,1)-1, for j8 = 0..Round2(bw4,1)-1:

     allowList[ 0 ] = 1
     allowList[ 1 ] = 1
     if (isWedge) {
         count0 = 0
         count1 = 0
         for ( i = 0; i < 8; i++ ) {
             for( j = 0; j < 8; j++) {
                 m = Mask[ i8 * 8 + i ][ j8 * 8 + j ]
                 if ( m > 60 )
                     count0++
                 if ( m < 4 )
                     count1++



    AV2 Specification                                                                              Page 572 of 1169
          }
      }
      if (count0 >= 60) {
          allowList[ 1 ] = 0
      } else if (count1 >= 60) {
          allowList[ 0 ] = 0
      }
 }

 x8 = (c >> 1) + j8
 y8 = (r >> 1) + i8
 row = r + (i8 << 1)
 col = c + (j8 << 1)
 for( list = 0;list < 2; list++ ) {
     refs[ list ] = NONE
     for( comp = 0; comp < 2; comp++ ) {
         mfmvs[ list ][ comp ] = 0
     }
 }
 for ( list = 0; list < 2; list++ ) {
     refIdx = list == 0 ? refIdx0 : refIdx1
     if ( is_inter_ref_frame(refIdx) ) {
         if ( mvMethod > 0 ) {
             mvs = ( use_refinemv || tipPred ) ?
                        RefineMvs[ i8 << 1 ][ j8 << 1 ] : Mvs[ row ][ col ]
             mvRow = mvs[list][0]
             mvCol = mvs[list][1]
             if ( mvMethod==1 ) {
                  if ( n==4 && !tipPred ) {
                      totalRow = 0
                      totalCol = 0
                      for(a=0;a<2;a++) {
                           for(b=0;b<2;b++) {
                               totalRow += MvDeltas[ a ][ b ][ list ][ 0 ]
                               totalCol += MvDeltas[ a ][ b ][ list ][ 1 ]
                           }
                      }
                      mvRow += Round2Signed(totalRow, 1 + 2)
                      mvCol += Round2Signed(totalCol, 1 + 2)
                  } else {
                      mvRow += Round2Signed(
                                    MvDeltas[ i8 << 1 ][ j8 << 1 ][ list ][ 0 ], 1)
                      mvCol += Round2Signed(
                                    MvDeltas[ i8 << 1 ][ j8 << 1 ][ list ][ 1 ], 1)
                  }
             }
         } else {
             if ( tipPred ) {
                  candMvs = get_tip_cand( row, col )
                  mv = candMvs[ list ]
             } else if ( motion_mode >= LOCALWARP && !force_integer_mv ) {
                  mv = get_sub_block_warp_mv( LocalWarpParams[ list ], 0,
                                               col * MI_SIZE, row * MI_SIZE,
                                               8, 8, 1 )
             } else if ( is_global_mv_cand( YMode, bSize, refIdx ) &&
                           !force_integer_mv ) {
                  mv = get_sub_block_warp_mv( gm_params[ refIdx ], 0,
                                               col * MI_SIZE, row * MI_SIZE,
                                               8, 8, 1 )
             } else {
                  mv = Mvs[ row ][ col ][ list ]
             }
             mvRow = mv[ 0 ]
             mvCol = mv[ 1 ]
         }

           if ( Abs( mvRow ) <= REFMVS_LIMIT && Abs( mvCol ) <= REFMVS_LIMIT ) {
               if ( allowList[list] ) {
                   mfmvs[ list ][ 0 ] = mvRow
                   mfmvs[ list ][ 1 ] = mvCol
                   refs[ list ] = refIdx



AV2 Specification                                                                     Page 573 of 1169
                }
           }
     }
 }
 ref0 = refs[ 0 ]
 mvRow0 = mfmvs[ 0 ][ 0 ]
 mvCol0 = mfmvs[ 0 ][ 1 ]
 ref1 = refs[ 1 ]
 mvRow1 = mfmvs[ 1 ][ 0 ]
 mvCol1 = mfmvs[ 1 ][ 1 ]
 if ( ref0 != NONE && ref1 == NONE ) {
     refs[ 1 ] = ref0
     mfmvs[ 1 ][ 0 ] = mvRow0
     mfmvs[ 1 ][ 1 ] = mvCol0
 } else if ( ref1 != NONE && ref0 == NONE ) {
     refs[ 0 ] = ref1
     mfmvs[ 0 ][ 0 ] = mvRow1
     mfmvs[ 0 ][ 1 ] = mvCol1
 } else if ( ref0 != NONE && refs[ 1 ] != NONE ) {
     refOrder0 = OrderHints[ref0]
     refOrder1 = OrderHints[ref1]
     if ( get_relative_dist( refOrder0, OrderHint ) < 0 &&
                  get_relative_dist( refOrder1, OrderHint ) < 0 ) {
         toSwitch = get_relative_dist( refOrder0, refOrder1 ) < 0
     } else if ( get_relative_dist( refOrder0, OrderHint) > 0 &&
                  get_relative_dist( refOrder1, OrderHint) > 0 ) {
         toSwitch = get_relative_dist( refOrder0, refOrder1 ) < 0
     } else {
         toSwitch = get_relative_dist( refOrder0, OrderHint ) > 0 &&
                     get_relative_dist( refOrder1, OrderHint ) < 0
     }
     if (toSwitch) {
         refs[ 0 ] = ref1
         mfmvs[ 0 ][ 0 ] = mvRow1
         mfmvs[ 0 ][ 1 ] = mvCol1
         refs[ 1 ] = ref0
         mfmvs[ 1 ][ 0 ] = mvRow0
         mfmvs[ 1 ][ 1 ] = mvCol0
     }
 }

 for ( list = 0; list < 2; list++ ) {
     MfRefFrames[ y8 ][ x8 ][ list ] = refs[ list ]
     for ( comp = 0; comp < 2; comp++ ) {
         MfMvs[ y8 ][ x8 ][ list ][ comp ] = compression_mv( mfmvs[list][comp] )
     }
 }


The functions get_tip_cand, get_tip_offsets, to_fullmv, get_sub_block_warp_mv are defined as:

 to_fullmv(mv) {
     return (mv + 3 + ((mv >= 0) ? 1 : 0) ) >> 3
 }

 get_tip_cand(candRow,candCol) {
     baseRow = MiRowBase[ 0 ][ candRow ][ candCol ]
     baseCol = MiColBase[ 0 ][ candRow ][ candCol ]
     shift = 1 + TipSizes16x16[ candRow ][ candCol ]
     candRow = baseRow + (((candRow - baseRow) >> shift) << shift)
     candCol = baseCol + (((candCol - baseCol) >> shift) << shift)
     x8 = candCol >> 1
     y8 = candRow >> 1
     candMvs[ 0 ][ 0 ] = 0
     candMvs[ 0 ][ 1 ] = 0
     candMvs[ 1 ][ 0 ] = 0
     candMvs[ 1 ][ 1 ] = 0
     refX8 = Clip3( 0, (MiCols >> 1) - 1, x8 )
     refY8 = Clip3( 0, (MiRows >> 1) - 1, y8 )
     if ( MotionFieldValid[ refY8 ][ refX8 ] ) {



AV2 Specification                                                                          Page 574 of 1169
           (refOffset, pastOffset, futureOffset) = get_tip_offsets()
           candMvs[ 0 ] = get_mv_projection( MotionFieldMvs[ refY8 ][ refX8 ],
                                             pastOffset, refOffset )
           candMvs[ 1 ] = get_mv_projection( MotionFieldMvs[ refY8 ][ refX8 ],
                                             futureOffset, refOffset )
      }
      for( list = 0; list < 2; list++ ) {
          for(comp=0;comp<2;comp++) {
              candMvs[ list ][ comp ] += Mvs[ candRow ][ candCol ][ 0 ][ comp ]
              candMvs[ list ][ comp ] =
                  Clip3(MV_LOW + 1, MV_UPP - 1, candMvs[ list ][ comp ] )
          }
      }
      return candMvs
 }

 get_tip_offsets() {
     if ( NumFutureRefs > 0 && NumPastRefs > 0 ) {
         refOffset = get_relative_dist( OrderHints[ClosestFuture],
                                        OrderHints[ClosestPast])
     } else {
         refOffset = get_relative_dist( OrderHints[ClosestPast],
                                        OrderHints[ClosestFuture])
     }
     pastOffset = get_relative_dist( OrderHint,
                                     OrderHints[ClosestPast])
     futureOffset = get_relative_dist( OrderHint,
                                       OrderHints[ClosestFuture])
     refOffset = Min( refOffset, MAX_FRAME_DISTANCE )
     return (refOffset, pastOffset, futureOffset)
 }

 get_sub_block_warp_mv( warpParams, plane, x, y, w, h, rnd ) {
     if ( plane == 0 ) {
         subX = 0
         subY = 0
     } else {
         subX = SubsamplingX
         subY = SubsamplingY
     }
     srcX = (x + (w >> 1) ) << subX
     srcY = (y + (h >> 1) ) << subY
     dstX = warpParams[ 2 ] * srcX + warpParams[ 3 ] * srcY + warpParams[ 0 ]
     dstY = warpParams[ 4 ] * srcX + warpParams[ 5 ] * srcY + warpParams[ 1 ]
     if (rnd) {
         mv[ 0 ] = Round2Signed( dstY - (srcY << WARPEDMODEL_PREC_BITS),
                                 WARPEDMODEL_PREC_BITS - 3)
         mv[ 1 ] = Round2Signed( dstX - (srcX << WARPEDMODEL_PREC_BITS),
                                 WARPEDMODEL_PREC_BITS - 3)
     } else {
         mv[ 0 ] = (dstY - (srcY << WARPEDMODEL_PREC_BITS)) >>
                   (WARPEDMODEL_PREC_BITS - 3)
         mv[ 1 ] = (dstX - (srcX << WARPEDMODEL_PREC_BITS)) >>
                   (WARPEDMODEL_PREC_BITS - 3)
     }
     mv[ 0 ] = Clip3(MV_LOW + 1, MV_UPP - 1, mv[ 0 ])
     mv[ 1 ] = Clip3(MV_LOW + 1, MV_UPP - 1, mv[ 1 ])
     return mv
 }


The function compression_mv (which compresses a motion vector component into fewer bits to reduce
memory bandwidth) is specified as:

 compression_mv( v ) {
     a = Abs( v )
     stepLog2 = Max( 0, GetMsb( a ) - 4 )




AV2 Specification                                                                      Page 575 of 1169
          c = ( a >> stepLog2 ) + ( stepLog2 << 4 )
          return v < 0 ? -c : c
     }


    The function uncompression_mv (which decompresses a motion vector component) is specified as:

     uncompression_mv( v ) {
         c = Abs( v )
         stepLog2 = Max( 0, (c >> 4) - 1 )
         a = ( c - (stepLog2 << 4) ) << stepLog2
         return v < 0 ? -a : a
     }


```

<a id="s-7-23"></a>

### § 7.23 Reference frame update process

```text
§   7.23. Reference frame update process
    This process is invoked as the final step in decoding a frame.

    The inputs to this process are the decoded samples for the current frame LrFrame[ plane ][ x ][ y ].

    The output from this process is an updated set of reference frames and previous motion vectors.

    If this is the first time this process is invoked, the variable FrameCounter (used to identify when a frame
    is stored in multiple reference frames) is set equal to 0. Otherwise, the variable FrameCounter is
    incremented by 1.

    The variable first (indicating which is the first reference frame to be updated) is set equal to 1.

    For each value of i from 0 to NUM_REF_FRAMES - 1, the following applies if bit i of refresh_frame_flags
    is equal to 1 (i.e., if (refresh_frame_flags >> i) & 1 is equal to 1):

      • If is_frame_eligible_for_output(i) is equal to 1, the output frame buffers process specified in § 7.21.6
        Output frame buffers process is invoked with i as input.
      • RefValid[ i ] is set equal to (FrameType == KEY_FRAME || FrameType == SWITCH_FRAME) ? first : 1.
      • first is set equal to 0.
      • RefFrameWidth[ i ] is set equal to FrameWidth.
      • RefFrameHeight[ i ] is set equal to FrameHeight.
      • RefCropWidth[ i ] is set equal to CropWidth.
      • RefCropHeight[ i ] is set equal to CropHeight.
      • RefCropLeft[ i ] is set equal to CropLeft.
      • RefCropTop[ i ] is set equal to CropTop.
      • RefMiCols[ i ] is set equal to MiCols.
      • RefMiRows[ i ] is set equal to MiRows.
      • RefFrameType[ i ] is set equal to FrameType.
      • RefSubsamplingX[ i ] is set equal to SubsamplingX.
      • RefSubsamplingY[ i ] is set equal to SubsamplingY.
      • RefLongTermId[ i ] is set equal to LongTermId.
      • RefOutputOrder[ i ] is set equal to output_ordering( -1 ).


    AV2 Specification                                                                                  Page 576 of 1169
  • RefBitDepth[ i ] is set equal to BitDepth.
  • RefNumPlanes[ i ] is set equal to NumPlanes.
  • RefFilmGrainPresent[ i ] is set equal to film_grain_params_present.
  • RefImplicitOutputFrame[ i ] is set equal to implicit_output_frame.
  • RefImmediateOutputFrame[ i ] is set equal to immediate_output_frame.
  • RefOrderHint[ i ] is set equal to OrderHint.
  • RefOrderHintLsbs[ i ] is set equal to OrderHintLsbs.
  • RefBaseQIdx[ i ] is set equal to base_q_idx.
  • RefDeltaQUAc[ i ] is set equal to DeltaQUAc.
  • RefDeltaQVAc[ i ] is set equal to DeltaQVAc.
  • RefFrameFiltersOn[ i ] is set equal to a copy of frame_filters_on.
  • RefFrameLrWienerNs[ i ] is set equal to a copy of FrameLrWienerNs.
  • RefNumFilterClasses[ i ] is set equal to NumFilterClasses.
  • RefCounter[ i ] is set equal to FrameCounter.
  • RefNumTotalRefs[ i ] is set equal to NumTotalRefs.
  • RefTLayerId[ i ] is set equal to obu_tlayer_id.
  • RefMLayerId[ i ] is set equal to obu_mlayer_id.
  • SavedOrderHints[ i ][ j ] is set equal to OrderHints[ j ] for j = 0..REFS_PER_FRAME-1.
  • FrameStore[ i ][ 0 ][ y ][ x ] is set equal to LrFrame[ 0 ][ y ][ x ] for x = 0..(MiCols * MI_SIZE-1), for y
    = 0..(MiRows * MI_SIZE-1).
  • FrameStore[ i ][ plane ][ y ][ x ] is set equal to LrFrame[ plane ][ y ][ x ] for plane = 1..2, for x = 0..
    (MiCols * MI_SIZE >> SubsamplingX) - 1, for y = 0..((MiRows * MI_SIZE >> SubsamplingY) - 1).

  • SavedRefFrames[ i ][ y8 ][ x8 ][ list ] is set equal to MfRefFrames[ y8 ][ x8 ][ list ] for y8 = 0..
    (MiRows>>1)-1, for x8 = 0..(MiCols>>1)-1, for list = 0..1.

  • SavedMvs[ i ][ y8 ][ x8 ][ list ][ comp ] is set equal to MfMvs[ y8 ][ x8 ][ list ][ comp ] for comp = 0..1,
    for y8 = 0..(MiRows>>1)-1, for x8 = 0..(MiCols>>1)-1, for list = 0..1.
  • SavedGmParams[ i ][ ref ][ j ] is set equal to gm_params[ ref ][ j ] for ref = 0..REFS_PER_FRAME-1,
    for j = 0..5.
  • SavedSegmentIds[ i ][ row ][ col ] is set equal to SegmentIds[ row ][ col ] for row = 0..MiRows-1, for
    col = 0..MiCols-1.
  • The function save_cdfs( i ) is invoked (see below).
  • If film_grain_params_present is equal to 1, the following ordered steps apply:

      1. The function load_grain_params is invoked with NUM_REF_FRAMES as input (see § 7.21.2
         Intermediate output preparation process).
      2. The function save_grain_params( i ) is invoked (see below).
  • The function save_ccso_params( i, plane ) is invoked (see below) for plane = 0..2.




AV2 Specification                                                                                  Page 577 of 1169
save_cdfs( ctx ) is a function call that indicates that all the CDF arrays are saved into frame context
number ctx in the range 0 to (NUM_REF_FRAMES - 1). When this function is invoked the following takes
place:

  • A copy of each CDF array mentioned in the semantics for init_coeff_cdfs and init_non_coeff_cdfs is
    saved in an area of memory indexed by ctx.

save_grain_params( i ) is a function call that indicates that all the syntax elements that can be read in
both film_grain_model and film_grain_config should be saved into an area of memory indexed by i.

save_ccso_params( i, plane ) is a function call that indicates that certain variables and arrays are saved
into an area of memory indexed by i and plane:

  • CcsoLumaSizeLog2
  • ccso_planes[plane]
  • ccso_scale_idx[plane]
  • ccso_bo_only[plane]
  • ccso_quant_idx[plane]
  • ccso_ext_filter[plane]
  • ccso_max_band_log2[plane]
  • ccso_edge_clf[plane]
  • CcsoFilterOffset[plane]
  • CcsoBlks[plane]

is_frame_eligible_for_output is a function call that is specified in § 7.21.4 Output implicit output frame
process.

The function load_ccso_params is used in other parts of the specification to reload the specified values.

load_ccso_params( i, plane ) is a function call that indicates that the variables and arrays saved in
save_ccso_params are to be reloaded from an area of memory indexed by i and plane.

                                                                                    ↑ Back to Table of Contents




AV2 Specification                                                                               Page 578 of 1169
```
