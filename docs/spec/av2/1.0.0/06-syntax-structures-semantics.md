# AV2 v1.0.0 — § 6. Syntax structures semantics

<!-- Verbatim mirror of the AOM AV2 v1.0.0 specification (© Alliance for Open Media). The PDF is normative; this is a faithful `pdftotext -layout` copy. See [./README.md](./README.md) and [./index.md](./index.md). Do not hand-edit: regenerate via scripts/spec/regenerate-av2-spec.sh. -->

<a id="s-6"></a>

## § 6 Syntax structures semantics

```text
§   6. Syntax structures semantics
```

<a id="s-6-1"></a>

### § 6.1 General

```text
§   6.1. General
    This section specifies the meaning of the syntax elements read in the syntax structures.

    Important variables and function calls are also described.

```

<a id="s-6-2"></a>

### § 6.2 OBU semantics

```text
§   6.2. OBU semantics
```

<a id="s-6-2-1"></a>

#### § 6.2.1 General OBU semantics

```text
§   6.2.1. General OBU semantics

    An ordered series of OBUs is presented to the decoding process. Each OBU is given to the decoding
    process as a string of bytes along with a variable sz that identifies the total number of bytes in the OBU.

    Methods of framing the OBUs (i.e., of identifying the series of OBUs and their size and payload data) in a
    delivery or container format may be established in a manner outside the scope of this specification. One
    simple method is described in Annex B.

    OBU data starts on the first (most significant) bit and ends on the last bit of the given bytes. The payload
    of an OBU lies between the first bit of the given bytes and the last bit before the first trailing bit. Trailing
    bits are always present, unless the OBU consists of only the header. Trailing bits achieve byte alignment
    when the payload of an OBU is not byte aligned. The trailing bits may also be used for additional byte
    padding, and if used are taken into account in the sz value. In all cases, the pattern used for the trailing
    bits guarantees that all OBUs (except header-only OBUs) end with the same pattern: one bit set to one,
    optionally followed by zeros.


      NOTE: As a validity check for malformed encoded data and for operation in environments in which
      losses and errors can occur, decoders may detect an error if the end of the parsed data is not directly
      followed by the correct trailing bits pattern or if the parsing of the OBU header and payload leads to
      the consumption of bits within the trailing bits (except for Tile Group data which is allowed to read a
      small distance into the trailing bits as described in § 8.2.4 Exit process for symbol decoder).


    obu_extension_flag equal to 1 specifies that extension data is present in the OBU payload.
    obu_extension_flag equal to 0 specifies that no extension data is present and only trailing bits follow the
    OBU payload.

    It is a requirement of bitstream conformance that obu_extension_flag is equal to 0 in bitstreams
    conforming to this specification.

    obu_extension_data_bit is a bit of extension data. The content of this data is not specified in this version
    of this specification and shall be ignored by conforming decoders.


      NOTE:       The extension data will end with trailing bits in the usual manner.

```

<a id="s-6-2-2"></a>

#### § 6.2.2 OBU header semantics

```text
§   6.2.2. OBU header semantics

    OBUs are structured with a header and a payload. The header identifies the type of the payload using the
    obu_type header parameter.



    AV2 Specification                                                                                 Page 255 of 1169
obu_header_extension_flag equal to 1 indicates that the obu_header contains the obu_mlayer_id and
obu_xlayer_id syntax elements to identify the embedded layer and extended layer of this OBU.
obu_header_extension_flag equal to 0 indicates that obu_mlayer_id and obu_xlayer_id are not present and
inferred.


  NOTE:         The inference is defined in § 5.2.2 OBU header syntax


obu_type specifies the type of data structure contained in the OBU payload:

                                Table 6.1: OBU types and their layer-specific status

     obu_type                    Name of obu_type                                 Layer-specific

 0                Reserved                                     -

 1                OBU_SEQUENCE_HEADER                          N

 2                OBU_TEMPORAL_DELIMITER                       N

 3                OBU_MULTI_FRAME_HEADER                       Y

 4                OBU_CLOSED_LOOP_KEY                          Y

 5                OBU_OPEN_LOOP_KEY                            Y

 6                OBU_LEADING_TILE_GROUP                       Y

 7                OBU_REGULAR_TILE_GROUP                       Y

 8                OBU_METADATA_SHORT                           See Table in § 6.16 Metadata OBU semantics

 9                OBU_METADATA_GROUP                           See Table in § 6.16 Metadata OBU semantics

 10               OBU_SWITCH                                   Y

 11               OBU_LEADING_SEF                              Y

 12               OBU_REGULAR_SEF                              Y

 13               OBU_LEADING_TIP                              Y

 14               OBU_REGULAR_TIP                              Y

 15               OBU_BUFFER_REMOVAL_TIMING                    Y

 16               OBU_LAYER_CONFIGURATION_RECORD               N

 17               OBU_ATLAS_SEGMENT                            N

 18               OBU_OPERATING_POINT_SET                      N

 19               OBU_BRIDGE_FRAME                             Y

 20               OBU_MSDO                                     N

 21               OBU_RAS_FRAME                                Y

 22               OBU_QUANTIZATION_MATRIX                      Y

 23               OBU_FILM_GRAIN                               Y

 24               OBU_CONTENT_INTERPRETATION                   Y

 25               OBU_PADDING                                  Either

 26-31            Reserved                                     -


Reserved OBUs are for future use by AOMedia and shall be ignored by decoders conforming to this
version of this specification.




AV2 Specification                                                                                     Page 256 of 1169
The column “Layer-specific” indicates if the corresponding OBU type is considered to be associated with
a specific layer ("Y"), or not ("N").

Metadata OBU types may or may not be layer-specific, depending on the metadata type. The table in
§ 6.16 Metadata OBU semantics specifies which types of metadata OBUs are layer-specific and which are
not.

Padding OBUs may or may not be layer-specific.

obu_tlayer_id specifies the temporal level of the data contained in the OBU.

obu_mlayer_id specifies the embedded level of the data contained in the OBU.

obu_xlayer_id specifies the extended level of the data contained in the OBU.

If obu_xlayer_id is equal to GLOBAL_XLAYER_ID, it is a requirement of bitstream conformance that both
obu_mlayer_id and obu_tlayer_id are equal to 0.

Tile group OBU data associated with obu_tlayer_id and obu_mlayer_id equal to 0 are referred to as the
base layer, whereas tile group OBU data that are associated with obu_mlayer_id greater than 0 or
obu_tlayer_id greater than 0 are referred to as enhancement layer(s).

It is a requirement of bitstream conformance that obu_tlayer_id is less than or equal to max_tlayer_id
obtained from an activated sequence header.

It is a requirement of bitstream conformance that obu_mlayer_id is less than or equal to max_mlayer_id
obtained from an activated sequence header.


  NOTE: These constraints on obu_tlayer_id and obu_mlayer_id apply after a sequence header OBU is
  activated to specify max_tlayer_id and max_mlayer_id.


If obu_type is equal to OBU_MSDO or OBU_TEMPORAL_DELIMITER, it is a requirement of bitstream
conformance that obu_xlayer_id is equal to GLOBAL_XLAYER_ID.

If obu_xlayer_id is equal to GLOBAL_XLAYER_ID, it is a requirement of bitstream conformance that
obu_type is equal to one of OBU_TEMPORAL_DELIMITER, OBU_BUFFER_REMOVAL_TIMING,
OBU_METADATA_SHORT, OBU_METADATA_GROUP, OBU_LAYER_CONFIGURATION_RECORD,
OBU_ATLAS_SEGMENT, OBU_OPERATING_POINT_SET, OBU_MSDO, or OBU_PADDING.

If obu_type is equal to one of OBU_SEQUENCE_HEADER, OBU_TEMPORAL_DELIMITER,
OBU_LAYER_CONFIGURATION_RECORD, OBU_OPERATING_POINT_SET, or OBU_ATLAS_SEGMENT, it
is a requirement of bitstream conformance that all of the following are true:

  • obu_tlayer_id is equal to 0.
  • obu_mlayer_id is equal to 0.

If obu_type is equal to one of OBU_CLOSED_LOOP_KEY, OBU_OPEN_LOOP_KEY, OBU_SWITCH, or
OBU_RAS_FRAME, it is a requirement of bitstream conformance that obu_tlayer_id is equal to 0.




AV2 Specification                                                                           Page 257 of 1169
```

<a id="s-6-2-3"></a>

#### § 6.2.3 Trailing bits semantics

```text
§   6.2.3. Trailing bits semantics

      NOTE: Tile group OBUs and frame OBUs do end with trailing bits, but for these cases, the trailing
      bits are consumed by the exit_symbol process.


    trailing_one_bit shall be equal to 1.

    When the syntax element trailing_one_bit is read, it is a requirement that nbBits is greater than zero.

    trailing_zero_bit shall be equal to 0 and is inserted into the bitstream to align the bit position to a
    multiple of 8 bits and add optional zero padding bytes to the OBU.

```

<a id="s-6-2-4"></a>

#### § 6.2.4 Byte alignment semantics

```text
§   6.2.4. Byte alignment semantics

    zero_bit shall be equal to 0 and is inserted into the bitstream to align the bit position to a multiple of 8
    bits.

```

<a id="s-6-3"></a>

### § 6.3 Reserved OBU semantics

```text
§   6.3. Reserved OBU semantics
    The reserved OBU allows the extension of this specification with additional OBU types in a way that
    allows older decoders to ignore them.

```

<a id="s-6-4"></a>

### § 6.4 Sequence header OBU semantics

```text
§   6.4. Sequence header OBU semantics
```

<a id="s-6-4-1"></a>

#### § 6.4.1 General sequence header OBU semantics

```text
§   6.4.1. General sequence header OBU semantics

    seq_header_id specifies an identification number for the sequence header.

    It is a requirement of bitstream conformance that seq_header_id is less than MAX_SEQ_NUM.

    seq_profile_idc specifies the profile for the coded video sequence identified by the associated
    obu_xlayer_id. The profile constrains the coding capabilities that may be used, as specified in Annex A.2
    Profiles.


      NOTE:       The value space for seq_profile_idc is the same as for multistream_profile_idc.


    single_picture_header_flag specifies that the syntax elements not needed by a still frame are omitted.

    seq_level_idx specifies the level that the coded video sequence conforms to.

    seq_tier equal to 0 specifies that the coded video sequence conforms to the main tier. seq_tier equal to 1
    specifies that the coded video sequence conforms to the high tier.

    monotonic_output_order_flag defines the output mode for a coded video sequence associated with this
    sequence header.

    monotonic_output_order_flag equal to 1 specifies that the output order of coded output frame units is the
    same as their decoding order within the associated coded video sequence. monotonic_output_order_flag
    equal to 0 specifies that the output order of coded output frame units can differ from their decoding
    order within the associated coded video sequence.




    AV2 Specification                                                                               Page 258 of 1169
  NOTE: When monotonic_output_order_flag is equal to 1 for an associated coded video sequence,
  the output order for this coded video sequence is monotonic and the systems or application layer can
  determine that the presentation time is equal to the decoding time without parsing any frame
  headers. When monotonic_output_order_flag is equal to 0 for an associated coded video sequence,
  the output order can be non-monotonic for this coded video sequence and the systems or application
  layer will have to derive the presentation time from coded information associated with each frame.


When single_picture_header_flag is equal to 1, monotonic_output_order_flag is inferred to be equal to 1.

It is a requirement of bitstream conformance that in a coded multistream video sequence, all extended
layers shall be associated with the same value of monotonic_output_order_flag.

It is a requirement of bitstream conformance that in a coded multistream video sequence, all extended
layers within a temporal unit share the same output time and the coded extended layer units from
different extended layers within a temporal unit shall appear in ascending order of obu_xlayer_id.

When monotonic_output_order_flag is equal to 0, additional display order hint constraints on the
temporal unit apply as specified in § 7.3.7 Temporal unit.

chroma_format_idc specifies the chroma subsampling format.

                                 Table 6.2: Chroma format indicator values

 chroma_format_idc    Name of chroma_format_idc   SubsamplingX   SubsamplingY   Monochrome      Description

 0                    CHROMA_FORMAT_420           1              1              0             YUV 4:2:0

 1                    CHROMA_FORMAT_400           1              1              1             Monochrome 4:0:0

 2                    CHROMA_FORMAT_444           0              0              0             YUV 4:4:4

 3                    CHROMA_FORMAT_422           1              0              0             YUV 4:2:2


It is a requirement of bitstream conformance that chroma_format_idc is less than or equal to 3.

bit_depth_idc is used to determine the bit depth.

It is a requirement of bitstream conformance that bit_depth_idc is less than or equal to 1.


  NOTE:       Values of bit_depth_idc greater than 1 are reserved for future use by AOMedia.


The function set_chroma_format_and_bit_depth( ) is defined as follows:

 set_chroma_format_and_bit_depth( ) {
     if ( chroma_format_idc == CHROMA_FORMAT_420 ) {
         SubsamplingX = 1
         SubsamplingY = 1
     } else if ( chroma_format_idc == CHROMA_FORMAT_444 ) {
         SubsamplingX = 0
         SubsamplingY = 0
     } else if ( chroma_format_idc == CHROMA_FORMAT_422 ) {
         SubsamplingX = 1
         SubsamplingY = 0
     } else if ( chroma_format_idc == CHROMA_FORMAT_400 ) {
         SubsamplingX = 1
         SubsamplingY = 1
     }



AV2 Specification                                                                               Page 259 of 1169
      BitDepth = lookup_bitdepth( bit_depth_idc )
      MaxQ = lookup_maxq( bit_depth_idc )
      Monochrome = chroma_format_idc == CHROMA_FORMAT_400
      NumPlanes = Monochrome ? 1 : 3
 }


where lookup_bitdepth and lookup_maxq are functions that indicate that the bit depth and maximum
quantizer value are fetched based on the value of bit_depth_idc from the following table:

                                    Table 6.3: Bit depth indicator values

                  bit_depth_idc                     BitDepth                        MaxQ

 0                                       10                        MAXQ_10_BITS

 1                                       8                         MAXQ_8_BITS

 Greater than 1                          Reserved                  Reserved


Monochrome equal to 1 indicates that the video does not contain U and V color planes. Monochrome
equal to 0 indicates that the video contains Y, U, and V color planes.

SubsamplingX, SubsamplingY specify the chroma subsampling format.

seq_lcr_id specifies the layer configuration record id that corresponds to this sequence header. If this
sequence header is associated with a coded video sequence in an extended layer with obu_xlayer_id equal
to xLayerId and if seq_lcr_id is not equal to 0, the following applies:

  • if an OBU of obu_type equal to OBU_LAYER_CONFIGURATION_RECORD is associated with the
    extended layer id xLayerId (by having lcr_local_id equal to seq_lcr_id) and is either present prior to
    this sequence header in the same bitstream or is provided through external means, then this OBU is
    associated with this sequence header,
  • otherwise, if an OBU of obu_type equal to OBU_LAYER_CONFIGURATION_RECORD is associated
    with an obu_xlayer_id equal to GLOBAL_XLAYER_ID (by having lcr_global_config_record_id equal to
    seq_lcr_id) and is either present prior to this sequence header in the same bitstream or is provided
    through external means, then this OBU is associated with this sequence header,
  • otherwise, no OBU of obu_type equal to OBU_LAYER_CONFIGURATION_RECORD is associated with
    this sequence header.

It is a requirement of bitstream conformance that when seq_lcr_id is not equal to 0 and the activated
layer configuration record is a global layer configuration record, the extended layer with obu_xlayer_id
equal to the obu_xlayer_id of the sequence header shall be included in the lcr_xlayer_map of the
referenced global layer configuration record.


  NOTE: See § 7.3.8.3 LCR availability for the general availability requirements for layer
  configuration record OBUs.


still_picture equal to 1 specifies that the coded video sequence contains only one coded frame.
still_picture equal to 0 specifies that the coded video sequence contains one or more coded frames.

max_tlayer_id specifies the maximum value for obu_tlayer_id for the OBUs represented by this sequence
header.



AV2 Specification                                                                            Page 260 of 1169
max_mlayer_id specifies the maximum value for obu_mlayer_id for the OBUs represented by this
sequence header.

seq_max_mlayer_cnt_minus_1 plus 1 specifies the maximum number of embedded layers that can be
included in the coded video sequence associated with this sequence header. It is a requirement of
bitstream conformance that the value of seq_max_mlayer_cnt_minus_1 is less than or equal to
max_mlayer_id. It is a requirement of bitstream conformance that the number of distinct values of
obu_mlayer_id present in the coded video sequence associated with this sequence header is less than or
equal to SeqMaxMlayerCnt.


  NOTE: The counting applies to all OBUs, even if they are not layer-specific. This means that a
  sequence containing only embedded layer 1 will count as two layers as OBU_SEQUENCE_HEADER is
  forced to use an embedded layer of 0.


frame_width_bits_minus_1 specifies the number of bits minus 1 used for transmitting the frame width
syntax elements.

frame_height_bits_minus_1 specifies the number of bits minus 1 used for transmitting the frame height
syntax elements.

max_frame_width_minus_1 specifies the maximum frame width minus 1 for the frames represented by
this sequence header.

max_frame_height_minus_1 specifies the maximum frame height minus 1 for the frames represented
by this sequence header.

seq_cropping_window_present_flag equal to 1 specifies that the cropping window syntax elements
seq_cropping_win_left_offset, seq_cropping_win_right_offset, seq_cropping_win_top_offset, and
seq_cropping_win_bottom_offset are present in the sequence header to define a cropping rectangle.
seq_cropping_window_present_flag equal to 0 specifies that the cropping window syntax elements are not
present and all crop offset values are inferred to be equal to 0 (no cropping applied).

seq_cropping_win_left_offset is the amount to crop off the left of the frame.

It is a requirement of bitstream conformance that seq_cropping_win_left_offset is less than or equal to
max_frame_width_minus_1.

seq_cropping_win_right_offset is the amount to crop off the right of the frame.

It is a requirement of bitstream conformance that seq_cropping_win_right_offset is less than or equal to
max_frame_width_minus_1.

seq_cropping_win_top_offset is the amount to crop off the top of the frame.

It is a requirement of bitstream conformance that seq_cropping_win_top_offset is less than or equal to
max_frame_height_minus_1.

seq_cropping_win_bottom_offset is the amount to crop off the bottom of the frame.

It is a requirement of bitstream conformance that seq_cropping_win_bottom_offset is less than or equal to
max_frame_height_minus_1.




AV2 Specification                                                                           Page 261 of 1169
  NOTE: The amounts are expressed in terms of pixels to crop for a frame of maximum size. Smaller
  frames will have proportionately fewer pixels cropped.


seq_initial_display_delay_present_flag equal to 1 specifies that the syntax element
seq_initial_display_delay_minus_1 is present to indicate the initial display delay for the xlayer or
sequence that uses this sequence header. seq_initial_display_delay_present_flag equal to 0 specifies that
seq_initial_display_delay_minus_1 is not present and is inferred to be equal to NumRefFrames + 1.

seq_initial_display_delay_minus_1 plus 1 specifies the initial display delay for use in the decoder model
when the video sequence or xlayer is to be decoded. When seq_initial_display_delay_minus_1 is not
present in the bitstream, it is inferred to be equal to NumRefFrames + 1.

decoder_model_info_present_flag equal to 1 specifies that decoder model information is present in the
coded video sequence and the decoder_model_info() syntax structure shall be parsed to specify decoder
buffering model parameters. decoder_model_info_present_flag equal to 0 specifies that decoder model
information is not present and decoder buffering model parameters are not specified in the bitstream.

num_units_in_decoding_tick is the number of time units of a decoding clock operating at the frequency
time_scale Hz that corresponds to one increment of a clock tick counter:

 DecCT = num_units_in_decoding_tick ÷ time_scale



  NOTE: The ÷ operator represents standard mathematical division (in contrast to the / operator
  which represents integer division).


num_units_in_decoding_tick shall be greater than 0. DecCT represents the expected time to decode a
single frame or a common divisor of the expected times to decode frames of different sizes and
dimensions present in the coded video sequence.

seq_decoder_model_info_present_flag equal to 1 specifies that the seq_decoder_model_info() syntax
structure is present and contains decoder model parameters for the xlayer or sequence that uses this
sequence header. seq_decoder_model_info_present_flag equal to 0 specifies that the
seq_decoder_model_info() syntax structure is not present.

An operating point specifies which extended layers, embedded layers, and temporal layers should be
decoded. Operating points are defined within Operating Point Set (OPS) OBUs (see § 5.10 Operating point
set OBU syntax).

For AV2, operating points are specified using:

  • ops_xlayer_map (for global operating point sets): A 31-bit bitmask indicating which extended layers
    are included
  • ops_mlayer_map: An 8-bit bitmask indicating which embedded layers are included for a given
    extended layer
  • ops_tlayer_map: A 4-bit bitmask indicating which temporal layers are included for a given embedded
    layer

See Annex F: Sub-bitstream extraction (informative) for details on operating point selection and sub-
bitstream extraction.


AV2 Specification                                                                           Page 262 of 1169
  NOTE: Operating points are optional. A decoder may choose to decode the entire bitstream without
  selecting a specific operating point.


Operating point selection is an optional decoder capability. When an Operating Point Set (OPS) OBU is
present, a decoder may:

 1. Decode the entire bitstream without selecting a specific operating point
 2. Select an operating point from a global operating point set (for multistream bitstreams)
 3. Select an operating point from a local operating point set (for specific extended layers)

The selection process depends on:

  • Decoder capabilities (profile, level, tier support)
  • Application requirements (resolution, frame rate, bitrate constraints)
  • Available operating points in the OPS OBU(s)

When an operating point is selected, the decoder should perform the sub-bitstream extraction process to
obtain a sub-bitstream containing only the OBUs associated with that operating point. See Annex F: Sub-
bitstream extraction (informative) for the extraction process.


  NOTE: To help with conformance testing, decoders may allow the operating point to be explicitly
  signaled by external means.


  NOTE: A decoder may need to change the operating point selection when a new coded video
  sequence begins or when different extended layers are encountered in a multistream bitstream.


It is a requirement of bitstream conformance that the display order hints computation for any frame (i.e.,
the value returned from get_disp_order_hint) is the same for all the operating points within the bitstream
associated with this frame.

It is a requirement of bitstream conformance that if explicit_ref_frame_map is equal to 0 for a frame, the
implicit reference mapping process results in the same reference mapping (i.e., they result in exactly the
same reference frames to be associated with exactly the same reference indices) for all the operating
points within the bitstream associated with the current frame.


  NOTE: This means that the corresponding calls to the get ref frames process specified in § 7.7 Get
  ref frames process result in exactly the same contents being written to the ref_frame_idx array, and
  that the corresponding reference frames are the same.


It is a requirement of bitstream conformance that if explicit_ref_frame_map is equal to 1 for a frame, any
reference buffer index associated with a particular reference frame, indicated by the explicit reference
mapping process, corresponds to the same frame for all operating points within the bitstream associated
with the current frame.


  NOTE: These requirements ensure that the references used by a frame are the same for all the
  operating points that are associated with the current frame.



AV2 Specification                                                                               Page 263 of 1169
    mlayer_dependency_present_flag specifies whether mlayer_dependency_map syntax elements are
    present in the bitstream.

    mlayer_dependency_map specifies the embedded layer dependencies.

    If obu_type is equal to either OBU_SWITCH or OBU_RAS_FRAME, it is a requirement of bitstream
    conformance that, for any embedded layer ID m not equal to obu_mlayer_id,
    MLayerDependencyMap[obu_mlayer_id][m] shall be equal to 0.

    tlayer_dependency_present_flag specifies whether tlayer_dependency_map syntax elements are
    present in the bitstream.

    multi_tlayer_dependency_map_present_flag equal to 1 specifies that tlayer_dependency_map values
    are signaled for all embedded layers. multi_tlayer_dependency_map_present_flag equal to 0 specifies that
    tlayer_dependency_map is only signaled for embedded layer 0, and the same values are used for all
    embedded layers.

    tlayer_dependency_map specifies the temporal layer dependencies.

    film_grain_params_present equal to 1 specifies that film grain parameters are present in the coded
    video sequence and can be signaled in the frame_header_info to apply film grain synthesis.
    film_grain_params_present equal to 0 specifies that film grain parameters are not present and film grain
    synthesis is disabled for the entire coded video sequence.


      NOTE: Although some film grain parameters (such as apply_grain) are present when
      film_grain_params_present is equal to 1, this does not imply that OBUs with obu_type equal to
      OBU_FILM_GRAIN are definitely present.


    save_sequence_header is a function call that indicates that all the syntax elements and variables read in
    sequence_header_obu are stored in an area of memory indexed by seq_header_id.

```

<a id="s-6-4-2"></a>

#### § 6.4.2 Sequence tile config semantics

```text
§   6.4.2. Sequence tile config semantics

    seq_tile_info_present_flag equal to 1 specifies that tile parameters are present in the coded video
    sequence and the tile_params() syntax structure shall be parsed to determine tile configuration at the
    sequence level. seq_tile_info_present_flag equal to 0 specifies that tile parameters are not present at the
    sequence level and can be signaled at the frame level when allow_tile_info_change is enabled, or default
    to a single tile covering the entire frame.

    allow_tile_info_change equal to 1 specifies that tile configuration can be overridden on a per-frame
    basis in the frame_header_info. allow_tile_info_change equal to 0 specifies that tile configuration cannot
    be changed in the frame_header_info and the sequence-level tile configuration applies to all frames.

```

<a id="s-6-4-3"></a>

#### § 6.4.3 Sequence partition config semantics

```text
§   6.4.3. Sequence partition config semantics

    use_256x256_superblock, when equal to 1, indicates that superblocks in inter frames contain 256x256
    luma samples. When equal to 0, it indicates that use_128x128_superblock is read to determine the
    superblock size.




    AV2 Specification                                                                             Page 264 of 1169
    use_128x128_superblock, when equal to 1, indicates that superblocks contain 128x128 luma samples.
    When equal to 0, it indicates that superblocks contain 64x64 luma samples. (The number of contained
    chroma samples depends on SubsamplingX and SubsamplingY.)

    enable_sdp equal to 1 specifies that SDP is enabled and chroma components can use different
    partitioning structures than the luma component within the coded video sequence. enable_sdp equal to 0
    specifies that SDP is disabled and chroma components use the same partitioning structure as the luma
    component.


      NOTE: When Monochrome is equal to 1, enable_sdp is inferred to be equal to 0. When enabled,
      SDP is triggered when TreeType is equal to SHARED_PART, block size is BLOCK_64X64, and
      FrameIsIntra is equal to 1.


    enable_extended_sdp equal to 1 specifies that extended SDP is enabled and chroma components can
    use different partitioning structures than luma within inter-coded frames. enable_extended_sdp equal to
    0 specifies that extended SDP is disabled for inter frames.


      NOTE: enable_extended_sdp is only signaled when enable_sdp is equal to 1 and
      single_picture_header_flag is equal to 0. Otherwise, it is inferred to be equal to 0.


    enable_ext_partitions equal to 1 specifies that an extended range of partition types beyond the basic set
    is allowed in the coded video sequence. enable_ext_partitions equal to 0 specifies that only the basic set
    of partition types is allowed.


      NOTE: The actual usage of extended partitions (via is_ext_partition_allowed()) requires TreeType
      not equal to CHROMA_PART, or specific block size constraints for CHROMA_PART blocks.


    enable_uneven_4way_partitions equal to 1 specifies that uneven four-way partitions are allowed in the
    coded video sequence. enable_uneven_4way_partitions equal to 0 specifies that uneven four-way
    partitions are not allowed.


      NOTE: enable_uneven_4way_partitions is only signaled when enable_ext_partitions is equal to 1.
      Otherwise, it is inferred to be equal to 0.


    reduce_pb_aspect_ratio equal to 1 specifies that a reduced aspect ratio of blocks is used in the coded
    video sequence. reduce_pb_aspect_ratio equal to 0 specifies that the full range of block aspect ratios is
    allowed.

    max_pb_aspect_ratio_log2_minus_1 plus 1 specifies the base 2 logarithm of the maximum aspect ratio
    of blocks in the coded video sequence.

```

<a id="s-6-4-4"></a>

#### § 6.4.4 Sequence segment config semantics

```text
§   6.4.4. Sequence segment config semantics

    enable_ext_seg enables extra segment ids. enable_ext_seg equal to 0 specifies there are 8 segments
    available. enable_ext_seg equal to 1 specifies there are 16 segments available.

    seq_seg_info_present_flag equal to 1 specifies that segment information is present in this sequence
    header and the seg_info() syntax structure shall be parsed to define sequence-level segmentation



    AV2 Specification                                                                            Page 265 of 1169
    parameters. seq_seg_info_present_flag equal to 0 specifies that segment information is not present at the
    sequence level and can be signaled at the frame level when seq_allow_seg_info_change is enabled.

    seq_allow_seg_info_change equal to 1 specifies that segment information can be overridden on a per-
    frame basis in the frame_header_info. seq_allow_seg_info_change equal to 0 specifies that segment
    information cannot be changed in the frame_header_info and the sequence-level segmentation
    parameters apply to all frames.

```

<a id="s-6-4-5"></a>

#### § 6.4.5 Sequence intra config semantics

```text
§   6.4.5. Sequence intra config semantics

    enable_dip equal to 1 specifies that the use_dip syntax element can be present. enable_dip equal to 0
    specifies that the use_dip syntax element is not present.

    enable_intra_edge_filter equal to 1 specifies that the intra edge filtering process is enabled for intra
    prediction reference samples in the coded video sequence. enable_intra_edge_filter equal to 0 specifies
    that intra edge filtering is disabled and shall not be applied.

    enable_mrls equal to 1 specifies that multiple reference line selection (MRLS) for intra prediction is
    allowed in the coded video sequence. enable_mrls equal to 0 specifies that MRLS is not allowed and only
    the first reference line is used for intra prediction.


      NOTE:       When enable_mrls is equal to 1, MRLS is only used for directional intra prediction modes.


    enable_cfl_intra equal to 1 specifies that chroma from luma (CfL) intra prediction is allowed in the
    coded video sequence. enable_cfl_intra equal to 0 specifies that CfL intra prediction is not allowed.


      NOTE: When enable_cfl_intra is equal to 1, CfL prediction is subject to additional conditions
      including block size constraints, tree type restrictions, and lossless mode considerations as specified
      in the cflAllowed derivation.


    cfl_ds_filter_index specifies the type of down-sampling applied to luma samples in CFL prediction
    process. It is also used to specify the type of down-sampling applied to luma samples in loop restoration
    filtering process.


      NOTE:       A value of 3 can be read for cfl_ds_filter_index, but behaves the same as a value of 0.


    enable_mhccp equal to 1 specifies that MHCCP is allowed in the coded video sequence. enable_mhccp
    equal to 0 specifies that MHCCP is not allowed.


      NOTE: When enable_mhccp is equal to 1, MHCCP is subject to additional conditions including block
      size constraints, tree type restrictions, and lossless mode considerations as specified in the
      is_mhccp_allowed() function.


    enable_ibp equal to 1 specifies that IBP is enabled in the coded video sequence. enable_ibp equal to 0
    specifies that IBP is disabled.

```

<a id="s-6-4-6"></a>

#### § 6.4.6 Sequence inter config semantics

```text
§   6.4.6. Sequence inter config semantics

    seq_enabled_motion_modes specifies which motion modes are enabled.



    AV2 Specification                                                                               Page 266 of 1169
seq_frame_motion_modes_present_flag equal to 1 specifies that the frame_enabled_motion_modes
syntax element can be present in the frame_header_info to override motion mode settings on a per-frame
basis. seq_frame_motion_modes_present_flag equal to 0 specifies that frame_enabled_motion_modes is
not present in frame headers and the sequence-level seq_enabled_motion_modes values apply to all
frames.

enable_six_param_warp_delta equal to 1 specifies that six or four parameters are used for warp delta.
enable_six_param_warp_delta equal to 0 specifies that four parameters are used for warp delta.

enable_masked_compound equal to 1 specifies that the mode info for inter blocks can contain the
syntax element compound_type. enable_masked_compound equal to 0 specifies that the syntax element
compound_type will not be present.

enable_ref_frame_mvs equal to 1 indicates that the use_ref_frame_mvs syntax element can be present.
enable_ref_frame_mvs equal to 0 indicates that the use_ref_frame_mvs syntax element will not be
present.

reduced_ref_frame_mvs_mode equal to 1 indicates that motion fields from at most one reference frame
will be processed.

order_hint_bits_minus_1 is used to compute OrderHintBits.

OrderHintBits specifies the number of bits used for the order_hint syntax element.

enable_refmvbank equal to 1 specifies that banks of recently used motion vectors are used during
motion vector prediction.

disable_drl_reorder and constrain_drl_reorder are used to set the value for DrlReorder:

                                Table 6.4: DrlReorder values and names

              DrlReorder          Name of DrlReorder

                    0             DRL_REORDER_DISABLED

                    1             DRL_REORDER_CONSTRAINT

                    2             DRL_REORDER_ALWAYS


explicit_ref_frame_map equal to 1 specifies that the ref_frame_idx syntax elements will be present in
the frame_header_info.

explicit_num_ref_frames equal to 1 specifies that the num_ref_frames_minus_1 syntax element is
present. Otherwise, num_ref_frames_minus_1 is not present and NumRefFrames is inferred equal to 8.

num_ref_frames_minus_1 plus 1 specifies the number of reference frame slots in the coded video
sequence.

long_term_frame_id_bits specifies the number of bits used to specify long term ids.

It is a requirement of bitstream conformance that if long_term_frame_id_bits is equal to 0, no OBU with
obu_type equal to OBU_RAS_FRAME shall be present in the coded video sequence.


seq_max_drl_bits_minus_1 controls the number of bits read for drl_idx for inter blocks.



AV2 Specification                                                                             Page 267 of 1169
allow_frame_max_drl_bits equal to 1 indicates that change_drl is present in the frame_header_info.

seq_max_bvp_drl_bits_minus_1 controls the number of bits read for drl_idx for intra block copy.

allow_frame_max_bvp_drl_bits equal to 1 indicates that change_bvp_drl is present in the
frame_header_info.

num_same_ref_compound specifies the number of references that can be used for same reference
compound prediction. This refers to a case when a block uses compound inter prediction, but both
references are to the same reference frame.

enable_tip equal to 1 specifies that TIP is enabled in the coded video sequence. enable_tip equal to 0
specifies that TIP is disabled.


  NOTE: When enable_tip is equal to 1, several TIP-related syntax elements and features become
  available: disable_tip_output and EnableTipOutput are determined, enable_tip_refinemv can be
  signaled (when enable_opfl_refine != 0 or enable_refinemv is 1), and TIP reference frame usage
  requires additional conditions including use_ref_frame_mvs equal to 1, NumTotalRefs >= 2, and
  bru_inactive equal to 0.


disable_tip_output equal to 1 prevents TipFrameMode from being set to TIP_FRAME_AS_OUTPUT in
the coded video sequence.

enable_tip_hole_fill equal to 1 specifies that holes in the interpolated motion field are filled in with
estimated motion vectors. enable_tip_hole_fill equal to 0 specifies that holes in the interpolated motion
field are not filled.

enable_mv_traj equal to 1 specifies that motion vector trajectory analysis is enabled. enable_mv_traj
equal to 0 specifies that motion vector trajectory analysis is disabled.

enable_bawp equal to 1 specifies that the allow_bawp syntax element can be present in frame headers
for inter frames, and morph_pred can be used for intra frames when allow_screen_content_tools is
enabled. Otherwise, allow_bawp is not present in frame headers, morph_pred is not used, and both are
inferred to be equal to 0.


  NOTE: The allow_bawp syntax element is only present when FrameIsIntra is equal to 0 (inter
  frames). For intra frames, morph_pred is only signaled when FrameIsIntra is equal to 1 and
  allow_screen_content_tools is equal to 1.


enable_cwp equal to 1 specifies that compound weighted prediction is enabled in the coded video
sequence. enable_cwp equal to 0 specifies that compound weighted prediction is disabled.

enable_imp_msk_bld equal to 1 specifies that implicit mask blending is enabled in the coded video
sequence. enable_imp_msk_bld equal to 0 specifies that implicit mask blending is disabled.

enable_df_sub_pu equal to 1 specifies that the allow_df_sub_pu syntax element is present in frame
headers. enable_df_sub_pu equal to 0 specifies that the allow_df_sub_pu syntax element is not present in
frame headers (and allow_df_sub_pu will be inferred to be equal to 0).

enable_tip_explicit_qp equal to 1 specifies that the quantization parameters for TIP are sent explicitly.
enable_tip_explicit_qp equal to 0 specifies that the quantization parameters are inferred.


AV2 Specification                                                                             Page 268 of 1169
enable_opfl_refine specifies how optical flow is signaled:

                                         Table 6.5: Optical flow signaling modes

                    enable_opfl_refine                               Name of enable_opfl_refine

 0                                                  REFINE_NONE

 1                                                  REFINE_SWITCHABLE

 2                                                  REFINE_ALL

 3                                                  REFINE_AUTO



  NOTE: REFINE_NONE means optical flow is not used in the coded video sequence.
  REFINE_SWITCHABLE means the syntax element use_optflow is present to signal the use per block.
  REFINE_ALL means that optical flow will be used where allowed without being signaled.
  REFINE_AUTO means that the frame_header_info contains the syntax element opfl_refine_type that
  allows the method to be varied per frame.


enable_refinemv equal to 1 specifies that motion vector refinement is enabled in the coded video
sequence. enable_refinemv equal to 0 specifies that motion vector refinement is disabled.

enable_tip_refinemv equal to 1 specifies that motion vector refinement and optical flow can be used
with TIP prediction in the coded video sequence. enable_tip_refinemv equal to 0 specifies that motion
vector refinement and optical flow are not allowed with TIP prediction.

enable_bru equal to 1 specifies that the use_bru syntax element is present for inter frames in frame
headers and backwards reference update is enabled. enable_bru equal to 0 specifies that use_bru is not
present and backwards reference update is disabled.

enable_adaptive_mvd equal to 1 specifies that adaptive motion vector differences are enabled in the
coded video sequence. enable_adaptive_mvd equal to 0 specifies that adaptive motion vector differences
are not allowed.

enable_mvd_sign_derive equal to 1 specifies that the motion vector sign can be derived instead of being
explicitly signaled in the coded video sequence. enable_mvd_sign_derive equal to 0 specifies that motion
vector signs are explicitly signaled.

enable_flex_mvres equal to 1 specifies that the motion vector precision can be specified per block in the
coded video sequence. enable_flex_mvres equal to 0 specifies that a fixed motion vector precision is used
for all blocks.

enable_global_motion equal to 1 specifies that global motion is enabled in the coded video sequence.
enable_global_motion equal to 0 specifies that global motion is disabled.

enable_short_refresh_frame_flags equal to 1 specifies that a compact refresh frame signaling mode is
used where the has_refresh_frame_flags and frame_to_refresh syntax elements can be present to indicate
a single reference frame slot to refresh. enable_short_refresh_frame_flags equal to 0 specifies that the
full refresh_frame_flags bitmask is used to indicate which reference frame slots are refreshed.




AV2 Specification                                                                                 Page 269 of 1169
```

<a id="s-6-4-7"></a>

#### § 6.4.7 Sequence screen content config semantics

```text
§   6.4.7. Sequence screen content config semantics

    seq_choose_screen_content_tools equal to 0 indicates that the seq_force_screen_content_tools syntax
    element will be present. seq_choose_screen_content_tools equal to 1 indicates that
    seq_force_screen_content_tools is set to SELECT_SCREEN_CONTENT_TOOLS.

    seq_force_screen_content_tools equal to SELECT_SCREEN_CONTENT_TOOLS indicates that the
    allow_screen_content_tools syntax element will be present in the frame_header_info. Otherwise,
    seq_force_screen_content_tools contains the value for allow_screen_content_tools.

    seq_choose_integer_mv equal to 0 indicates that the seq_force_integer_mv syntax element will be
    present. seq_choose_integer_mv equal to 1 indicates that seq_force_integer_mv is set to
    SELECT_INTEGER_MV.

    seq_force_integer_mv equal to SELECT_INTEGER_MV indicates that the force_integer_mv syntax
    element will be present in the frame_header_info (providing allow_screen_content_tools is equal to 1).
    Otherwise, seq_force_integer_mv contains the value for force_integer_mv.

```

<a id="s-6-4-8"></a>

#### § 6.4.8 Sequence transform quant entropy config semantics

```text
§   6.4.8. Sequence transform quant entropy config semantics

    enable_fsc equal to 1 specifies that forward skip coding (FSC) is enabled in the coded video sequence.
    enable_fsc equal to 0 specifies that FSC is disabled.

    enable_idtx_intra equal to 1 specifies that the identity transform is allowed for intra blocks when
    enable_fsc is equal to 0. enable_idtx_intra equal to 0 specifies that the identity transform is not allowed
    for intra blocks when enable_fsc is equal to 0. When enable_fsc is equal to 1, enable_idtx_intra is inferred
    to be equal to 1.


      NOTE: The actual usage of identity transform for intra blocks (via allow_fsc_intra()) is also subject
      to block size constraints where block width and height must be less than or equal to FSC_MAX.


    enable_intra_ist equal to 1 specifies that the intra-inter secondary transform (IST) is allowed for intra
    blocks in the coded video sequence. enable_intra_ist equal to 0 specifies that IST is not allowed for intra
    blocks.

    enable_inter_ist equal to 1 specifies that the intra-inter secondary transform (IST) is allowed for inter
    blocks in the coded video sequence. enable_inter_ist equal to 0 specifies that IST is not allowed for inter
    blocks.

    enable_chroma_dctonly equal to 1 specifies that the chroma transform is forced to be only DCT.
    enable_chroma_dctonly equal to 0 specifies that other transform types are allowed for chroma.

    enable_inter_ddt equal to 1 specifies that DDT is allowed for inter blocks in the coded video sequence.
    enable_inter_ddt equal to 0 specifies that DDT is not allowed for inter blocks.

    reduced_tx_part_set equal to 1 specifies that a reduced set of transform partitions is allowed in the
    coded video sequence. reduced_tx_part_set equal to 0 specifies that the full set of transform partitions is
    allowed.

    enable_cctx equal to 1 specifies that CCTX is allowed in the coded video sequence. enable_cctx equal to
    0 specifies that CCTX is not allowed.



    AV2 Specification                                                                             Page 270 of 1169
    enable_tcq equal to 1 specifies that TCQ is allowed in the coded video sequence. enable_tcq equal to 0
    specifies that TCQ is not allowed in the coded video sequence.

    choose_tcq_per_frame equal to 1 specifies that allow_tcq is specified in each frame header.
    choose_tcq_per_frame equal to 0 specifies that allow_tcq is inferred to be equal to enable_tcq.

    enable_parity_hiding equal to 1 specifies that the allow_parity_hiding syntax elements are present in
    the coded video sequence and Parity hiding can be enabled. enable_parity_hiding equal to 0 specifies that
    allow_parity_hiding syntax elements are not present and Parity hiding is disabled.


      NOTE: enable_parity_hiding is inferred to be equal to 0 when enable_tcq is equal to 1 and
      choose_tcq_per_frame is equal to 0. Additionally, allow_parity_hiding is set to 0 when CodedLossless
      is equal to 1 or allow_tcq is equal to 1.


    enable_avg_cdf equal to 1 specifies that the CDFs will be based on an average across CDFs.

    avg_cdf_type equal to 1 specifies that the CDFs will be averaged across tiles. avg_cdf_type equal to 0
    specifies that the CDFs can be blended between the CDFs saved for different reference frames.

    separate_uv_delta_q equal to 1 indicates that the U and V planes may have separate delta quantizer
    values. separate_uv_delta_q equal to 0 indicates that the U and V planes will share the same delta
    quantizer value.

    equal_ac_dc_q specifies that the DC quantizers match the AC quantizers.

    base_y_dc_delta_q specifies a quantizer offset for the DC coefficients in the Y plane.

    base_uv_dc_delta_q specifies a quantizer offset for the DC coefficients in the U and V planes.

    base_uv_ac_delta_q specifies a quantizer offset for the AC coefficients in the U and V planes.

    y_dc_delta_q_enabled specifies that the frame_header_info has a quantizer offset for DC coefficients in
    the Y plane.

    uv_dc_delta_q_enabled specifies that the frame_header_info has a quantizer offset for DC coefficients in
    the U and V planes.

    uv_ac_delta_q_enabled specifies that the frame_header_info has a quantizer offset for AC coefficients in
    the U and V planes.

```

<a id="s-6-4-9"></a>

#### § 6.4.9 Segment information semantics

```text
§   6.4.9. Segment information semantics

    feature_enabled equal to 0 indicates that the corresponding feature is unused and has value equal to 0.
    feature_enabled equal to 1 indicates that the feature value is coded.

    feature_value specifies the feature data for a segment feature.

```

<a id="s-6-4-10"></a>

#### § 6.4.10 Sequence filter config semantics

```text
§   6.4.10. Sequence filter config semantics

    disable_loopfilters_across_tiles equal to 1 specifies that the loop filters do not access samples from a
    different tile.




    AV2 Specification                                                                            Page 271 of 1169
enable_cdef equal to 1 specifies that cdef filtering can be enabled. enable_cdef equal to 0 specifies that
cdef filtering is disabled.


  NOTE: It is allowed to set enable_cdef equal to 1 even when cdef filtering is not used on any frame
  in the coded video sequence. CDEF filtering is automatically disabled when CodedLossless is equal to
  1.


enable_gdf equal to 1 specifies that GDF filtering can be enabled. enable_gdf equal to 0 specifies that
GDF filtering is disabled.


  NOTE:       GDF filtering is automatically disabled when CodedLossless is equal to 1.


gdf_unit_matches_sb_size equal to 1 specifies that the GDF size is taken from the superblock size.
gdf_unit_matches_sb_size equal to 0 specifies that the GDF size is computed based on tile alignment.

enable_restoration equal to 1 specifies that loop restoration filtering can be enabled. enable_restoration
equal to 0 specifies that loop restoration filtering is disabled.


  NOTE: It is allowed to set enable_restoration equal to 1 even when loop restoration is not used on
  any frame in the coded video sequence.


lr_tools_disable[ isChroma ][ i ] equal to 1 specifies that loop restoration tool i is disabled.
lr_tools_disable[ isChroma ][ i ] equal to 0 specifies that loop restoration tool i is not disabled. isChroma
equal to 0 selects luma; isChroma equal to 1 selects chroma.

lr_tools_uv_present equal to 1 specifies that the chroma lr_tools_disable syntax elements are present in
the coded video sequence. lr_tools_uv_present equal to 0 specifies that the chroma lr_tools_disable syntax
elements are not present.


  NOTE: It is allowed to set lr_tools_uv_present equal to 1 even if the stream does not contain
  chroma.


enable_ccso equal to 1 specifies that CCSO filtering can be enabled. enable_ccso equal to 0 specifies
that CCSO filtering is disabled.

ccso_unit_matches_sb_size equal to 1 specifies that the CCSO size is taken from the superblock size.
ccso_unit_matches_sb_size equal to 0 specifies that the CCSO size is computed based on tile alignment.

cdef_on_skip_txfm_always_on equal to 1 specifies that CDEF will always be on for skipped transform
blocks.

cdef_on_skip_txfm_disabled equal to 1 specifies that CDEF will always be off for skipped transform
blocks. cdef_on_skip_txfm_disabled equal to 0 specifies that a frame level enable is used to specify how
CDEF is applied for skipped transform blocks.

df_par_bits_minus_2 plus 2 specifies the number of bits used to read the df_delta_q[ i ] syntax element.




AV2 Specification                                                                               Page 272 of 1169
```

<a id="s-6-4-11"></a>

#### § 6.4.11 User defined QM semantics

```text
§   6.4.11. User defined QM semantics

    qm_copy_from_previous_plane equal to 1 specifies that the quantization matrices are copied from the
    previous plane.

    qm_8x8_is_symmetric equal to 1 specifies that the quantization matrix for TX_8X8 is symmetric (so
    certain entries can be inferred instead of being present in the bitstream).

    qm_4x8_is_transpose_of_8x4 equal to 1 specifies that the quantization matrix for TX_4X8 is equal to the
    transpose of the matrix for TX_8X4.

    quant_delta specifies the adjustment between quantizer values.

    It is a requirement of bitstream conformance that quant_delta is greater than or equal to -128, and less
    than or equal to 127.

    It is a requirement of bitstream conformance that no value written into UserQm is equal to 0.

```

<a id="s-6-4-12"></a>

#### § 6.4.12 Timing info semantics

```text
§   6.4.12. Timing info semantics

    num_units_in_display_tick is the number of time units of a clock operating at the frequency time_scale
    Hz that corresponds to one increment of a clock tick counter. A display clock tick, in seconds, is equal to
    num_units_in_display_tick divided by time_scale:

     DispCT = num_units_in_display_tick ÷ time_scale



      NOTE: The ÷ operator represents standard mathematical division (in contrast to the / operator
      which represents integer division).


    It is a requirement of bitstream conformance that num_units_in_display_tick is greater than 0.

    It is a requirement of bitstream conformance that within a coded video sequence,
    num_units_in_display_tick, when present, has the same value across all embedded layers.

    time_scale is the number of time units that pass in one second.

    It is a requirement of bitstream conformance that time_scale is greater than 0.

    It is a requirement of bitstream conformance that within a coded video sequence, time_scale, when
    present, has the same value across all embedded layers.

    equal_picture_interval equal to 1 indicates that pictures should be displayed according to their output
    order with the number of ticks between two consecutive pictures (without dropping frames) specified by
    num_ticks_per_picture_minus_1 + 1. equal_picture_interval equal to 0 indicates that the interval between
    two consecutive pictures is not specified.

    It is a requirement of bitstream conformance that within a coded video sequence, equal_picture_interval,
    when present, has the same value across all embedded layers.

    num_ticks_per_picture_minus_1 plus 1 specifies the number of clock ticks corresponding to output
    time between two consecutive pictures in the output order.




    AV2 Specification                                                                             Page 273 of 1169
    It is a requirement of bitstream conformance that the value of num_ticks_per_picture_minus_1 shall be in the
    range of 0 to (1 << 32) − 2, inclusive.

    It is a requirement of bitstream conformance that within a coded video sequence,
    num_ticks_per_picture_minus_1, when present, has the same value across all embedded layers.


      NOTE: The frame rate, when specified explicitly, applies to the top temporal layer of the bitstream.
      If bitstream is expected to be manipulated, e.g., by intermediate network elements, then the resulting
      frame rate may not match the specified one. In this case, an encoder is advised to use explicit time
      codes or some mechanisms that convey picture timing information outside the bitstream.

```

<a id="s-6-4-13"></a>

#### § 6.4.13 Sequence decoder model info semantics

```text
§   6.4.13. Sequence decoder model info semantics

    decoder_buffer_delay specifies the time interval between the arrival of the first bit in the smoothing
    buffer and the subsequent removal of the data that belongs to the first coded frame, measured in units of
    1/90000 seconds.

    encoder_buffer_delay specifies, in combination with decoder_buffer_delay syntax element, the first bit
    arrival time of frames to be decoded to the smoothing buffer. encoder_buffer_delay is measured in units
    of 1/90000 seconds.

    For a video sequence that includes one or more random access points the sum of decoder_buffer_delay
    and encoder_buffer_delay shall be kept constant.

    low_delay_mode_flag equal to 1 indicates that the smoothing buffer operates in low-delay mode. In low-
    delay mode late decode times and buffer underflow are both permitted. low_delay_mode_flag equal to 0
    indicates that the smoothing buffer operates in strict mode, where buffer underflow is not allowed.

    The parameters decoder_buffer_delay, encoder_buffer_delay, and low_delay_mode_flag are applied to the
    xlayer or sub-bitstream that uses the sequence header containing these parameters.

```

<a id="s-6-5"></a>

### § 6.5 Temporal delimiter OBU semantics

```text
§   6.5. Temporal delimiter OBU semantics
    SeenFrameHeader is a variable used to mark whether the frame_header_info for the current frame has
    been received. It is initialized to zero.

```

<a id="s-6-6"></a>

### § 6.6 Multi Stream Decoder Operation OBU semantics

```text
§   6.6. Multi Stream Decoder Operation OBU semantics
    It is a requirement of bitstream conformance that a Multi Stream Decoder Operation OBU has:

     1. obu_tlayer_id equal to 0.
     2. obu_mlayer_id equal to 0.
     3. obu_xlayer_id equal to GLOBAL_XLAYER_ID.

    num_streams_minus_2 plus 2 specifies the number of independent streams in the bitstream. It is a
    requirement of bitstream conformance that num_streams_minus_2 is not greater than 2.

    multistream_profile_idc specifies the coding features that can be used in a coded multistream video
    sequence. The allowed values for multistream_profile_idc are the same as those for seq_profile_idc as
    defined in Table A.4.




    AV2 Specification                                                                              Page 274 of 1169
It is a requirement of bitstream conformance that multistream_profile_idc is greater than or equal to
sub_stream_max_profile[i] for all i in the range 0 to num_streams_minus_2 + 1, inclusive.

multistream_level_idx specifies the level to which the coded multistream video sequence conforms.

multistream_tier specifies the tier to which the coded multistream video sequence conforms.

multistream_even_allocation_flag specifies the resource allocation for the multistream.

multistream_large_picture_idc specifies an index of the sub_xlayer_id array that has a larger resource
allocation than the other independent sub-bitstreams.

sub_xlayer_id[ i ] specifies the value of obu_xlayer_id in the OBU header for the i-th independent sub-
bitstream in the present bitstream.

sub_stream_max_profile[ i ] indicates the maximum value for seq_profile_idc that may appear in a
sequence header activated by the i-th independent sub-bitstream.

It is a requirement of bitstream conformance that seq_profile_idc is less than or equal to
sub_stream_max_profile[i] for each sequence header activated by the i-th independent sub-stream.

sub_stream_max_level[ i ] indicates the maximum value for seq_level_idx that may appear in a
sequence header activated by the i-th independent sub-bitstream.

It is a requirement of bitstream conformance that seq_level_idx is less than or equal to
sub_stream_max_level[i] for each sequence header activated by the i-th independent sub-stream.

sub_stream_max_tier[ i ] indicates the maximum value for seq_tier that may appear in a sequence
header activated by the i-th independent sub-bitstream.

It is a requirement of bitstream conformance that seq_tier is less than or equal to sub_stream_max_tier[i]
for each sequence header activated by the i-th independent sub-stream.


  NOTE: The values of sub_stream_max_profile[i], sub_stream_max_level[i], and
  sub_stream_max_tier[i] are not used in determining the profile and level constraints in Annex A.
  There is no constraint that there exists a value of seq_profile_idc, seq_level_idx or seq_tier equal to
  the indicated maximum.


multistream_doh_constraint_flag equal to 1 specifies that additional display order hint (DOH)
constraints on the temporal unit are enabled. multistream_doh_constraint_flag equal to 0 specifies that
additional DOH constraints on the temporal unit are not enabled.

It is a requirement of bitstream conformance that when monotonic_output_order_flag is equal to 0 in any
activated sequence header of the coded multistream video sequence, multistream_doh_constraint_flag
shall be equal to 1.


  NOTE:       The constraints enabled by the multistream_doh_constraint_flag appear in § 7.3.7 Temporal
  unit




AV2 Specification                                                                              Page 275 of 1169
```

<a id="s-6-7"></a>

### § 6.7 Multi frame header OBU semantics

```text
§   6.7. Multi frame header OBU semantics
    mfh_seq_header_id specifies a sequence header id.

    It is a requirement of bitstream conformance that mfh_seq_header_id is less than MAX_SEQ_NUM.

    mfh_id_minus_1 plus 1 identifies the multi-frame header for reference by a frame header or a coded
    frame.

    It is a requirement of bitstream conformance that mfh_id_minus_1 + 1 is less than MAX_MFH_NUM.

    mfh_frame_size_present_flag equal to 1 specifies that the syntax elements mfh_frame_width_minus_1
    and mfh_frame_height_minus_1 are present in the multi-frame header to override the sequence-level
    frame size. mfh_frame_size_present_flag equal to 0 specifies that these syntax elements are not present
    and the frame size from the sequence header applies to frames using this multi-frame header.

    mfh_frame_width_bits_minus_1 plus one specifies the number of bits used to read
    mfh_frame_width_minus_1.

    mfh_frame_height_bits_minus_1 plus one specifies the number of bits used to read
    mfh_frame_height_minus_1.

    mfh_frame_width_minus_1 plus one specifies the width of the frame that references the multi-frame
    header in luma samples.

    mfh_frame_height_minus_1 plus one specifies the height of the frame that references the multi-frame
    header in luma samples.

    mfh_deblocking_filter_update equal to 1 specifies that the syntax elements
    mfh_apply_deblocking_filter are present in the multi-frame header. mfh_deblocking_filter_update equal to
    0 specifies that mfh_apply_deblocking_filter syntax elements are not present.

    mfh_apply_deblocking_filter is an array containing flags that specify if the deblocking filter is applied
    for a particular plane and direction. Different mfh_apply_deblocking_filter values from the array are used
    by a frame header or a coded frame that references the multi-frame header, depending on the image
    plane being filtered, and the edge direction (vertical or horizontal) being filtered.

    mfh_seg_info_present_flag equal to 1 specifies that segment information is present in this multi-frame
    header and the seg_info() syntax structure shall be parsed. mfh_seg_info_present_flag equal to 0 specifies
    that segment information is not present in this multi-frame header.

    mfh_ext_seg_flag equal to 1 specifies that the segment information uses an extended number of 16
    segments. mfh_ext_seg_flag equal to 0 specifies that the segment information uses the standard 8
    segments.

    mfh_allow_seg_info_change equal to 1 specifies that the segment information in this multi-frame
    header can be overridden in the frame_header_info. mfh_allow_seg_info_change equal to 0 specifies that
    segment information cannot be changed in the frame_header_info.

```

<a id="s-6-8"></a>

### § 6.8 Layer config record OBU semantics

```text
§   6.8. Layer config record OBU semantics
    This OBU contains either global information or local layer information depending on the value of
    obu_xlayer_id.


    AV2 Specification                                                                           Page 276 of 1169
```

<a id="s-6-8-1"></a>

#### § 6.8.1 General

```text
§   6.8.1. General

    The Layer Configuration Record (LCR) provides comprehensive metadata about the structure, properties,
    and relationships of layers within an AV2 bitstream. The LCR serves multiple critical purposes:

    Multi-view and multi-layer organization: The LCR enables complex content scenarios where multiple
    independent layers represent different aspects or views of the same scene. Each embedded layer within
    an extended layer can be annotated with metadata that describes its role in the overall composition.

    Layer type and purpose identification: Through the combination of lcr_layer_type and
    lcr_auxiliary_type, the LCR distinguishes between primary texture content and auxiliary data. Texture
    layers (lcr_layer_type == TEXTURE_LAYER) carry the main visual content, while auxiliary layers
    (lcr_layer_type == AUX_LAYER) provide supplementary information such as alpha channels
    (transparency), depth maps for 3D representation, segmentation masks, or gain maps for HDR tone
    mapping.

    View association and multi-view content: The lcr_view_type and lcr_view_id fields enable sophisticated
    multi-view scenarios. For stereoscopic content, different layers can be marked as VIEW_LEFT or
    VIEW_RIGHT, or assigned explicit view IDs through VIEW_EXPLICIT combined with lcr_view_id. This
    allows a single bitstream to carry multiple perspectives of the same scene, where each view can have its
    own texture layer plus associated auxiliary layers (alpha, depth, etc.). For example, a stereoscopic stream
    might have:

      • Layer 0: Left view texture (lcr_view_id = 0, lcr_layer_type = TEXTURE_LAYER)
      • Layer 1: Left view depth (lcr_view_id = 0, lcr_layer_type = AUX_LAYER, lcr_auxiliary_type =
        DEPTH_AUX)
      • Layer 2: Right view texture (lcr_view_id = 1, lcr_layer_type = TEXTURE_LAYER)
      • Layer 3: Right view depth (lcr_view_id = 1, lcr_layer_type = AUX_LAYER, lcr_auxiliary_type =
        DEPTH_AUX)

    Atlas integration: The lcr_layer_atlas_segment_id field associates each layer with a specific atlas segment,
    enabling spatial composition and layout specification. The atlas defines how different layers should be
    positioned, scaled, or composed to form the final rendered output. This association is particularly
    powerful for:

      • Subpicture and region-of-interest applications where different regions are encoded as separate layers
      • Virtual screen composition where the atlas represents a larger virtual canvas and layers are placed at
        specific positions
      • Multistream composition where layers from different extended layers are combined spatially

    Layer dependencies: The lcr_dependent_layer_map indicates inter-prediction dependencies between layers,
    allowing decoders to understand which layers can be decoded independently and which require other
    layers as references.

    The LCR can be specified at two scopes: global (obu_xlayer_id == 31) for multistream scenarios, or local
    (obu_xlayer_id in 0..30) for individual extended layers. Global LCRs provide cross-layer metadata and
    relationships, while local LCRs describe the structure within a single extended layer sub-bitstream.

    For detailed usage examples including stereoscopic video, multi-property layers, and subpicture
    composition, see Annex G: Layer composition and Atlas usage examples (informative).


    AV2 Specification                                                                             Page 277 of 1169
```

<a id="s-6-8-2"></a>

#### § 6.8.2 LCR global info semantics

```text
§   6.8.2. LCR global info semantics

    lcr_global_config_record_id provides an identifier for the global LCR for reference by other syntax
    elements.

    It is a requirement of bitstream conformance that lcr_global_config_record_id is in the range of 1 to 7,
    inclusive.

    lcr_xlayer_map is a bitmap indicating the extended layer sub-bitstreams that are associated with this
    global LCR and can be present in a CVS that refers to this global LCR. It is a requirement of bitstream
    conformance that lcr_xlayer_map is in the range of 1 to (1 << 31) - 1, inclusive.

    It is a requirement of bitstream conformance that all extended layers present in the multistream shall
    reference the same activated global LCR (i.e., the same value of lcr_global_config_record_id).

    lcr_aggregate_info_present_flag equal to 1 specifies that the lcr_aggregate_info() syntax structure is
    present in the current LCR to indicate the aggregate information of all sub-bitstreams that can be
    present in the CVS associated with this global LCR. lcr_aggregate_info_present_flag equal to 0 specifies
    that this information is not present but may be derived by examining the profile, tier, and level indicators,
    in addition to the maximum number of embedded layers that are indicated for each individual extended
    layer that is associated with this LCR.

    lcr_seq_profile_tier_level_info_present_flag equal to 1 specifies that the
    lcr_seq_profile_tier_level_info( i ) syntax structure is present in the current LCR for an extended layer
    with index i to indicate the sequence profile, tier, level, and maximum number of embedded layers that
    can be present in the extended layer sub-bitstream with obu_xlayer_id equal to i that is associated with
    this global LCR. lcr_seq_profile_tier_level_info_present_flag equal to 0 specifies that this information is
    not present but may be derived through other means.

    lcr_global_payload_present_flag equal to 1 specifies that the payload lcr_global_payload( i ) is present
    in this syntax structure for each individual extended layer i associated with this LCR.
    lcr_global_payload_present_flag equal to 0 specifies that lcr_global_payload( i ) for each individual
    extended layer i associated with this LCR is not present.

    lcr_dependent_xlayers_flag equal to 1 specifies that the syntax element
    lcr_num_dependent_xlayer_map[ j ] for any extended layer with ID equal to j is present in the current
    LCR. lcr_dependent_xlayers_flag equal to 0 specifies that the lcr_num_dependent_xlayer_map[ j ] syntax
    element is not present in the current global LCR.

    It is a requirement of bitstream conformance that the value of lcr_dependent_xlayers_flag is equal to 0.
    Decoders conforming to this version of this specification shall ignore non-zero values of
    lcr_dependent_xlayers_flag.

    lcr_global_atlas_id_present_flag equal to 1 specifies that the lcr_global_atlas_id syntax element is
    present in the current global LCR. lcr_global_atlas_id_present_flag equal to 0 specifies that the
    lcr_global_atlas_id syntax element is not present in the current global LCR.

    lcr_global_purpose_id specifies the application purpose for the layered bitstream associated with this
    global LCR by referencing its lcr_global_config_record_id, as follows:

                                 Table 6.6: LCR global purpose identifier values




    AV2 Specification                                                                              Page 278 of 1169
          lcr_global_purpose_id                                       Application Purpose

 0                                     Unspecified

 1                                     Stereoscopic Viewports

 2                                     Immersive Multiple Viewports

 3                                     Immersive Multiple Viewports + Alpha

 4                                     Immersive Multiple Viewports + Depth

 5                                     Immersive Multiple Viewports + Alpha + Depth

 6                                     Multiview Playback

 7                                     Subregion Playback

 8-127                                 Reserved


lcr_doh_constraint_flag equal to 1 specifies that additional display order hint (DOH) constraints on the
temporal unit are enabled. lcr_doh_constraint_flag equal to 0 specifies that additional DOH constraints on
the temporal unit are not enabled.

It is a requirement of bitstream conformance that when monotonic_output_order_flag is equal to 0 in any
activated sequence header of the coded multistream video sequence, lcr_doh_constraint_flag shall be
equal to 1.


  NOTE:       The constraints enabled by the lcr_doh_constraint_flag appear in § 7.3.7 Temporal unit


lcr_enforce_tile_alignment_flag equal to 1 specifies that all extended layer sub-bitstreams associated
with this global LCR shall use the same tile structure. When lcr_enforce_tile_alignment_flag is set equal
to 1, it is a requirement of bitstream conformance that all extended layers use the same values of
TileCols, TileRows, and the same tile column and row start positions. lcr_enforce_tile_alignment_flag
equal to 0 specifies that the extended layer sub-bitstreams are not required to use the same tile
structure.

lcr_global_atlas_id specifies the value of the atlas_segment_id[ 31 ] associated with the current global
LCR. When lcr_global_atlas_id_present_flag is equal to 0, the value of lcr_global_atlas_id is inferred to be
equal to 0.

lcr_global_reserved_zero_3bits shall be equal to 0 in bitstreams conforming to this specification. Other
values for lcr_global_reserved_zero_3bits are reserved for future use by AOMedia. Decoders shall ignore
the value of lcr_global_reserved_zero_3bits.

lcr_global_reserved_zero_5bits shall be equal to 0 in bitstreams conforming to this specification. Other
values for lcr_global_reserved_zero_5bits are reserved for future use by AOMedia. Decoders shall ignore
the value of lcr_global_reserved_zero_5bits.

When both an OBU with obu_type equal to OBU_MSDO and an activated global layer configuration
record OBU are present in the same coded multistream video sequence, it is a requirement of bitstream
conformance that the following constraints hold:

 1. The value of num_streams_minus_2 + 2 is equal to LcrMaxNumXLayerCount.
 2. For each i in the range of 0 to num_streams_minus_2 + 1, inclusive, there exists a j in the range of 0
    to LcrMaxNumXLayerCount - 1, inclusive, such that sub_xlayer_id[ i ] is equal to LcrXLayerID[ j ].



AV2 Specification                                                                             Page 279 of 1169
     3. When lcr_aggregate_info_present_flag is equal to 1 in the activated global LCR:

      • multistream_profile_idc shall be consistent with the multi-sequence configuration indicated by
        lcr_config_idc, as specified in Annex A.3 Multi-sequence configurations.
      • The interoperability point associated with multistream_profile_idc, as specified in Annex A.2 Profiles,
        shall be equal to lcr_max_interop.
      • multistream_level_idx shall be equal to lcr_aggregate_level_idx.
      • multistream_tier shall be equal to lcr_max_tier_flag.

     1. When lcr_seq_profile_tier_level_info_present_flag is equal to 1 in the activated global LCR, for each i
        in the range of 0 to num_streams_minus_2 + 1, inclusive:

      • sub_stream_max_profile[ i ] shall be equal to lcr_seq_profile_idc[ sub_xlayer_id[ i ] ].
      • sub_stream_max_level[ i ] shall be equal to lcr_max_level_idx[ sub_xlayer_id[ i ] ].
      • sub_stream_max_tier[ i ] shall be equal to lcr_tier_flag[ sub_xlayer_id[ i ] ].

     1. multistream_doh_constraint_flag shall be equal to lcr_doh_constraint_flag.


      NOTE: The above constraints ensure that when both an MSDO OBU and a global LCR are present
      in the same coded multistream video sequence, the common information signaled in both structures
      is aligned.


    lcr_data_size[ i ] indicates the number of bytes present in an indicated lcr_global_payload() module that
    is associated with the extended layer sub-bitstream with obu_xlayer_id equal to i.


      NOTE: A decoder can use lcr_data_size[ i ] to skip over the lcr_global_payload() for extended layers
      that are not required for decoding.

```

<a id="s-6-8-3"></a>

#### § 6.8.3 LCR local info semantics

```text
§   6.8.3. LCR local info semantics

    lcr_global_id[ i ] specifies the value of the lcr_global_config_record_id associated with the local LCR that
    is indicated in an extended layer with obu_xlayer_id equal to i.

    If lcr_global_id is equal to 0, no global LCR is associated with this local LCR.

    lcr_local_id[ i ] provides an identifier for the local LCR indicated in an extended layer with ID equal to i
    for reference by other syntax elements.

    It is a requirement of bitstream conformance that lcr_local_id[ i ] is not equal to 0.

    lcr_profile_tier_level_info_present_flag[ i ] equal to 1 specifies that the
    lcr_seq_profile_tier_level_info( i ) syntax structure is present in the current LCR for the extended layer
    with index i, indicating the sequence profile, tier, level, and maximum number of embedded layers that
    can be present in the extended layer sub-bitstream with obu_xlayer_id equal to i.
    lcr_profile_tier_level_info_present_flag[ i ] equal to 0 specifies that this information is not present but may
    be derived through other means.

    lcr_local_atlas_id_present_flag[ i ] equal to 1 specifies that the syntax element lcr_local_atlas_id[ i ] is
    present in the local LCR in the extended layer with obu_xlayer_id equal to i.


    AV2 Specification                                                                               Page 280 of 1169
    lcr_local_atlas_id_present_flag[ i ] equal to 0 specifies that the lcr_local_atlas_id[ i ] syntax element is not
    present.

    lcr_local_atlas_id[ i ] provides an identifier for a local atlas with atlas_segment_id equal to
    lcr_local_atlas_id[ i ] that is associated with the extended layer with obu_xlayer_id equal to i. If this value
    is not present this information can be provided by a global atlas, if present, or is considered as
    unspecified.

    lcr_local_reserved_zero_3bits[ i ] shall be equal to 0 in bitstreams conforming to this specification.
    Other values for lcr_local_reserved_zero_3bits[ i ] are reserved for future use by AOMedia. Decoders shall
    ignore the value of lcr_local_reserved_zero_3bits[ i ].

    lcr_local_reserved_zero_5bits[ i ] shall be equal to 0 in bitstreams conforming to this specification.
    Other values for lcr_local_reserved_zero_5bits[ i ] are reserved for future use by AOMedia. Decoders shall
    ignore the value of lcr_local_reserved_zero_5bits[ i ].

```

<a id="s-6-8-4"></a>

#### § 6.8.4 LCR aggregate info semantics

```text
§   6.8.4. LCR aggregate info semantics

    lcr_config_idc indicates a configuration to which the associated bitstream that has activated this global
    LCR conforms to Annex A. Bitstreams conforming to this specification shall not contain values of
    lcr_config_idc outside those specified in Annex A. Other values of lcr_config_idc are reserved for future
    extensions of this specification by AOMedia.

    lcr_aggregate_level_idx indicates an aggregate level indicator to which the combination of all sub-
    bitstreams associated with a bitstream that has activated this LCR conforms to Annex A. Bitstreams
    conforming to this specification shall not contain values of lcr_aggregate_level_idx outside those specified
    in Annex A.

    lcr_max_tier_flag indicates the maximum tier indicator to which all sub-bitstreams associated with a
    bitstream that has activated this LCR conform to according to Annex A.

    lcr_max_interop indicates the maximum interoperability point that the associated bitstream that has
    activated this LCR conforms to Annex A. Bitstreams conforming to this specification shall not contain
    values of lcr_max_interop outside those specified in Annex A.

```

<a id="s-6-8-5"></a>

#### § 6.8.5 LCR sequence profile tier level information semantics

```text
§   6.8.5. LCR sequence profile tier level information semantics

    lcr_seq_profile_idc[ i ] specifies the value of the seq_profile_idc associated with the local LCR that is
    indicated in an extended layer with obu_xlayer_id equal to i. Bitstreams conforming to this specification
    shall not contain values of lcr_seq_profile_idc[ i ] outside those specified in Annex A.

    It is a requirement of bitstream conformance that, when lcr_seq_profile_tier_level_info( i ) is present in an
    activated LCR, seq_profile_idc is less than or equal to lcr_seq_profile_idc[ i ] for each sequence header
    activated by the extended layer sub-bitstream with obu_xlayer_id equal to i.

    lcr_max_level_idx[ i ] specifies the maximum level associated with the local LCR that is indicated in an
    extended layer with obu_xlayer_id equal to i. Bitstreams conforming to this specification shall not contain
    values of lcr_max_level_idx[ i ] outside those specified in Annex A.

    It is a requirement of bitstream conformance that, when lcr_seq_profile_tier_level_info( i ) is present in an
    activated LCR, seq_level_idx is less than or equal to lcr_max_level_idx[ i ] for each sequence header
    activated by the extended layer sub-bitstream with obu_xlayer_id equal to i.



    AV2 Specification                                                                                 Page 281 of 1169
    lcr_tier_flag[ i ] specifies the tier indicator associated with the local LCR that is indicated in an
    extended layer with obu_xlayer_id equal to i. Bitstreams conforming to this specification shall not contain
    values of lcr_tier_flag[ i ] outside those specified in Annex A.

    It is a requirement of bitstream conformance that, when lcr_seq_profile_tier_level_info( i ) is present in an
    activated LCR, seq_tier is less than or equal to lcr_tier_flag[ i ] for each sequence header activated by the
    extended layer sub-bitstream with obu_xlayer_id equal to i.


      NOTE: The values of lcr_seq_profile_idc[ i ], lcr_max_level_idx[ i ], and lcr_tier_flag[ i ] are not used
      in determining the profile and level constraints in Annex A. There is no constraint that there exists a
      value of seq_profile_idc, seq_level_idx or seq_tier equal to the indicated maximum.


    lcr_max_mlayer_count[ i ] specifies the maximum number of embedded layers that can be associated
    with the local LCR that is indicated in an extended layer with obu_xlayer_id equal to i. Bitstreams
    conforming to this specification shall not contain values of lcr_max_mlayer_count[ i ] outside those
    specified in Annex A.

    It is a requirement of bitstream conformance that, when lcr_seq_profile_tier_level_info( i ) is present in an
    activated LCR, seq_max_mlayer_cnt_minus_1 plus 1 is less than or equal to lcr_max_mlayer_count[ i ] for
    each sequence header activated by the extended layer sub-bitstream with obu_xlayer_id equal to i.

    lsptli_reserved_2bits shall be equal to 0 in bitstreams conforming to this specification. Other values for
    lsptli_reserved_2bits are reserved for future use by AOMedia. Decoders shall ignore the value of
    lsptli_reserved_2bits.

```

<a id="s-6-8-6"></a>

#### § 6.8.6 LCR global payload semantics

```text
§   6.8.6. LCR global payload semantics

    lcr_num_dependent_xlayer_map[ j ] indicates the extended layers on which the extended layer with ID
    j can depend on in terms of inter-layer prediction. An extended layer with ID j can only depend on layers
    with an ID smaller than j. When lcr_dependent_xlayers_flag is equal to 0, or when j is equal to 0, the
    value of lcr_num_dependent_xlayer_map[ j ] is inferred to be equal to 0.

    lcr_remaining_payload_bit can take any value but is reserved for future use by AOMedia. Decoders
    conforming to this specification shall ignore the value of lcr_remaining_payload_bit.

    It is a requirement of bitstream conformance that any computed values for RemainingLcrPayloadBits
    shall not be less than 0.

```

<a id="s-6-8-7"></a>

#### § 6.8.7 LCR xlayer info semantics

```text
§   6.8.7. LCR xlayer info semantics

    lcr_rep_info_present_flag[ i ][ j ] indicates the presence of the global, if i is equal to 1, or local, if i is
    equal to 0, lcr_rep_info( i, j ) syntax in the extended layer information for extended layer id j. If
    lcr_rep_info_present_flag[ i ][ j ] is equal to 1, the corresponding lcr_rep_info( i, j) syntax is present,
    otherwise, this syntax is not present.

    lcr_xlayer_purpose_present_flag[ i ][ j ] indicates the presence of the lcr_xlayer_purpose_id[ i ][ j ]
    syntax element in the current LCR. If lcr_xlayer_purpose_present_flag[ i ][ j ] is equal to 1, then
    lcr_xlayer_purpose_id[ i ][ j ] is present. Otherwise, if lcr_xlayer_purpose_present_flag[ i ][ j ] is equal to 0,
    then lcr_xlayer_purpose_id[ i ][ j ] is not present.




    AV2 Specification                                                                                  Page 282 of 1169
lcr_xlayer_color_info_present_flag[ i ][ j ] indicates the presence of the global, if i is equal to 1, or
local, if i is equal to 0, lcr_xlayer_color_info( i, j) syntax in the extended layer information for extended
layer id j. If lcr_xlayer_color_info_present_flag[ i ][ j ] is equal to 1, the corresponding
lcr_xlayer_color_info( i, j) syntax is present, otherwise, this syntax is not present.

lcr_embedded_layer_info_present_flag[ i ][ j ] indicates the presence of the global, if i is equal to 1, or
local, if i is equal to 0, lcr_embedded_layer_info( i, j) syntax in the extended layer information for
extended layer id j. If lcr_embedded_layer_info_present_flag[ i ][ j ] is equal to 1, the corresponding
lcr_embedded_layer_info( i, j) syntax is present, otherwise, this syntax is not present.

lcr_xlayer_purpose_id[ i ][ j ] specifies the application purpose for the extended layer with id j, in a
global, if i is equal to 1, or a local, if i is equal to 0, LCR with the same semantics as for
lcr_global_purpose_id. When the syntax elements lcr_xlayer_purpose_id[ i ][ j ] and lcr_global_purpose_id
are not present then lcr_xlayer_purpose_id[ i ][ j ] is set to 0 (Unspecified).

lcr_xlayer_atlas_segment_id[ j ] indicates the corresponding atlas segment ID that the extended layer
with index j in the global LCR is associated with. If lcr_xlayer_atlas_segment_id[ j ] is not present, such
association can be provided in the embedded layer information, can be specified through external means,
or can be unspecified.

lcr_xlayer_priority_order[ j ] indicates the priority order of an extended layer with index j when
rendering it on an atlas compared to other extended layers. The lower the value of
lcr_xlayer_priority_order[ j ] the higher the priority rendering order of that layer compared to other
layers with a higher value. If this information is missing or two or more layers have the same priority
value, then the priority between them is determined based on the extended layer ID of the layers (the
lower ID value has a higher rendering priority than a higher ID value). Layers with a higher rendering
priority value are rendered first compared to layers with a lower rendering priority value when placed on
an atlas.

lcr_xlayer_rendering_method[ j ] indicates the rendering method applied to the extended layer j
compared to previously rendered layers according to their priority order value. The interpretation of the
value of lcr_xlayer_rendering_method[ j ] for rendering purposes is shown below:

                               Table 6.7: Extended layer rendering methods

                       lcr_xlayer_rendering_method                                    Interpretation

 0                                                                        Overwrite

 1                                                                        Blend 50%

 2                                                                        Multiply

 3                                                                        Darken

 4                                                                        Lighten

 5-255                                                                    Reserved


Values corresponding to a reserved interpretation are for future use by AOMedia. They shall be ignored
by decoders conforming to this version of this specification.




AV2 Specification                                                                                 Page 283 of 1169
```

<a id="s-6-8-8"></a>

#### § 6.8.8 LCR rep info semantics

```text
§   6.8.8. LCR rep info semantics

    lcr_max_pic_width[ i ][ j ] specifies the maximum picture width for the decoded pictures associated
    with the extended layer j in either a global, when i is equal to 1, or a local, when i is equal to 0, LCR OBU.
    The value of lcr_max_pic_width[ i ][ j ] in an activated LCR OBU in an extended layer with index j shall
    equal max_frame_width_minus_1 + 1.

    lcr_max_pic_height[ i ][ j ] specifies the maximum picture height for the decoded pictures associated
    with the extended layer j in either a global, when i is equal to 1, or a local, when i is equal to 0, LCR OBU.
    The value of lcr_max_pic_height[ i ][ j ] in an activated LCR OBU in an extended layer with index j shall
    equal max_frame_height_minus_1 + 1.

    lcr_format_info_present_flag[ i ][ j ] specifies the presence of the lcr_bit_depth_idc[ i ][ j] and
    lcr_chroma_format_idc[ i ][ j ] syntax elements that indicate the bitdepth and chroma format of the
    decoded pictures associated with the extended layer j in either a global, when i is equal to 1, or a local,
    when i is equal to 0, LCR OBU. If lcr_format_info_present_flag[ i ][ j ] is 1, then the syntax elements
    lcr_bit_depth_idc[ i ][ j ] and lcr_chroma_format_idc[ i ][ j ] are present in the LCR OBU. If
    lcr_format_info_present_flag[ i ][ j ] is 0, then the syntax elements lcr_bit_depth_idc[ i ][ j ] and
    lcr_chroma_format_idc[ i ][ j ] are not present in the LCR OBU.

    lcr_cropping_window_present_flag[ i ][ j ] specifies the presence of a cropping window that should be
    applied to the decoded pictures associated with the extended layer j in either a global, when i is equal to
    1, or a local, when i is equal to 0, LCR OBU, after upscaling such pictures to a width of
    lcr_max_pic_width[ i ][ j ] and to a height of lcr_max_pic_height[ i ][ j ]. The value of
    lcr_cropping_window_present_flag[ i ][ j ], when present in an activated LCR OBU in an extended layer
    with index j shall equal seq_cropping_window_present_flag.

    lcr_bit_depth_idc[ i ][ j ] specifies the bit_depth for the decoded pictures associated with the extended
    layer j in either a global, when i is equal to 1, or a local, when i is equal to 0, LCR OBU. The value of
    lcr_bit_depth_idc[ i ][ j ] in an activated LCR OBU in an extended layer with index j shall equal
    bit_depth_idc.

    lcr_chroma_format_idc[ i ][ j ] specifies the chroma format idc for the decoded pictures associated
    with the extended layer j in either a global, when i is equal to 1, or a local, when i is equal to 0, LCR OBU.
    The value of lcr_chroma_format_idc[ i ][ j ] in an activated LCR OBU in an extended layer with index j
    shall equal chroma_format_idc.

    lcr_cropping_win_left_offset[ i ][ j ], lcr_cropping_win_right_offset[ i ][ j ],
    lcr_cropping_win_top_offset[ i ][ j ], and lcr_cropping_win_bottom_offset[ i ][ j ] specify the
    cropping window that should be used to generate the output of the decoding process in combination with
    the lcr_max_pic_width[ i][ j ] and lcr_max_pic_height[ i][ j ] syntax elements, using the decoded pictures
    associated with the extended layer j in either a global, when i is equal to 1, or a local, when i is equal to
    0, LCR OBU. The values of lcr_cropping_win_left_offset[ i ][ j ], lcr_cropping_win_right_offset[ i ][ j ],
    lcr_cropping_win_top_offset[ i ][ j ], and lcr_cropping_win_bottom_offset[ i ][ j ] in an activated LCR OBU
    in an extended layer with index j shall match the values of seq_cropping_win_left_offset,
    seq_cropping_win_right_offset, seq_cropping_win_top_offset, and seq_cropping_win_bottom_offset.

```

<a id="s-6-8-9"></a>

#### § 6.8.9 LCR embedded layer info semantics

```text
§   6.8.9. LCR embedded layer info semantics

    lcr_mlayer_map[ isGlobal ][ xId ] specifies a map that indicates which embedded layers are present in
    the extended layer with ID equal to xId.



    AV2 Specification                                                                              Page 284 of 1169
lcr_tlayer_map[ isGlobal ][ xId ][ j ] specifies a map that indicates which temporal layers are present
in the extended layer with ID equal to xId for the current embedded layer with ID equal to j.

It is a requirement of bitstream conformance that the indication of the dependency information for each
extended layer with obu_xlayer_id equal to xId, in the activated LCR OBU, denoted by
lcr_mlayer_map[ isGlobal ][ xId ] and lcr_tlayer_map[ isGlobal ][ xId ][ cMId ], if present, shall agree with
the equivalent indication in the activated sequence header, denoted by MlayerDependencyMap[ cMId ]
[ rMId ] and TlayerDependencyMap[ cMId ][ cTId ][ rTId ], so that:

  • If isGlobal is equal to 0, for any embedded layer with ID equal to cMId if
    MLayerDependencyMap[ cMId ][ rMId ] is equal to 1 and lcr_mlayer_map[ 0 ][ xId ] & (1 << cMId) is
    greater than 0, lcr_mlayer_map[ 0 ][ xId ] & (1 << rMId) shall not be equal to 0 for all non-negative rMId
    less than cMId.
  • If isGlobal is equal to 1, for any embedded layer with ID equal to cMId if
    MLayerDependencyMap[ cMId ][ rMId ] is equal to 1 and lcr_mlayer_map[ 1 ][ xId ] & (1 << cMId) is
    greater than 0, lcr_mlayer_map[ 1 ][ xId ] & (1 << rMId) shall not be equal to 0 for all non-negative rMId
    less than cMId.
  • If isGlobal is equal to 0, for any embedded layer with ID equal to cMId and temporal layer with ID
    equal to cTId, if TLayerDependencyMap[ cMId ][ cTId ][ rTId ] is equal to 1 and lcr_tlayer_map[ 0 ]
    [ xId ][ cMId ] & (1 << cTId) is greater than 0, lcr_tlayer_map[ 0 ][ xId ][ cMId ] & (1 << rTId) shall not
    be equal to 0 for all non-negative rTId less than cTId.
  • If isGlobal is equal to 1, for any embedded layer with ID equal to cMId and temporal layer with ID
    equal to cTId, if TLayerDependencyMap[ cMId ][ cTId ][ rTId ] is equal to 1 and lcr_tlayer_map[ 1 ]
    [ xId ][ cMId ] & (1 << cTId) is greater than 0, lcr_tlayer_map[ 1 ][ xId ][ cMId ] & (1 << rTId) shall not
    be equal to 0 for all non-negative rTId less than cTId.


  NOTE: Above bitstream constraints on lcr_mlayer_map (and similarly for lcr_tlayer_map based on
  TLayerDependencyMap) make sure that, if MLayerDependencyMap[ cMId ][ rMId ] is equal to 1, any
  embedded layer with ID rMId referenced from the existing embedded layer with ID cMId are
  indicated to be present in the activated LCR. Otherwise, if MLayerDependencyMap[ cMId ][ rMId ] is
  equal to 0, indicating that an embedded layer with ID cMId does not depend on an embedded layer
  with ID rMId, lcr_mlayer_map[ isGlobal ][ xId ] is allowed to indicate that the embedded layer with ID
  rMId may or may not be present.


lcr_layer_atlas_segment_id[ isGlobal ][ xId ][ j ] specifies the atlas segment ID with which the
current embedded layer with obu_mlayer_id equal to j in the extended layer with obu_xlayer_id equal to
xId is associated.

lcr_priority_order[ isGlobal ][ xId ][ j ] indicates the priority order of an embedded layer with ID j in
an extended layer with ID xId when rendering it on an atlas compared to other embedded layers. The
lower the value of lcr_priority_order[ isGlobal ][ xId ][ j ] the higher the priority rendering order of that
layer compared to other layers with a higher value. If this information is missing or two or more layers
have the same priority value, then the priority between them is determined based on the embedded layer
ID followed by the extended layer ID of the layers (the lower ID value has a higher rendering priority
than a higher ID value). Layers with a higher rendering priority value are rendered first compared to
layers with a lower rendering priority value when placed on an atlas.




AV2 Specification                                                                                Page 285 of 1169
lcr_rendering_method[ isGlobal ][ xId ][ j ] indicates the rendering method applied to the embedded
layer with ID j in the extended layer with ID xId compared to previously rendered layers according to
their priority order value. The interpretation of the value of lcr_rendering_method is the same as for
lcr_xlayer_rendering_method.

lcr_layer_type[ isGlobal ][ xId ][ j ] indicates the type of the embedded layer with ID j in the extended
layer with ID xId as specified in Table 6.8:

                               Table 6.8: Layer type values for LCR embedded layers

             lcr_layer_type                                Label                             Interpretation

 0                                       TEXTURE_LAYER                          Texture

 1                                       AUX_LAYER                              Auxiliary

 2-255                                   -                                      Reserved


Reserved values of lcr_layer_type[ isGlobal ][ xId ][ j ] are for future use by AOMedia. They shall be
ignored by decoders conforming to this version of this specification.

lcr_auxiliary_type[ isGlobal ][ xId ][ j ] indicates the auxiliary type of the embedded layer with ID j in
the extended layer with ID xId as specified in Table 6.9:

                              Table 6.9: Auxiliary type values for LCR embedded layers

         lcr_auxiliary_type                        Label                              Interpretation

 0                                  ALPHA_AUX                        Alpha auxiliary image

 1                                  DEPTH_AUX                        Depth auxiliary image

 2                                  SEGMENTATION_AUX                 Segmentation auxiliary image

 3                                  GAIN_MAP_AUX                     Gain map auxiliary image

 4–127                              -                                Reserved

 128–159                            -                                Unspecified

 160–255                            -                                Reserved



  NOTE: The interpretation of auxiliary layers with lcr_auxiliary_type in the range 128 to 159,
  inclusive, is specified through means external to the bitstream (e.g., container metadata or
  application-layer signaling).


lcr_auxiliary_type[ isGlobal ][ xId ][ j ] shall be in the range of 0 to 3, inclusive, or 128 to 159, inclusive,
for bitstreams conforming to this specification. Decoders shall ignore auxiliary layers whose
lcr_auxiliary_type[ isGlobal ][ xId ][ j ] value is reserved or whose interpretation is not known through
external means.

lcr_view_type[ isGlobal ][ xId ][ j ] indicates the view type of the embedded layer with ID j in the
extended layer with ID xId as specified in Table 6.10:

                               Table 6.10: View type values for LCR embedded layers

     lcr_view_type                      Label                                   Interpretation




AV2 Specification                                                                                       Page 286 of 1169
     0                    VIEW_UNSPECIFIED              The view type is undefined or not specified

     1                    VIEW_CENTER                   Central perspective view

     2                    VIEW_LEFT                     View from the left perspective

     3                    VIEW_RIGHT                    View from the right perspective

     4                    VIEW_EXPLICIT                 Explicit view ID indication

     5-255                -                             Reserved


    Reserved values of lcr_view_type[ isGlobal ][ xId ][ j ] are for future use by AOMedia. They shall be
    ignored by decoders conforming to this version of this specification.

    lcr_view_id[ isGlobal ][ xId ][ j ] indicates the view id associated with the embedded layer with ID j in
    the extended layer with ID xId.

    lcr_dependent_layer_map[ isGlobal ][ xId ][ j ] indicates with which embedded layers the current
    embedded layer with layer ID equal to j, in the extended layer xId, depends on in terms of inter
    prediction. If lcr_dependent_layer_map[ isGlobal ][ xId ][ j ] is equal to 0, then the current embedded
    layer can be independently decoded from other embedded layers.

    lcr_same_sh_max_resolution_flag[ isGlobal ][ xId ][ j ] equal to 1, or not present, indicates that for
    the embedded layer with obu_mlayer_id equal to j in the extended layer with obu_xlayer_id equal to xId in
    an activated LCR OBU, the resolution limits for that layer are set equal to those in the activated sequence
    header, i.e., equal to max_frame_width_minus_1 + 1 and max_frame_height_minus_1 + 1 respectively. In
    that case the syntax elements lcr_max_expected_width[ isGlobal ][ xId ][ j ] and
    lcr_max_expected_height[ isGlobal ][ xId ][ j ] are not present.

    lcr_max_expected_width[ isGlobal ][ xId ][ j ] in an activated LCR OBU specifies the maximum
    expected FrameWidth for all frames in embedded layer j of extended layer xId. It is a requirement of
    bitstream conformance that FrameWidth for all frames in embedded layer j of extended layer xId shall be
    less than or equal to lcr_max_expected_width[ isGlobal ][ xId ][ j ]. It is also a requirement of bitstream
    conformance that lcr_max_expected_width[ isGlobal ][ xId ][ j ] shall be less than or equal to
    max_frame_width_minus_1 + 1 obtained from the activated sequence header.

    lcr_max_expected_height[ isGlobal ][ xId ][ j ] in an activated LCR OBU specifies the maximum
    expected FrameHeight for all frames in embedded layer j of extended layer xId. It is a requirement of
    bitstream conformance that FrameHeight for all frames in embedded layer j of extended layer xId shall
    be less than or equal to lcr_max_expected_height[ isGlobal ][ xId ][ j ]. It is also a requirement of
    bitstream conformance that lcr_max_expected_height[ isGlobal ][ xId ][ j ] shall be less than or equal to
    max_frame_height_minus_1 + 1 obtained from the activated sequence header.

```

<a id="s-6-8-10"></a>

#### § 6.8.10 LCR xlayer color info semantics

```text
§   6.8.10. LCR xlayer color info semantics

    layer_color_description_idc, layer_color_primaries, layer_matrix_coefficients,
    layer_transfer_characteristics, layer_full_range_flag specify the color information for this layer with
    the same interpretation as ops_color_description_idc, ops_color_primaries, ops_matrix_coefficients,
    ops_transfer_characteristics and ops_full_range_flag.




    AV2 Specification                                                                                 Page 287 of 1169
```

<a id="s-6-9"></a>

### § 6.9 Atlas segment info OBU semantics

```text
§   6.9. Atlas segment info OBU semantics
```

<a id="s-6-9-1"></a>

#### § 6.9.1 General

```text
§   6.9.1. General

    The Atlas Segment provides spatial layout and composition information for organizing multiple layers into
    a unified visual presentation. An atlas defines a virtual canvas or coordinate space onto which different
    video layers can be mapped, positioned, and composed. The atlas mechanism serves several key
    purposes:

    Spatial composition and layout: An atlas specifies how multiple decoded video layers should be
    arranged in 2D space to form the final rendered output. Each atlas segment represents a rectangular
    region that can be populated by content from one or more video layers. The atlas defines:

      • The nominal dimensions of the virtual canvas (signaled as ats_nominal_width_minus_1 + 1 and
        ats_nominal_height_minus_1 + 1)

      • How the canvas is divided into regions (column and row grid)
      • Which input streams (layers) contribute to each region
      • The position and size of each layer’s content within the atlas space

    Multi-layer composition modes: The atlas supports several composition modes through
    ats_atlas_segment_mode_idc:


      • Enhanced Atlas (mode 0): Defines the atlas as a 2D grid of rectangular regions that are grouped
        into segments; stream-to-segment association is indicated in the LCR via lcr_layer_atlas_segment_id,
        enabling multiple layers to share a single segment
      • Region-based layout (mode 1): Divides the atlas into a grid of regions that can be uniformly or non-
        uniformly spaced, enabling regular tiling patterns
      • Basic composition (mode 2): Direct mapping of input streams to rectangular regions within the
        atlas
      • Multistream composition (mode 3): Composes multiple independent video streams into a single
        atlas, with optional background filling
      • Multistream with alpha (mode 4): Like mode 3 but with per-segment alpha channel support for
        transparency

    Subpicture and region-of-interest support: The atlas is particularly powerful for subpicture
    applications where different regions of interest are encoded as separate layers. For example, in a video
    conferencing scenario, the atlas might define a 1920x1080 virtual screen where:

      • Segment 0 maps to layer 0: main speaker at position (0, 0) with size 960x1080
      • Segment 1 maps to layer 1: participant thumbnails at position (960, 0) with size 960x540
      • Segment 2 maps to layer 2: shared content at position (960, 540) with size 960x540

    Each segment can be independently decoded and positioned, enabling selective decoding and rendering
    based on viewport or bandwidth constraints.

    Relationship with LCR and MSDO: The atlas works in conjunction with either the Layer Configuration
    Record (LCR) or the Multi Stream Decoder Operation (MSDO) OBU to define the complete layer
    structure. While the LCR describes the semantic properties of each layer (texture vs auxiliary, view


    AV2 Specification                                                                            Page 288 of 1169
association, layer type), the atlas describes the geometric properties (position, size, spatial relationships).
Layers are associated with atlas segments through lcr_layer_atlas_segment_id in the LCR, creating the link
between semantic layer metadata and spatial layout information.

Alternatively, when using MSDO instead of LCR, the atlas provides spatial layout information for the
extended layers defined in the MSDO OBU. In this case, each extended layer identified by sub_xlayer_id[i]
in the MSDO corresponds to an input stream in the atlas segment description (via ats_input_stream_id or
ats_msi_input_stream_id), and the atlas defines how these independently decodable extended layers are
spatially composed. The MSDO approach provides a simpler layer identification mechanism suitable for
applications where extended layers represent complete, independently decodable views or streams that
are spatially composed using the atlas.

Virtual canvas rendering: The atlas can represent a virtual image larger than any individual layer,
which is particularly useful for:

  • Viewport-dependent streaming where different regions are encoded at different qualities
  • Subpicture video where high-resolution content is split into independently decodable subpictures
  • Multi-view displays where different viewing positions see different subsets of the atlas
  • Scalable region-of-interest encoding where focus areas get higher quality encoding

For detailed usage examples including stereoscopic composition, subpicture layouts, and multi-view
scenarios, see Annex G: Layer composition and Atlas usage examples (informative).

atlas_segment_id indicates the atlas segment id associated with the current atlas segment information
OBU, which can be referred by other syntax structures in this specification.

ats_atlas_segment_mode_idc specifies the representation description and coding of the atlas segments
as specified in Table 6.11:

          Table 6.11: Specifies the representation description and coding of the atlas segments

     ats_atlas_segment_mode_idc                    Label                               Description

 0                                  ENHANCED_ATLAS                     Enhanced Atlas description

 1                                  BASIC_ATLAS                        Basic Atlas description

 2                                  SINGLE_ATLAS                       Single Atlas description

 3                                  MULTISTREAM_ATLAS                  Multistream Atlas description

 4                                  MULTISTREAM_ALPHA_ATLAS            Multistream Alpha Atlas description


It is a requirement of bitstream conformance that ats_atlas_segment_mode_idc is less than or equal to 4.

It is a requirement of bitstream conformance that when ats_atlas_segment_mode_idc[ xAId ] is equal to
MULTISTREAM_ATLAS or MULTISTREAM_ALPHA_ATLAS, obu_xlayer_id is equal to
GLOBAL_XLAYER_ID.

ats_nominal_width_minus_1 plus 1 specifies the nominal width of the atlas.

ats_nominal_height_minus_1 plus 1 specifies the nominal height of the atlas.




AV2 Specification                                                                                    Page 289 of 1169
```

<a id="s-6-9-2"></a>

#### § 6.9.2 Atlas label segment info semantics

```text
§   6.9.2. Atlas label segment info semantics

    ats_signaled_atlas_segment_ids_flag indicates whether the atlas segments are assigned explicit IDs or
    these are set equal to their index. When ats_signaled_atlas_segment_ids_flag is equal to 1, then explicit
    IDs are assigned to each atlas segment. If ats_signaled_atlas_segment_ids_flag is equal to 0, then the ID
    of each atlas segment is equal to its index.

    ats_atlas_segment_id[ xlayerId ][ xAId ][ i ] indicates the ID associated with the atlas segment with
    index i.

```

<a id="s-6-9-3"></a>

#### § 6.9.3 Atlas enhanced atlas info semantics

```text
§   6.9.3. Atlas enhanced atlas info semantics

    The Enhanced Atlas (ats_atlas_segment_mode_idc == ENHANCED_ATLAS) describes the spatial layout of an atlas as
    a two-dimensional grid of rectangular regions. The ats_enhanced_atlas_info syntax structure is the top-level
    container for this description; it calls ats_region_info to define the grid geometry and
    ats_region_to_segment_mapping to group grid regions into named atlas segments.


    Purpose and spatial layout: The atlas grid divides the virtual canvas into (ats_num_region_columns_minus_1
    + 1) columns and (ats_num_region_rows_minus_1 + 1) rows. Each cell of the grid is a rectangular region.
    When ats_uniform_spacing_flag is equal to 1, all regions have the same width and height. When it is equal
    to 0, each column width and row height is signaled individually, enabling non-uniform layouts such as a
    large main area flanked by smaller participant windows. One or more adjacent rectangular groups of
    regions are then combined into atlas segments by ats_region_to_segment_mapping.

    Association with the LCR: The segment IDs assigned by ats_enhanced_atlas_info (either implicitly as
    indices 0, 1, 2, … or explicitly via ats_label_segment_info when ats_signaled_atlas_segment_ids_flag is equal to
    1) are the values that decoders must match against lcr_layer_atlas_segment_id[ isGlobal ][ xId ][ j ] in the
    Layer Configuration Record. When lcr_local_atlas_id_present_flag[ xId ] is equal to 1, the local LCR for
    extended layer xId identifies its associated atlas via lcr_local_atlas_id[ xId ], and each embedded layer j
    within that extended layer indicates which atlas segment it contributes to through
    lcr_layer_atlas_segment_id. This is the sole mechanism by which the Enhanced Atlas resolves which layer
    provides content for a given segment — no stream identifiers are present in the atlas itself.

    Multiple layers per segment: Because the mapping is expressed in the LCR rather than in the atlas,
    multiple embedded layers from the same or different extended layers may reference the same segment
    ID. This supports co-located auxiliary data: for example, a texture layer, an alpha layer, and a depth layer
    for the same spatial region all carry the same lcr_layer_atlas_segment_id. The rendering order among
    layers sharing a segment is controlled by lcr_priority_order, and the compositing operation by
    lcr_rendering_method.

```

<a id="s-6-9-3-1"></a>

##### § 6.9.3.1 Atlas region info semantics

```text
§   6.9.3.1. Atlas region info semantics

    ats_num_region_columns_minus_1[ xAId ] plus 1 specifies the number of column regions to which an
    atlas with ID equal to xAId needs to be segmented.

    It is a requirement of bitstream conformance that ats_num_region_columns_minus_1 is less than
    MAX_ATLAS_COLS.

    ats_num_region_rows_minus_1[ xAId ] plus 1 specifies the number of row regions to which an atlas
    with ID equal to xAId needs to be segmented.




    AV2 Specification                                                                                Page 290 of 1169
    It is a requirement of bitstream conformance that ats_num_region_rows_minus_1 is less than
    MAX_ATLAS_ROWS.

    ats_uniform_spacing_flag[ xAId ] equal to 1 specifies that the regions to which an atlas is segmented
    are uniformly spaced. ats_uniform_spacing_flag[ xAId ] equal to 0 specifies that the atlas regions are not
    uniformly spaced and the region widths and heights are signaled individually.

    ats_column_width_minus_1[ xAId ][ i ] plus 1 indicates the width of the regions in column i in the
    atlas with ID xAId.

    ats_row_height_minus_1[ xAId ][ i ] plus 1 indicates the height of the regions in row i in the atlas with
    ID xAId.

    ats_region_width_minus_1[ xAId ] plus 1 indicates the width of all regions in the atlas with ID xAId.

    ats_region_height_minus_1[ xAId ] plus 1 indicates the height of all regions in the atlas with ID xAId.

```

<a id="s-6-9-3-2"></a>

##### § 6.9.3.2 Atlas region to segment mapping semantics

```text
§   6.9.3.2. Atlas region to segment mapping semantics

    ats_single_region_per_atlas_segment_flag[ xAId ] indicates whether there is one to one mapping of
    atlas regions with atlas segments.

    If ats_single_region_per_atlas_segment_flag[ xAId ] is equal to 0, then the mapping of atlas regions with
    atlas segments is not one to one.

    If ats_single_region_per_atlas_segment_flag[ xAId ] is equal to 1, then the mapping of atlas regions with
    atlas segments is one to one.

    If ats_single_region_per_atlas_segment_flag[ xAId ] is equal to 1, it is a requirement of bitstream
    conformance that NumRegionsInAtlas[ xAId ] is less than or equal to MAX_NUM_ATLAS_SEGMENTS.

    ats_top_left_region_column[ xAId ][ i ] indicates the column of the first region associated with the
    segment with index i.

    ats_top_left_region_row[ xAId ][ i ] indicates the row of the first region associated with the segment
    with index i.

    ats_bottom_right_region_column_off[ xAId ][ i ] indicates the offset for the column of the last region
    associated with the segment with index i. The column of the last region is derived as
    ats_top_left_region_column[ xAId ][ i ] + ats_bottom_right_region_column_off[ xAId ][ i ].

    ats_bottom_right_region_row_off[ xAId ][ i ] indicates the offset for the row of the last region
    associated with the segment with index i. The row of the last region is derived as
    ats_top_left_region_row[ xAId ][ i ] + ats_bottom_right_region_row_off[ xAId ][ i ].


      NOTE: The semantics of ats_num_atlas_segments_minus_1 are provided in § 6.9.6 Atlas basic info
      semantics.




    AV2 Specification                                                                            Page 291 of 1169
```

<a id="s-6-9-4"></a>

#### § 6.9.4 Atlas multistream info semantics

```text
§   6.9.4. Atlas multistream info semantics

      NOTE: An informative composition process for MULTISTREAM_ATLAS and
      MULTISTREAM_ALPHA_ATLAS modes is described in Annex D: Multistream composition process
      (informative).


    ats_msi_input_stream_id, ats_msi_width, ats_msi_height, ats_msi_num_atlas_segments_minus_1,
    ats_msi_segment_top_left_pos_x, ats_msi_segment_top_left_pos_y, ats_msi_segment_width, and
    ats_msi_segment_height have the same semantics as ats_input_stream_id, ats_width, ats_height,
    ats_num_atlas_segments_minus_1, ats_segment_top_left_pos_x, ats_segment_top_left_pos_y,
    ats_segment_width, and ats_segment_height in the Atlas basic info semantics § 6.9.6 Atlas basic info
    semantics.

    ats_msi_background_info_present_flag equal to 1 specifies that the syntax elements
    ats_msi_background_red_value, ats_msi_background_green_value, and ats_msi_background_blue_value
    are present. ats_msi_background_info_present_flag equal to 0 specifies the syntax elements are not
    present.

    ats_msi_background_red_value specifies the red component of the background color as the 8-bit
    quantized value (D’R) in Recommendation ITU-R BT.709. When ats_msi_background_red_value is not
    present, it is inferred to be equal to 16.

    ats_msi_background_green_value specifies the green component of the background color as the 8-bit
    quantized value (D’G) in Recommendation ITU-R BT.709. When ats_msi_background_green_value is not
    present, it is inferred to be equal to 16.

    ats_msi_background_blue_value specifies the blue component of the background color as the 8-bit
    quantized value (D’B) in Recommendation ITU-R BT.709. When ats_msi_background_blue_value is not
    present, it is inferred to be equal to 16.

```

<a id="s-6-9-5"></a>

#### § 6.9.5 Atlas multistream with alpha info semantics

```text
§   6.9.5. Atlas multistream with alpha info semantics

    ats_msi_alpha_segments_present_flag equal to 1 specifies that the syntax element
    ats_msi_alpha_segment_flag is present in the bitstream. ats_msi_alpha_segments_present_flag equal to 0
    specifies that the syntax element is not present.

    ats_msi_alpha_segment_flag[ xlayerId ][ xAId ][ i ] specifies that the atlas segment with index i is an
    alpha frame. When not present, ats_msi_alpha_segment_flag[ xlayerId ][ xAId ][ i ] shall be inferred to be
    equal to 0.


      NOTE: The semantics of ats_msi_input_stream_id, ats_msi_width, ats_msi_height,
      ats_msi_num_atlas_segments_minus_1, ats_msi_segment_top_left_pos_x,
      ats_msi_segment_top_left_pos_y, ats_msi_segment_width, ats_msi_segment_height,
      ats_msi_background_info_present_flag, ats_msi_background_red_value,
      ats_msi_background_green_value, and ats_msi_background_blue_value are provided in § 6.9.4 Atlas
      multistream info semantics.

```

<a id="s-6-9-6"></a>

#### § 6.9.6 Atlas basic info semantics

```text
§   6.9.6. Atlas basic info semantics

    ats_stream_id_present[ xAId ] indicates ats_input_stream_id is signaled.



    AV2 Specification                                                                           Page 292 of 1169
    ats_width[ xAId ] indicates the width of the atlas with ID xAId.

    ats_height[ xAId ] indicates the height of the atlas with ID xAId.

    ats_num_atlas_segments_minus_1[ xAId ] plus one indicates the number of atlas segments of the atlas
    with ID xAId.

    It is a requirement of bitstream conformance that ats_num_atlas_segments_minus_1 is less than
    MAX_NUM_ATLAS_SEGMENTS.

    ats_input_stream_id[ xAId ][ i ] specifies the obu_xlayer_id value of the stream corresponding to the i-
    th composed region.

    All values in ats_input_stream_id[ xAId ][] shall be unique.

    ats_segment_top_left_pos_x[ xAId ][ i ] indicates the horizontal coordinate of the top left position of
    the atlas segment with index i.

    ats_segment_top_left_pos_y[ xAId ][ i ] indicates the vertical coordinate of the top left position of the
    atlas segment with index i.

    ats_segment_width[ xAId ][ i ] indicates the width of the atlas segment with index i.

    ats_segment_height[ xAId ][ i ] indicates the height of the atlas segment with index i.

```

<a id="s-6-10"></a>

### § 6.10 Operating point set OBU semantics

```text
§   6.10. Operating point set OBU semantics
```

<a id="s-6-10-1"></a>

#### § 6.10.1 General

```text
§   6.10.1. General

    The Operating Point Set (OPS) OBU indicates possible decoding operating points associated with the
    bitstream.

    Each OPS OBU is associated with an extended layer via obu_xlayer_id:

      • When obu_xlayer_id is equal to GLOBAL_XLAYER_ID (31), the OPS applies to the entire multistream
        (global OPS).
      • When obu_xlayer_id is less than GLOBAL_XLAYER_ID, the OPS applies to that specific extended layer
        (local OPS).

    OPS are identified by the pair (obu_xlayer_id, ops_id). Up to 16 OPS can be defined per extended layer
    (ops_id is a 4-bit value), each containing up to 7 operating points (ops_cnt is a 3-bit value with 0 reserved
    for reset). In a multistream with up to 31 extended layers and 16 OPS each, up to 496 total OPS are
    possible. Singlestream bitstreams support up to 16 OPS.

    Each OPS groups operating points sharing a common ops_intent (e.g., scalability, stereo, gain map).
    Applications can:

     1. First filter OPS by intent to find relevant sets.
     2. Then examine individual operating points for detailed selection based on profile/level/tier, color info,
        decoder model info, and layer maps.
     3. Consider multiple OPS simultaneously when needed.




    AV2 Specification                                                                              Page 293 of 1169
    The reset and update behavior of the OPS OBU is determined by the combination of ops_reset_flag and
    ops_cnt:

     1. ops_reset_flag equal to 1 and ops_cnt equal to 0: All OPS for the associated extended layer (or all
        layers if global) are reset. No OPS remains active.
     2. ops_reset_flag equal to 1, ops_id equal to x, and ops_cnt equal to N (N > 0): All OPS are reset, then
        OPS x is defined with N operating points.
     3. ops_reset_flag equal to 0, ops_id equal to x, and ops_cnt equal to 0: Only OPS x is reset. Other OPS
        remain active.
     4. ops_reset_flag equal to 0, ops_id equal to x, and ops_cnt equal to N (N > 0): OPS x is set or updated
        with N operating points. Other OPS are unchanged.

    OPS information persists across coded video sequences. As informative guidance (not a normative
    requirement): a decoder that selects an operating point for the duration of a coded video sequence may
    only switch to an operating point that is a subset of the current one (downgrading is permitted;
    upgrading to decode additional layers is not, since the required data may not be available).

    OPS processing is entirely optional. A decoder may ignore all OPS information and decode the entire
    bitstream.

```

<a id="s-6-10-2"></a>

#### § 6.10.2 Operating point set OBU syntax elements

```text
§   6.10.2. Operating point set OBU syntax elements

    ops_reset_flag[ obu_xlayer_id ] equal to 1 specifies that all operating point sets associated with
    obu_xlayer_id are reset. ops_reset_flag equal to 0 specifies that the operating point sets associated with
    obu_xlayer_id are not reset. The specific behavior depends on the combination with ops_cnt as described
    in § 6.10.1 General.

    ops_id[ obu_xlayer_id ] specifies the operating point set identifier within the extended layer given by
    obu_xlayer_id. The value of ops_id is in the range of 0 to 15, inclusive.

    ops_cnt[ obu_xlayer_id ][ opsID ] specifies the number of operating points in the OPS identified by
    opsID within the extended layer given by obu_xlayer_id. When ops_cnt is equal to 0, the OPS is being
    reset or cleared as described in § 6.10.1 General. When ops_cnt is greater than 0, it specifies the number
    of operating points (1 to 7).

    ops_priority[ obu_xlayer_id ][ opsID ] specifies the priority of the OPS identified by opsID within the
    extended layer given by obu_xlayer_id. Lower values indicate higher priority.

    When ops_priority[ obu_xlayer_id ][ opsID ] is not present, ops_priority [ obu_xlayer_id ][ opsID ] shall be
    inferred to be equal to 0.

    ops_intent[ obu_xlayer_id ][ opsID ] specifies the intent of the OPS at the opsID within the
    obu_xlayer_id as specified in Table 6.12:

                                     Table 6.12: ops_intent values and labels

                    ops_intent                                            Label

     0                                    OPSI_UNSPECIFIED

     1                                    OPSI_SCALABILITY

     2                                    OPSI_STEREO



    AV2 Specification                                                                              Page 294 of 1169
 3                                   OPSI_TEXTURE_ALPHA

 4                                   OPSI_TEXTURE_DEPTH

 5                                   OPSI_GAIN_MAP

 6                                   OPSI_MULTIVIEW

 7-127                               RESERVED


When ops_intent[ obu_xlayer_id ][ opsID ] is not present, ops_intent[ obu_xlayer_id ][ opsID ] shall be
inferred to be equal to 0. Reserved values of ops_intent[ obu_xlayer_id ][ opsID ] are for future use by
AOMedia. They shall be ignored by decoders conforming to this version of this specification.

ops_intent_present_flag[ obu_xlayer_id ][ opsID ] equal to 1 specifies that ops_op_intent is present in
the current OPS. ops_intent_present_flag[ obu_xlayer_id ][ opsID ] equal to 0 specifies ops_op_intent is
not present in the current OPS.

ops_ptl_present_flag[ obu_xlayer_id ][ opsID ] equal to 1 specifies that profile, tier, and level
information is present for all the operating points within the OPS identified by opsID. When obu_xlayer_id
is equal to GLOBAL_XLAYER_ID, this information is conveyed via the ops_aggregate_info( ) and
ops_seq_profile_tier_level_info( ) syntax structures. When obu_xlayer_id is less than GLOBAL_XLAYER_ID,
this information is conveyed via the ops_seq_profile_tier_level_info( ) syntax structure.
ops_ptl_present_flag[ obu_xlayer_id ][ opsID ] equal to 0 specifies that profile, tier, and level information
is not present for the operating points within the OPS identified by opsID.

ops_color_info_present_flag[ obu_xlayer_id ][ opsID ] equal to 1 specifies that the
ops_color_info( opsID, i ) syntax is present in the current OPS.
ops_color_info_present_flag[ obu_xlayer_id ][ opsID ] equal to 0 specifies that the ops_color_info( opsID,
i ) syntax is not present in the current OPS.

ops_mlayer_info_idc[ opsID ] is present only for global OPS (i.e., when obu_xlayer_id ==
GLOBAL_XLAYER_ID). ops_mlayer_info_idc[ opsID ] equal to 0 specifies that the ops_mlayer_info syntax
structure is not present in the current OPS. ops_mlayer_info_idc[ opsID ] equal to 1 specifies that the
ops_mlayer_info syntax is present in the current OPS for every extended layer in each operating point.
ops_mlayer_info_idc[ opsID ] equal to 2 specifies that, for each extended layer in each operating point,
the ops_mlayer_info syntax is either present in the current OPS or inherited from another operating
point, as indicated by ops_mlayer_explicit_info_flag.

It is a requirement of bitstream conformance that ops_mlayer_info_idc[ opsID ] is not equal to 3.

ops_reserved_2bits must be set to 0. The value shall be ignored by a decoder.

ops_data_size[ obu_xlayer_id ][ opsID ][ i ] specifies the size in bytes of the i-th operating point
payload data. This value enables a decoder to skip over or validate individual operating point payloads.

ops_op_intent[ obu_xlayer_id ][ opsID ][ i ] specifies the intent of the i-th operating point with the
same semantics as ops_intent.

It is a requirement of bitstream conformance that when ops_ptl_present_flag[ obu_xlayer_id ][ opsID ] is
equal to 1, the bitstream corresponding to the i-th operating point associated with obu_xlayer_id and
opsID shall satisfy all bitstream constraints specified in Annex A.4 Levels, by setting seq_profile_idc,
seq_tier, and seq_level_idx to ops_seq_profile_idc[ obu_xlayer_id ][ opsID ][ i ][ j ],




AV2 Specification                                                                             Page 295 of 1169
ops_tier_flag[ obu_xlayer_id ][ opsID ][ i ][ j ], and ops_level_idx[ obu_xlayer_id ][ opsID ][ i ][ j ],
respectively, where j is the applicable layer index.

ops_decoder_model_info_for_this_op_present_flag[ xId ][ opsID ][ i ] equal to 1 specifies that the
ops_decoder_model_info( ) syntax structure is present for the i-th operating point.
ops_decoder_model_info_for_this_op_present_flag[ xId ][ opsID ][ i ] equal to 0 specifies that the
ops_decoder_model_info( ) syntax structure is not present.

ops_initial_display_delay_present_flag[ xId ][ opsID ][ i ] equal to 1 specifies that the
ops_initial_display_delay_minus_1[ xId ][ opsID ][ i ] syntax element is present.
ops_initial_display_delay_present_flag[ xId ][ opsID ][ i ] equal to 0 specifies that
ops_initial_display_delay_minus_1[ xId ][ opsID ][ i ] is not present.

ops_initial_display_delay_minus_1[ xId ][ opsID ][ i ] plus 1 specifies the number of decoded frames
that should be present in the buffer pool before the first presentable frame is displayed. This will ensure
that all presentable frames in the sequence can be decoded at or before the time that they are scheduled
for display.

ops_xlayer_map[ opsID ][ i ] specifies a 31-bit bitmask for the i-th operating point. Bit j being set to 1
indicates that extended layer j is included in the operating point. ops_xlayer_map[ opsID ][ i ] is present
and meaningful only for global OPS, i.e., when xId == GLOBAL_XLAYER_ID; for local OPS (xId !=
GLOBAL_XLAYER_ID) this syntax element is not present in the OPS OBU syntax.

ops_mlayer_explicit_info_flag[ opsID ][ i ][ j ] equal to 1 specifies that the ops_mlayer_info( ) syntax
structure is explicitly present for the j-th extended layer. ops_mlayer_explicit_info_flag[ opsID ][ i ][ j ]
equal to 0 specifies that the embedded layer and temporal layer information is inherited from the
operating point set and operating point index referenced by ops_embedded_ops_id[ opsID ][ i ][ j ] and
ops_embedded_op_index[ opsID ][ i ][ j ], respectively.

ops_embedded_ops_id[ opsID ][ i ][ j ] and ops_embedded_op_index[ opsID ][ i ][ j ] provide the
operating point set identifier and operating point index, respectively, from which the j-th extended layer
inherits its ops_mlayer_info configuration. This enables compact signaling when multiple operating points
share embedded layer and temporal layer structure.

Let refID be equal to ops_embedded_ops_id[ opsID ][ i ][ j ].

It is a requirement of bitstream conformance that ops_embedded_op_index[ opsID ][ i ][ j ] is less than
ops_cnt[ obu_xlayer_id ][ refID ].

If refID is equal to opsID, it is a requirement of bitstream conformance that
ops_embedded_op_index[ opsID ][ i ][ j ] is less than j.


  NOTE: These requirements ensure that the operating point is inherited from a previously received
  operating point.


opsBytes is a variable that contains the number of bytes read for the operating point.

It is a requirement of bitstream conformance that the computed value of opsBytes is equal to
ops_data_size[ obu_xlayer_id ][ opsID ][ i ].




AV2 Specification                                                                                    Page 296 of 1169
```

<a id="s-6-10-3"></a>

#### § 6.10.3 Operating point set aggregate info semantics

```text
§   6.10.3. Operating point set aggregate info semantics

    The aggregate information applies to global OPS (obu_xlayer_id equal to GLOBAL_XLAYER_ID) and
    describes the constraints for the combined multistream operating point.

    ops_config_idc[ opsID ][ i ] indicates the aggregate profile identifier for the i-th operating point in the
    OPS identified by opsID. This profile applies to the combined multistream operating point.

    ops_aggregate_level_idx[ opsID ][ i ] specifies the aggregate level indicator for the i-th operating point
    in the OPS identified by opsID. This level applies to the combined multistream operating point.

    ops_max_tier_flag[ opsID ][ i ] specifies the maximum tier indicator for the i-th operating point in the
    OPS identified by opsID. This tier applies to the combined multistream operating point.

    ops_max_interop[ opsID ][ i ] indicates the maximum interoperability point for the i-th operating point
    in the OPS identified by opsID.

```

<a id="s-6-10-4"></a>

#### § 6.10.4 Operating point set sequence profile tier level information semantics

```text
§   6.10.4. Operating point set sequence profile tier level information semantics

    The sequence profile tier level information describes per-extended-layer profile, level, and tier
    constraints for each extended layer included in an operating point.

    ops_seq_profile_idc[ xId ][ opsID ][ i ][ j ] specifies the profile indicator for the j-th extended layer in
    the i-th operating point of the OPS identified by opsID. This constrains the profile required to decode the
    j-th extended layer.

    ops_level_idx[ xId ][ opsID ][ i ][ j ] specifies the level indicator for the j-th extended layer in the i-th
    operating point of the OPS identified by opsID. This constrains the level required to decode the j-th
    extended layer.

    ops_tier_flag[ xId ][ opsID ][ i ][ j ] specifies the tier indicator for the j-th extended layer in the i-th
    operating point of the OPS identified by opsID. This constrains the tier required to decode the j-th
    extended layer.

    ops_mlayer_count[ xId ][ opsID ][ i ][ j ] specifies the number of embedded layers for the j-th
    extended layer in the i-th operating point of the OPS identified by opsID.

    ops_ptl_reserved_2bits must be set to 0. The value shall be ignored by a decoder.

```

<a id="s-6-10-5"></a>

#### § 6.10.5 Operating point set decoder model info semantics

```text
§   6.10.5. Operating point set decoder model info semantics

    ops_decoder_buffer_delay[ obu_xlayer_id ][ opsID ][ i ] specifies the time interval between the
    arrival of the first bit in the smoothing buffer and the subsequent removal of the data that belongs to the
    first coded frame for operating point op, measured in units of 1/90000 seconds.

    ops_encoder_buffer_delay[ obu_xlayer_id ][ opsID ][ i ] specifies, in combination with the
    ops_decoder_buffer_delay syntax element, the first bit arrival time of frames to be decoded to the
    smoothing buffer. ops_encoder_buffer_delay is measured in units of 1/90000 seconds.

    For a video sequence that includes one or more random access points the sum of
    ops_decoder_buffer_delay and ops_encoder_buffer_delay shall be kept constant.




    AV2 Specification                                                                                Page 297 of 1169
    ops_low_delay_mode_flag[ obu_xlayer_id ][ opsID ][ i ] equal to 1 indicates that the smoothing buffer
    operates in low-delay mode for operating point op. In low-delay mode late decode times and buffer
    underflow are both permitted. ops_low_delay_mode_flag equal to 0 indicates that the smoothing buffer
    operates in strict mode, where buffer underflow is not allowed.

```

<a id="s-6-10-6"></a>

#### § 6.10.6 Operating point set color info semantics

```text
§   6.10.6. Operating point set color info semantics

    ops_color_description_idc[ obu_xlayer_id ][ opsID ][ i ] indicates the combination of color primaries,
    transfer characteristics, and matrix coefficients, within the i-th operating point index with an operating
    point id given by opsID, at the obu_xlayer_id as follows:

                          Table 6.13: ops_color_description_idc values and their interpretations

      Value      Interpretation          ops_color_primaries         ops_transfer_characteristics             ops_matrix_coefficients

     0         Explicitly signaled   Explicit                   Explicit                                  Explicit

     1         BT.709 SDR            1                          1                                         1

     2         BT.2100 PQ            9                          16                                        9

     3         BT.2100 HLG           9                          18                                        9

     4         sRGB                  1                          13                                        0

     5         sYCC                  1                          13                                        5

     6-127     Reserved              -                          -                                         -


    The value of ops_color_description_idc[ obu_xlayer_id ][ opsID ][ i ] shall be in the range of 0 to 127,
    inclusive. Values larger than 5 are reserved for future use by AOMedia and shall be ignored by decoders
    conforming to this version of this specification.

    ops_color_primaries[ obu_xlayer_id ][ opsID ][ i ] specifies the color primaries at the i-th operating
    point index with an operating point id given by opsID at the obu_xlayer_id is an integer that is associated
    with the ColourPrimaries variable specified in ISO/IEC 23091-4/ITU-T H.273.

                                     Table 6.14: ops_color_primaries values and names

         ops_color_primaries              Name of color primaries                                  Description

                   1                             CP_BT_709                 [ITU-R-BT.709]

                   2                         CP_UNSPECIFIED                Unspecified

                   4                            CP_BT_470_M                BT.470 System M (historical)

                   5                            CP_BT_470_B_G              BT.470 System B, G (historical)

                   6                             CP_BT_601                 [ITU-R-BT.601]

                   7                            CP_SMPTE_240               SMPTE 240

                   8                        CP_GENERIC_FILM                Generic film (color filters using illuminant C)

                   9                             CP_BT_2020                BT.2020, BT.2100

                   10                              CP_XYZ                  SMPTE 428 (CIE 1931 XYZ)

                   11                           CP_SMPTE_431               SMPTE RP 431-2

                   12                           CP_SMPTE_432               SMPTE EG 432-1

                   22                           CP_EBU_3213                EBU Tech. 3213-E




    AV2 Specification                                                                                                   Page 298 of 1169
ops_transfer_characteristics[ obu_xlayer_id ][ opsID ][ i ] specifies the transfer characteristics at
the i-th operating point index with an operating point id given by opsID at the obu_xlayer_id is an integer
that is associated with the TransferCharacteristics variable specified in ISO/IEC 23091-4/ITU-T H.273.

    ops_transfer_characteristics         Name of transfer characteristics                         Description

                    0                            TC_RESERVED_0                   For future use

                    1                              TC_BT_709                     [ITU-R-BT.709]

                    2                           TC_UNSPECIFIED                   Unspecified

                    3                            TC_RESERVED_3                   For future use

                    4                             TC_BT_470_M                    BT.470 System M (historical)

                    5                            TC_BT_470_B_G                   BT.470 System B, G (historical)

                    6                              TC_BT_601                     [ITU-R-BT.601]

                    7                            TC_SMPTE_240                    SMPTE 240 M

                    8                              TC_LINEAR                     Linear

                    9                              TC_LOG_100                    Logarithmic (100 : 1 range)

                    10                         TC_LOG_100_SQRT10                 Logarithmic (100 * Sqrt(10) : 1 range)

                    11                            TC_IEC_61966                   IEC 61966-2-4

                    12                             TC_BT_1361                    BT.1361

                    13                               TC_SRGB                     sRGB or sYCC

                    14                         TC_BT_2020_10_BIT                 BT.2020 10-bit systems [Rec.2020]

                    15                               Reserved                    Reserved for AOMedia use

                    16                           TC_SMPTE_2084                   SMPTE ST 2084, ITU BT.2100 PQ

                    17                           TC_SMPTE_428                    SMPTE ST 428

                    18                               TC_HLG                      BT.2100 HLG, ARIB STD-B67


ops_matrix_coefficients[ obu_xlayer_id ][ opsID ][ i ] specifies the matrix coefficients at the i-th
operating point index with an operating point id given by opsID at the obu_xlayer_id is an integer that is
associated with the MatrixCoefficients variable specified in ISO/IEC 23091-4/ITU-T H.273.

                             Table 6.15: ops_matrix_coefficients values and names

   ops_matrix_coefficients         Name of matrix coefficients                             Description

                0                         MC_IDENTITY              Identity matrix

                1                          MC_BT_709               [ITU-R-BT.709]

                2                      MC_UNSPECIFIED              Unspecified

                3                       MC_RESERVED_3              For future use

                4                           MC_FCC                 US FCC 73.628

                5                        MC_BT_470_B_G             BT.470 System B, G (historical)

                6                          MC_BT_601               [ITU-R-BT.601]

                7                        MC_SMPTE_240              SMPTE 240 M

                8                      MC_SMPTE_YCGCO              YCgCo

                9                       MC_BT_2020_NCL             BT.2020 non-constant luminance, BT.2100 YCbCr

               10                       MC_BT_2020_CL              BT.2020 constant luminance [Rec.2020]



AV2 Specification                                                                                               Page 299 of 1169
                   11                   MC_SMPTE_2085            SMPTE ST 2085 YDzDx

                   12                  MC_CHROMAT_NCL            Chromaticity-derived non-constant luminance

                   13                   MC_CHROMAT_CL            Chromaticity-derived constant luminance

                   14                      MC_ICTCP              BT.2100 ICtCp

                   15                      MC_IPT_C2             IPT-C2

                   16                    MC_YCGCO_RE             YCgCo-Re

                   17                    MC_YCGCO_RO             YCgCo-Ro


    ops_full_range_flag[ obu_xlayer_id ][ opsID ][ i ] is a binary value that is associated with the
    VideoFullRangeFlag variable specified in ISO/IEC 23091-4/ITU-T H.273. ops_full_range_flag specifies the
    value of the full range flag at the i-th operating point index with an operating point id given by opsID at
    the obu_xlayer_id. ops_full_range_flag equal to 0 shall be referred to as the studio swing representation
    and ops_full_range_flag equal to 1 shall be referred to as the full swing representation for all intents
    relating to this specification.

```

<a id="s-6-10-7"></a>

#### § 6.10.7 Operating point set mlayer info semantics

```text
§   6.10.7. Operating point set mlayer info semantics

    The mlayer info syntax structure describes the embedded layer and temporal layer configuration for each
    extended layer included in an operating point.

    ops_mlayer_map[ obuXLId ][ opsID ][ opIndex ][ xLId ] specifies an 8-bit bitmask representing the
    embedded layers included for the xLId extended layer, within the operating point at index opIndex, in the
    OPS identified by opsID, at the obuXLId. Bit j being set to 1 indicates that embedded layer j is included.

    ops_tlayer_map[ obuXLId ][ opsID ][ opIndex ][ xLId ][ j ] specifies a 4-bit bitmask representing the
    temporal layers included for embedded layer j of the xLId extended layer, within the operating point at
    index opIndex, in the OPS identified by opsID, at the obuXLId. Bit k being set to 1 indicates that temporal
    layer k is included.

    It is a requirement of bitstream conformance that the indication of the dependency information for any
    operating point specified in an OPS OBU associated with this bitstream, denoted by
    ops_mlayer_map[ obuXLId ][ opsID ][ opIndex ][ xLId ] and ops_tlayer_map[ obuXLId ][ opsID ][ opIndex ]
    [ xLId ][ cMId ], if present, shall agree with the indication in the information in the activated sequence
    header, denoted by MlayerDependencyMap[ cMId ][ rMId ] and TlayerDependencyMap[ cMId ][ cTId ]
    [ cTId ] so that:

      • For any embedded layer with ID equal to cMId, if MLayerDependencyMap[ cMId ][ rMId ] is equal to
        1 and ops_mlayer_map[ obuXLId ][ opsID ][ opIndex ][ xLId ] & (1 << cMId) is greater than 0,
        ops_mlayer_map[ obuXLId ][ opsID ][ opIndex ][ xLId ] & (1 << rMId) shall not be equal to 0 for all non-
        negative rMId less than cMId.
      • For any embedded layer with ID equal to cMId and temporal layer with ID equal to cTId, if
        TLayerDependencyMap[ cMId ][ cTId ][ rTId ] is equal to 1 and ops_tlayer_map[ obuXLId ][ opsID ]
        [ opIndex ][ xLId ][ cMId ] & (1 << cTId) is greater than 0, ops_tlayer_map[ obuXLId ][ opsID ][ opIndex ]
        [ xLId ][ cMId ] & (1 << rTId) shall not be equal to 0 for all non-negative rTId less than cTId.




    AV2 Specification                                                                                      Page 300 of 1169
      NOTE: Above bitstream constraints on ops_mlayer_map (and similarly for ops_tlayer_map based on
      TLayerDependencyMap) make sure that, if MLayerDependencyMap[ cMId ][ rMId ] is equal to 1, any
      embedded layer with ID rMId referenced from the existing embedded layer with ID cMId are
      indicated to be present in any operating point specified in an OPS OBU. Otherwise, if
      MLayerDependencyMap[ cMId ][ rMId ] is equal to 0, indicating that an embedded layer with ID
      cMId does not depend on an embedded layer with ID rMId, lcr_mlayer_map[ isGlobal ][ xId ] is
      allowed to indicate that the embedded layer with ID rMId may or may not be present in the operating
      point.


```

<a id="s-6-11"></a>

### § 6.11 Buffer removal timing OBU semantics

```text
§   6.11. Buffer removal timing OBU semantics
    br_ops_dependent_flag equal to 1 specifies that the timing information associated with a specific
    operating point set is present in the buffer_removal_timing_obu( ). br_ops_dependent_flag equal to 0
    specifies that timing information associated with an operating point set is not present in the
    buffer_removal_timing_obu( ).

    br_ops_id specifies the operating point set id.

    It is a requirement of bitstream conformance that br_ops_id is equal to an operating point set
    ops_id[ obu_xlayer_id ] that is present in the bitstream.

    br_ops_cnt[ br_ops_id ] specifies the operating point count.

    It is a requirement of bitstream conformance that br_ops_cnt[ br_ops_id ] is equal to
    ops_cnt[ obu_xlayer_id ][ br_ops_id ].


      NOTE: The conformance requirements on br_ops_id and br_ops_cnt[ br_ops_id ] ensure that the
      operating point index i in the buffer_removal_timing_obu( ) loop has a one-to-one correspondence
      with the operating point index i in the operating_point_set_obu( ) loop for the same operating point
      set. That is, the i-th operating point in the BRT OBU corresponds to the i-th operating point in the
      OPS OBU.


    br_decoder_model_present_op_flag[ br_ops_id ][ i ] equal to 1 specifies that br_buffer_removal_time
    is present for operating point i. br_decoder_model_present_op_flag[ br_ops_id ][ i ] equal to 0 specifies
    that br_buffer_removal_time is not present.

    br_time_op[ br_ops_id ][ i ] specifies the frame removal time in units of DecCT clock ticks counted from
    the removal time of the last random access point for operating point i of the specified operating point set
    br_ops_id when the current frame is not associated with a random access point and from the previous
    random access point when the current frame is associated with a random access point.

    br_time specifies the frame removal time in units of DecCT clock ticks counted from the removal time of
    the last random access point when the current frame is not associated with a random access point and
    from the previous random access point when the current frame is associated with a random access point.

```

<a id="s-6-12"></a>

### § 6.12 Quantizer Matrix OBU semantics

```text
§   6.12. Quantizer Matrix OBU semantics
    qm_bit_map is a bitmask that specifies which quantizer matrices are present in the OBU.




    AV2 Specification                                                                            Page 301 of 1169
    When there are multiple quantizer matrices OBUs between coded frames, it is a requirement of bitstream
    conformance that only the first quantizer matrix can have qm_bit_map equal to 0.

    When there are multiple quantizer matrices OBUs between coded frames, it is a requirement of bitstream
    conformance that the same level of quantizer matrix is not specified twice in those OBUs.

    qm_chroma_info_present_flag equal to 1 specifies that the chroma quantizer matrices are present in
    this OBU. qm_chroma_info_present_flag equal to 0 specifies that chroma quantizer matrices are not
    present and default chroma quantizer matrices shall be used.

    qm_is_default_flag equal to 1 specifies that the default quantizer matrix is used for the current
    quantizer level and QmDataPresent for this level is set to 0. qm_is_default_flag equal to 0 specifies that
    user-defined quantizer matrix data is present via the user_defined_qm() syntax structure.

    QmDataPresent is an array specifying which quantizer matrix levels have data that can be used.

    QmSeen is an array specifying which quantizer matrix levels have been seen since the last frame.

    QmProtected is an array specifying which quantizer matrix levels are protected. Unprotected levels will
    be reset at the first OBU with obu_type equal to OBU_CLOSED_LOOP_KEY or OBU_OPEN_LOOP_KEY in
    a temporal layer.

    Initialize every entry of QmProtected, QmSeen, and QmDataPresent to zero at the start of a bitstream.

```

<a id="s-6-13"></a>

### § 6.13 Film grain OBU semantics

```text
§   6.13. Film grain OBU semantics
    fgm_update_flags specifies a bitmap of which film grain models are present in the OBU. If bit i of
    fgm_update_flags is equal to 1 (i.e., if fgm_update_flags & (1 << i) is non-zero), then a film grain model is
    present for slot i.

    When there are multiple film grain OBUs present in the same coded frame unit, it is a requirement of
    bitstream conformance that bit i of fgm_update_flags is equal to 1 in at most one film grain OBU.


      NOTE: The same film grain slot can be reused or updated by a film grain OBU in a subsequent
      coded frame unit.


    It is a requirement of bitstream conformance that fgm_update_flags is not equal to 0.

    fgm_chroma_idc is used to derive the subsampling format used by the film grain.

    It is a requirement of bitstream conformance that fgm_chroma_idc is less than or equal to 3.

    save_grain_model( i ) is a function call that indicates that all the syntax elements read in
    film_grain_model should be saved into an area of memory indexed by i.

    FilmGrainPresent is an array that records which film grain OBUs have been received. Initialize every
    entry of FilmGrainPresent to zero at the start of a bitstream.


      NOTE: FilmGrainPresent is only used to specify a conformance constraint and does not affect the
      decoding process.




    AV2 Specification                                                                                Page 302 of 1169
```

<a id="s-6-14"></a>

### § 6.14 Content interpretation OBU semantics

```text
§   6.14. Content interpretation OBU semantics
    A content interpretation OBU can be present in any embedded layer. However, when present, all
    instances of a content interpretation OBU in an embedded layer within a coded video sequence shall
    contain the same information. No such constraint exists for content interpretation OBUs in different
    embedded layers except parameters in the time_info() structure which shall be the same across all
    embedded layers within a coded video sequence.

    If no content interpretation OBU is present for embedded layer m, the content interpretation parameters
    are inherited from embedded layer k, where k is the highest embedded layer less than m for which
    MLayerPresenceMap[m][k] is equal to 1 and content interpretation parameters have been established.

    The content interpretation parameters for each embedded layer are initialized and updated as specified
    in § 7.3.8.11 Content interpretation parameters initialization. When a content interpretation OBU is
    present in a temporal unit that does not contain a CLK or OLK for the same embedded layer, and does not
    contain a CLK or OLK for any embedded layer k where MLayerPresenceMap[m][k] is equal to 1, the
    contents shall be identical to the content interpretation parameters established at the most recent
    random access point.

    ci_scan_type_idc indicates how to interpret the pictures within a CVS in terms of progressive or
    interlace samples, as follows:

             ci_scan_type_idc     Interpretation of ci_scan_type_idc

                        0         Unspecified

                        1         Progressive frame picture samples

                        2         Interlace field picture samples

                        3         Interlace complementary field-pair picture samples


    ci_color_description_present_flag equal to 1 specifies that the syntax element ci_color_description_idc
    and associated color description syntax elements are present to indicate color space information.
    ci_color_description_present_flag equal to 0 specifies that ci_color_description_idc and associated syntax
    elements are not present.

    ci_chroma_sample_position_present_flag equal to 1 specifies that syntax elements describing the
    chroma sample positions are present. ci_chroma_sample_position_present_flag equal to 0 specifies that
    chroma sample position syntax elements are not present.

    ci_aspect_ratio_info_present_flag equal to 1 specifies that the aspect ratio syntax elements are present
    to indicate the aspect ratio of the decoded frames. ci_aspect_ratio_info_present_flag equal to 0 specifies
    that aspect ratio syntax elements are not present.

    ci_timing_info_present_flag equal to 1 specifies that timing information is present to indicate frame
    timing parameters. ci_timing_info_present_flag equal to 0 specifies that timing information is not present.

    ci_reserved_2bit must be set to 0. The value shall be ignored by a decoder.

    ci_color_description_idc, ci_color_primaries, ci_matrix_coefficients, ci_transfer_characteristics,
    ci_full_range_flag specify the color information for this layer with the same interpretation as
    ops_color_description_idc, ops_color_primaries, ops_matrix_coefficients, ops_transfer_characteristics and
    ops_full_range_flag.



    AV2 Specification                                                                            Page 303 of 1169
ci_chroma_sample_position_top indicates the chroma sampling grid alignment for top video field or for
a frame using the 4:2:0 (in which the two chroma arrays have half the width and half the height of the
associated luma array) or 4:2:2 (in which the two chroma arrays have half the width of the associated
luma array) color formats. For 4:2:0 formats, these interpretations match those of the
Chroma420SampleLocType variable specified in ISO/IEC 23091-4/ITU-T H.273.

The chroma sample positions allowed are:

   ci_chroma_sample_position_(top/    Name of chroma          Meaning for 4:2:2           Meaning for 4:2:0
              bottom)                 sample position      (offsets from (0,0) luma    (offsets from (0,0) luma
                                                                    sample)                     sample)

                    0                    CSP_LEFT         Horizontal offset 0         Horizontal offset 0, vertical
                                                                                      offset 0.5

                    1                   CSP_CENTER        Horizontal offset 0.5       Horizontal offset 0.5,
                                                                                      vertical offset 0.5

                    2                   CSP_TOPLEFT       N/A                         Horizontal offset 0, vertical
                                                                                      offset 0

                    3                     CSP_TOP         N/A                         Horizontal offset 0.5,
                                                                                      vertical offset 0

                    4                 CSP_BOTTOMLEFT      N/A                         Horizontal offset 0, vertical
                                                                                      offset 1

                    5                   CSP_BOTTOM        N/A                         Horizontal offset 0.5,
                                                                                      vertical offset 1

                    6                 CSP_UNSPECIFIED     Unknown or determined by    Unknown or determined by
                                                          the application             the application


If ci_chroma_sample_position_top is present in the bitstream, it is a requirement of bitstream
conformance that the value is less than or equal to 5.

ci_chroma_sample_position_bottom indicates the chroma sampling grid alignment for bottom video
field using the 4:2:0 (in which the two chroma arrays have half the width and half the height of the
associated luma array) or 4:2:2 (in which the two chroma arrays have half the width of the associated
luma array) color formats. For 4:2:0 formats, these interpretations match those of the
Chroma420SampleLocType variable specified in ISO/IEC 23091-4/ITU-T H.273.

If ci_chroma_sample_position_bottom is present in the bitstream, it is a requirement of bitstream
conformance that the value is less than or equal to 5.

ci_aspect_ratio_idc indicates the value of the sample aspect ratio of the coded luma samples. The
sample aspect ratio is a quantity that describes how the width of a sample compares to its height.

When ci_aspect_ratio_idc is equal to 255, then the sample aspect ratio is explicitly indicated using the
syntax elements ci_sar_width and ci_sar_height.

If ci_aspect_ratio_idc is not equal to 255, it is a requirement of bitstream conformance that
ci_aspect_ratio_idc is less than or equal to 16.

ci_sar_width and ci_sar_height indicate the horizontal and vertical size of the sample aspect ratio (in
the same arbitrary units).

When ci_sar_width is equal to 0 or ci_sar_height is equal to 0, the sample aspect ratio is unspecified in
this specification but may be provided through external means.


AV2 Specification                                                                                  Page 304 of 1169
```

<a id="s-6-15"></a>

### § 6.15 Padding OBU semantics

```text
§   6.15. Padding OBU semantics
    Multiple padding units can be present, each padding with an arbitrary number of bytes. Padding OBUs
    have no effect on the decoding process.

    obu_padding_byte is a padding byte. Padding bytes may have arbitrary values and have no effect on the
    decoding process.

```

<a id="s-6-16"></a>

### § 6.16 Metadata OBU semantics

```text
§   6.16. Metadata OBU semantics
```

<a id="s-6-16-1"></a>

#### § 6.16.1 Metadata unit semantics

```text
§   6.16.1. Metadata unit semantics

    Metadata units can be contained in either a metadata OBU or a metadata group OBU.

    metadata_unit_remaining_bit can take any value but is reserved for future use by AOMedia.

    Decoders conforming to this version of this specification shall ignore the value of
    metadata_unit_remaining_bit.


      NOTE: Encoders are recommended to set metadata_unit_remaining_bit to zero and to ensure that
      remainingMuPayloadBits is less than 8 (i.e., encoders should only extend to reach byte alignment).


    It is a requirement of bitstream conformance that any computed values for remainingMuPayloadBits shall
    not be less than 0.

```

<a id="s-6-16-2"></a>

#### § 6.16.2 Metadata short OBU semantics

```text
§   6.16.2. Metadata short OBU semantics

    metadata_is_suffix has the same semantics as in metadata group OBU semantics § 6.16.3 Metadata
    group OBU semantics.

    metadata_necessity_idc has the same semantics as in metadata group OBU semantics § 6.16.3
    Metadata group OBU semantics.

    metadata_application_id has the same semantics as in metadata group OBU semantics § 6.16.3
    Metadata group OBU semantics.

    muh_layer_idc has the same semantics as in metadata group OBU semantics § 6.16.3 Metadata group
    OBU semantics.

    It is a requirement of bitstream conformance that muh_layer_idc is less than 3.

    muh_cancel_flag has the same semantics as in metadata group OBU semantics § 6.16.3 Metadata group
    OBU semantics.

    muh_persistence_idc has the same semantics as in metadata group OBU semantics § 6.16.3 Metadata
    group OBU semantics.

    metadata_type has the same semantics as in metadata group OBU semantics § 6.16.3 Metadata group
    OBU semantics.


      NOTE:       muh_priority is not specified when this short form is used.




    AV2 Specification                                                                         Page 305 of 1169
      NOTE: For an OBU with obu_type equal to OBU_METADATA_SHORT and with metadata_type equal
      to METADATA_TYPE_ICC_PROFILE, METADATA_TYPE_ITUT_T35, or
      METADATA_TYPE_USER_DATA_UNREGISTERED, the value of the metadataPayloadSize ensures that
      the trailing_bits syntax contains exactly 8 bits. If an encoder wants to pad with additional bytes for
      these metadata types, it can add such bytes before the trailing_bits syntax. The added bytes do not
      need to be zero.

```

<a id="s-6-16-3"></a>

#### § 6.16.3 Metadata group OBU semantics

```text
§   6.16.3. Metadata group OBU semantics

    metadata_is_suffix, when equal to 0 (prefix), indicates that the metadata appears before the frame data
    within coded frame units. Otherwise, metadata_is_suffix equal to 1 (suffix) indicates that the metadata
    appears after the frame data within coded frame units.


      NOTE: Prefix metadata is suitable for signaling information that is known prior to encoding such as
      presentation time. Suffix metadata is suitable for information that is known after encoding such as a
      frame hash.


    metadata_necessity_idc indicates the essentiality of the metadata OBU and the contained metadata
    units as follows:

     metadata_necessity_idc      Name                                            Description

     0                         UNDEFINED     The necessity of the current metadata OBU is undefined.

     1                         NECESSARY     All metadata units within the metadata OBU are considered necessary for the
                                             receiving system.

     2                         ADVISORY      All metadata units within the metadata OBU are advisory for the receiving system.

     3                         MIXED         At least one metadata unit is considered necessary, and others may be advisory. The
                                             determination is made based on the semantics of each metadata type.


    metadata_application_id indicates the application id associated with the current metadata OBU as
    specified in Table 6.16:

                          Table 6.16: metadata_application_id values and descriptions

     metadata_application_id            Name                                            Description

     0                         UNSPECIFIED               Application is undetermined.

     1                         MOBILE_OR_TV              Metadata is intended for a mobile device (e.g., smartphone) or a TV.

     2                         MOBILE                    Metadata is intended for a mobile device (e.g., smartphone).

     3                         TV                        Metadata is intended for a TV.

     4                         HMD                       Metadata is intended for a Head Mounted Display.

     5                         WEARABLE                  Metadata is intended for a wearable device (e.g., watch).

     6-15                      Reserved for AOMedia      Reserved for AOMedia use.
                               use

     16-31                     Externally defined        Application can be determined through external signaling (e.g., within
                                                         an mp4 file).




    AV2 Specification                                                                                                Page 306 of 1169
metadata_unit_cnt_minus_1 plus 1, specifies the total number of metadata units present in the current
metadata_group_obu(). It is a requirement of bitstream conformance that the value of
metadata_unit_cnt_minus_1 is less than 16383.

metadata_type indicates the type of metadata as specified in Table 6.17:

                          Table 6.17: metadata_type values and layer-specific status

      metadata_type                             Name of metadata_type                                      Layer-specific

 0                        Reserved for AOMedia use                                                    -

 1                        METADATA_TYPE_HDR_CLL                                                       N

 2                        METADATA_TYPE_HDR_MDCV                                                      N

 3                        METADATA_TYPE_ITUT_T35                                                      payload-specific

 4                        METADATA_TYPE_TIMECODE                                                      Y

 5                        METADATA_TYPE_DECODED_FRAME_HASH                                            Y

 6                        METADATA_TYPE_BANDING_HINTS                                                 Y

 7                        METADATA_TYPE_ICC_PROFILE                                                   N

 8                        METADATA_TYPE_SCAN_TYPE                                                     N

 9                        METADATA_TYPE_TEMPORAL_POINT_INFO                                           Y

 10                       METADATA_TYPE_USER_DATA_UNREGISTERED                                        payload-specific

 11 and greater           Reserved for AOMedia use                                                    -


The semantics of the column “Layer-specific” and its values are defined in § 6.2.2 OBU header semantics.

muh_header_size specifies the number of bytes in the metadata unit header.


  NOTE: muh_header_size includes muh_header_extension_byte syntax elements but excludes
  muh_cancel_flag.


muh_cancel_flag when set to 1, indicates that any previously signaled metadata information for a
metadata with type equal to muh_metadata_type is cancelled for either the current extended layer if
obu_xlayer_id is less than GLOBAL_XLAYER_ID, or for a set of extended layers if obu_xlayer_id is equal to
GLOBAL_XLAYER_ID.

muh_layer_idc is used to signal a mode that specifies the layers to which the signaled metadata applies.
This value can represent different modes, such as applying the metadata to all layers, applying the
metadata to a continuous range of layer values, or applying the metadata to a set of specific layer values.
The specific values for the layer_idc are defined as follows:

 muh_layer_idc            Name                                               Description

 0                  LAYER_UNSPECIFIED   The current signaling does not specify to what layers the metadata applies to. This
                                        information can potentially be indicated or determined through external means.

 1                  LAYER_GLOBAL        The metadata applies to all layers if obu_xlayer_id is equal to GLOBAL_XLAYER_ID. If
                                        obu_xlayer_id is less than GLOBAL_XLAYER_ID, layers with matching obu_xlayer_id
                                        only.

 2                  LAYER_CURRENT       The metadata applies to the current layer only as indicated by the specific values for
                                        obu_xlayer_id and obu_mlayer_id in OBU header.




AV2 Specification                                                                                              Page 307 of 1169
 3                  LAYER_VALUES            The metadata applies to a set of specific layer values, which are explicitly signaled.

 4-7                Reserved                Reserved for AOMedia use.


muh_payload_size signals the size of the metadata payload in bytes.


  NOTE:       This includes the byte alignment bits if those are needed.


muh_persistence_idc is used to signal the mode in which the signaled metadata persists over time. This
value can represent different modes, such as global persistence for the entire video sequence,
persistence for a group of frames of a certain duration, or persistence for a single frame only.

The specific values for the muh_persistence_idc are defined as follows:

 muh_persistence_idc                 Name                                               Description

 0                        GLOBAL_PERSISTENCE             Global persistence for the entire video sequence. When this mode is
                                                         signaled previously signaled global metadata of this type are
                                                         overwritten. The cancel flag (muh_cancel_flag) does not do anything to
                                                         it.

 1                        BASIC_PERSISTENCE              Persistence until a new metadata unit of the same type is encountered
                                                         that applies to the layer or the cancel flag (muh_cancel_flag) is
                                                         encountered.

 2                        NO_PERSISTENCE                 Used only for the current frame.

 3                        ENHANCED_PERSISTENCE           This one is similar to basic but can allow updates of metadata without
                                                         full replacement.

 4-7                      Reserved                       Reserved for AOMedia use.


muh_priority is used to indicate the relative importance or urgency of a particular type of metadata. A
lower value indicates a higher priority, while a higher value indicates a lower priority.


  NOTE: This information can be used by decoders to prioritize the processing of different types of
  metadata, ensuring that critical or time-sensitive metadata is handled before less important
  metadata. Furthermore, it can also be beneficial on a system level. For example, in lossy channels,
  more important information can be protected or re-transmitted more frequently, ensuring that critical
  or time-sensitive metadata is less likely to be lost or corrupted during transmission.


muh_reserved_zero_2bits must be set to zero and shall be ignored by decoders.

muh_xlayer_map contains a bitmask. The metadata unit is intended for an extended layer x if bit x of
muh_xlayer_map is equal to 1.

It is a requirement of bitstream conformance that bit 31 of muh_xlayer_map is equal to 0.

muh_mlayer_map contains a bitmask. The metadata unit is intended for an embedded layer m if bit m of
muh_mlayer_map is equal to 1.

It is a requirement of bitstream conformance that bit m of muh_mlayer_map is equal to 0 for m less than
obu_mlayer_id.




AV2 Specification                                                                                                    Page 308 of 1169
      NOTE: It is possible that the layers indicated may have been removed because of a selection of an
      operating point. A decoder will only apply the metadata to the remaining layers according to the
      selected operating point.


    When metadata is indicated as persistent and is specified at embedded layer K and temporal layer T, the
    metadata applies to other layers according to the following rules:

      • Temporal persistence: Within embedded layer K, the metadata persists to temporal layer C if
        TLayerDependencyMap[K][C][T] is equal to 1. If TLayerDependencyMap[K][C][T] is equal to 0, the metadata does
        not apply to temporal layer C.
      • Multi-layer persistence: The metadata persists from embedded layer K to embedded layer M (where
        M > K) if the metadata has explicit layer persistence indication and MLayerDependencyMap[M][K] is equal to
        1.
      • Combined persistence: When metadata persists from embedded layer K to embedded layer M, it
        applies to temporal layer C within embedded layer M if TLayerDependencyMap[M][C][T] is equal to 1.


      NOTE: Metadata has explicit layer persistence indication when muh_layer_idc is equal to LAYER_VALUES
      (3) and muh_mlayer_map has bits set for embedded layers greater than obu_mlayer_id.


    Decoders shall ignore metadata that does not apply to the current operating point based on these rules.

    muh_header_extension_byte, if present, contains additional bytes. Decoders conforming to this version
    of this specification should ignore the contents.

```

<a id="s-6-16-4"></a>

#### § 6.16.4 Metadata ITUT T35 semantics

```text
§   6.16.4. Metadata ITUT T35 semantics

    itu_t_t35_country_code shall be a byte having a value specified as a country code by Annex A of
    Recommendation ITU-T T.35.

    itu_t_t35_country_code_extension_byte shall be a byte having a value specified as a country code by
    Annex B of Recommendation ITU-T T.35.

    itu_t_t35_payload_bytes shall be bytes containing data registered as specified in Recommendation ITU-
    T T.35.

    The ITU-T T.35 terminal provider code and terminal provider oriented code shall be contained in the first
    one or more bytes of the itu_t_t35_payload_bytes, in the format specified by the Administration that
    issued the terminal provider code. Any remaining bytes in itu_t_t35_payload_bytes data shall be data
    having syntax and semantics as specified by the entity identified by the ITU-T T.35 country code and
    terminal provider code.

```

<a id="s-6-16-5"></a>

#### § 6.16.5 Metadata high dynamic range content light level semantics

```text
§   6.16.5. Metadata high dynamic range content light level semantics

    This metadata unit identifies upper bounds of the nominal target brightness light level of the associated
    content.

    The values in this metadata unit are defined in relation to samples in a 4:4:4 representation of red, green,
    and blue color primary intensities in the linear light domain, in units of candelas per square meter. This
    metadata unit does not itself identify a conversion process from decoded sample values to that
    representation.


    AV2 Specification                                                                                Page 309 of 1169
      NOTE: Other syntax elements such as BitDepth, color_primaries, transfer_characteristics, and
      matrix_coefficients, when present, can assist in identifying such a conversion process.


    Given the red, green, and blue linear-light intensities at a sample location, denoted ER, EG, and EB, the
    maximum component intensity is computed as EMax = Max( ER, Max( EG, EB ) ). The light level at that
    location is the CIE 1931 luminance corresponding to equal amplitudes of EMax for all three primaries,
    scaled so that peak white corresponds to the nominal maximum luminance (e.g., 10 000 cd/m² when
    transfer_characteristics corresponds to PQ).


      NOTE: Because EMax rather than a direct RGB-to-luminance conversion is used, the CIE 1931
      luminance can be less than the indicated light level - for example when EB is large and ER, EG are
      near zero.


    The calculation method for max_cll and max_fall is defined in [CTA-861], Annex P (Calculation of MaxCLL
    and MaxFALL).

    metadata_hdr_cll metadata associated with an embedded layer, when present, shall be indicated at the first
    coded picture of that embedded layer in the coded video sequence. Any additional metadata_hdr_cll
    metadata units associated with an embedded layer in a coded video sequence shall have the same
    content. When an embedded layer inherits color information from another layer, the inherited layer’s
    metadata_hdr_cll applies unless overridden by a metadata_hdr_cll metadata unit present for the inheriting
    layer.


      NOTE: These values are determined from the source content prior to encoding. The light levels of
      the reconstructed decoded pictures may differ due to quantization and any color space or transfer
      characteristic conversions applied during the encoding process.


    max_cll, when not equal to 0, specifies an upper bound on the maximum light level among all individual
    samples, in a 4:4:4 representation of red, green, and blue color primary intensities in the linear light
    domain, across all pictures of the embedded layers of the coded video sequence, in units of cd/m²
    associated with this metadata unit. When equal to 0, no such upper bound is signaled.

    max_fall, when not equal to 0, specifies an upper bound on the maximum frame-average light level
    across all pictures, in a 4:4:4 representation of red, green, and blue color primary intensities in the linear
    light domain, of the embedded layers of the coded video sequence, in units of cd/m² associated with this
    metadata unit. When equal to 0, no such upper bound is signaled.


      NOTE: When the visually relevant region does not cover the entire decoded picture (e.g., letterbox
      content), the frame-average is expected to be computed only over the visually relevant region.

```

<a id="s-6-16-6"></a>

#### § 6.16.6 Metadata high dynamic range mastering display color volume semantics

```text
§   6.16.6. Metadata high dynamic range mastering display color volume semantics

    This metadata unit describes the color volume of the mastering display — the color primaries, white
    point, and luminance range of the display used when grading the associated video content.




    AV2 Specification                                                                               Page 310 of 1169
  NOTE: The semantics of this metadata unit differ from the equivalent metadata in AV1. AV2 uses
  integer units consistent with SMPTE ST 2086, making the binary encoding identical to other
  specifications and enabling mastering display metadata to be passed across container boundaries
  without conversion.


metadata_hdr_mdcv metadata associated with an embedded layer, when present, shall be indicated at the
first coded picture of that embedded layer in the coded video sequence. Any additional metadata_hdr_mdcv
metadata units associated with an embedded layer in a coded video sequence shall have the same
content. When an embedded layer inherits color information from another layer, the inherited layer’s
metadata_hdr_mdcv applies unless overridden by a metadata_hdr_mdcv metadata unit present for the inheriting
layer.

primary_chromaticity_x[ i ] specifies the normalized x chromaticity coordinate of color primary i of the
mastering display, as defined by CIE 1931, in integer units of 0.00002. Valid values are in the range 5 to
37000, inclusive. Values outside this range indicate that the coordinate is unknown or unspecified.

primary_chromaticity_y[ i ] specifies the normalized y chromaticity coordinate of color primary i of the
mastering display, as defined by CIE 1931, in integer units of 0.00002. Valid values are in the range 5 to
42000, inclusive. Values outside this range indicate that the coordinate is unknown or unspecified.

For mastering displays with red, green, and blue primaries, it is suggested that i = 0 corresponds to the
green primary, i = 1 to the blue primary, and i = 2 to the red primary.


  NOTE: SMPTE ST 2086 expresses chromaticity coordinates to four decimal places, which
  corresponds to multiples of 5 in this encoding. ANSI/CTA-861-G signals an unknown white point
  chromaticity using (x, y) = (0, 0).


white_point_chromaticity_x specifies the normalized x chromaticity coordinate of the mastering display
white point, as defined by CIE 1931, in integer units of 0.00002. Valid values are in the range 5 to 37000,
inclusive. Values outside this range indicate that the coordinate is unknown or unspecified.

white_point_chromaticity_y specifies the normalized y chromaticity coordinate of the mastering display
white point, as defined by CIE 1931, in integer units of 0.00002. Valid values are in the range 5 to 42000,
inclusive. Values outside this range indicate that the coordinate is unknown or unspecified.

luminance_max specifies the nominal maximum display luminance of the mastering display in units of
0.0001 cd/m². Valid values are in the range 50000 to 100000000, inclusive. Values outside this range
indicate that the maximum luminance is unknown or unspecified.


  NOTE: SMPTE ST 2086 expresses maximum luminance in whole cd/m², which corresponds to
  multiples of 10000 in this encoding. ANSI/CTA-861-G uses the value 0 to signal that the maximum
  display luminance is unknown.


luminance_min specifies the nominal minimum display luminance of the mastering display in units of
0.0001 cd/m². Valid values are in the range 1 to 50000, inclusive. Values outside this range indicate that
the minimum luminance is unknown or unspecified. It is a requirement of bitstream conformance that
when luminance_max is equal to 50000, luminance_min shall not be equal to 50000.




AV2 Specification                                                                             Page 311 of 1169
      NOTE: SMPTE ST 2086 expresses minimum luminance in units of 0.0001 cd/m², consistent with
      this encoding. ANSI/CTA-861-G uses the value 0 to signal that the minimum display luminance is
      unknown.


    At the minimum luminance level, the mastering display white point chromaticity applies.

```

<a id="s-6-16-7"></a>

#### § 6.16.7 Metadata timecode semantics

```text
§   6.16.7. Metadata timecode semantics

    counting_type specifies the method of dropping values of the n_frames syntax element as specified in
    the table below. counting_type should be the same for all pictures in the coded video sequence.

     counting_type                                                        Meaning

     0                  no dropping of n_frames count values and no use of time_offset_value

     1                  no dropping of n_frames count values

     2                  dropping of individual zero values of n_frames count

     3                  dropping of individual values of n_frames count equal to maxFps − 1

     4                  dropping of the two lowest (value 0 and 1) n_frames counts when seconds_value is equal to 0 and
                        minutes_value is not an integer multiple of 10

     5                  dropping of unspecified individual n_frames count values

     6                  dropping of unspecified numbers of unspecified n_frames count values

     7..31              reserved


    full_timestamp_flag equal to 1 indicates that the seconds_value, minutes_value, hours_value syntax
    elements will be present. full_timestamp_flag equal to 0 indicates that there are flags to control the
    presence of these syntax elements.

    When ci_timing_info_present_flag is equal to 1, the contents of the clock timestamp indicate a time of
    origin, capture, or ideal display. This indicated time is computed as follows:

     if ( equal_picture_interval ) {
       TicksPerPicture = ( num_ticks_per_picture_minus_1 + 1 ) * num_units_in_display_tick
     } else {
       TicksPerPicture = num_units_in_display_tick
     }
     ss = ( ( hours_value * 60 + minutes_value) * 60 + seconds_value )
     clockTimestamp = ss * time_scale +
                      n_frames * TicksPerPicture + time_offset_value


    clockTimestamp is in units of clock ticks of a clock with clock frequency equal to time_scale Hz, relative
    to some unspecified point in time for which clockTimestamp would be equal to 0.

    discontinuity_flag equal to 0 indicates that the difference between the current value of clockTimestamp
    and the value of clockTimestamp computed from the previous set of timestamp syntax elements in output
    order can be interpreted as the time difference between the times of origin or capture of the associated
    frames or fields. discontinuity_flag equal to 1 indicates that the difference between the current value of
    clockTimestamp and the value of clockTimestamp computed from the previous set of clock timestamp
    syntax elements in output order should not be interpreted as the time difference between the times of
    origin or capture of the associated frames or fields.




    AV2 Specification                                                                                                Page 312 of 1169
    When ci_timing_info_present_flag is equal to 1 and discontinuity_flag is equal to 0, the value of
    clockTimestamp shall be greater than or equal to the value of clockTimestamp for the previous set of
    clock timestamp syntax elements in output order.

    cnt_dropped_flag specifies the skipping of one or more values of n_frames using the counting method
    specified by counting_type.

    n_frames is used to compute clockTimestamp. When ci_timing_info_present_flag is equal to 1, n_frames
    shall be less than maxPicPerSecond, where maxPicPerSecond is specified by maxPicPerSecond =
    ceil( time_scale / TicksPerPicture ).

    seconds_flag equal to 1 specifies that seconds_value and minutes_flag are present when
    full_timestamp_flag is equal to 0. seconds_flag equal to 0 specifies that seconds_value and minutes_flag
    are not present.

    seconds_value is used to compute clockTimestamp and shall be in the range of 0 to 59. When
    seconds_value is not present, its value is inferred to be equal to the value of seconds_value for the
    previous set of clock timestamp syntax elements in decoding order, and it is required that such a previous
    seconds_value shall have been present.

    minutes_flag equal to 1 specifies that minutes_value and hours_flag are present when
    full_timestamp_flag is equal to 0 and seconds_flag is equal to 1. minutes_flag equal to 0 specifies that
    minutes_value and hours_flag are not present.

    minutes_value specifies the value of mm used to compute clockTimestamp and shall be in the range of 0
    to 59, inclusive. When minutes_value is not present, its value is inferred to be equal to the value of
    minutes_value for the previous set of clock timestamp syntax elements in decoding order, and it is
    required that such a previous minutes_value shall have been present.

    hours_flag equal to 1 specifies that hours_value is present when full_timestamp_flag is equal to 0 and
    seconds_flag is equal to 1 and minutes_flag is equal to 1.

    hours_value is used to compute clockTimestamp and shall be in the range of 0 to 23, inclusive. When
    hours_value is not present, its value is inferred to be equal to the value of hours_value for the previous
    set of clock timestamp syntax elements in decoding order, and it is required that such a previous
    hours_value shall have been present.

    time_offset_length greater than 0 specifies the length in bits of the time_offset_value syntax element.
    time_offset_length equal to 0 specifies that the time_offset_value syntax element is not present.
    time_offset_length should be the same for all frames in the coded video sequence.

    time_offset_value is used to compute clockTimestamp. The number of bits used to represent
    time_offset_value is equal to time_offset_length. When time_offset_value is not present, its value is
    inferred to be equal to 0.

```

<a id="s-6-16-8"></a>

#### § 6.16.8 Metadata banding hints semantics

```text
§   6.16.8. Metadata banding hints semantics

    When present, the banding metadata applies to a frame or multiple frames. It indicates hints about the
    presence of banding and its characteristics. A decoder may optionally choose to utilize this information
    and no normative debanding processing associated with this metadata is required for decoder
    conformance.




    AV2 Specification                                                                              Page 313 of 1169
coding_banding_present_flag equal to 1 indicates banding due to compression is present in the current
frame. coding_banding_present_flag equal to 0 indicates banding due to compression is not present in the
current frame.

source_banding_present_flag equal to 1 indicates that source content that may be identified as banding
by a debanding algorithm is present in the current frame. source_banding_present_flag equal to 0
indicates that no specific source content that may be identified as banding has been detected in the
current frame.


  NOTE: This parameter indicates that banding-like patterns are present in the source that might be
  detected as banding on the decoded output. The hint aims to reduce false positives and aid in better
  preserving source information. However, source_banding_present_flag equal to 0 does not guarantee
  the absence of content that an algorithm may mistakenly identify as banding.


banding_hints_flag equal to 1 indicates that additional information hints about the banding
characteristic are present in this metadata message. banding_hints_flag equal to 0 indicates that
additional information hints about the banding characteristic are not present in this metadata message.

three_color_components_flag equal to 1 indicates that the banding related additional information is
signaled for three color components. three_color_components_flag equal to 0 indicates that the banding
related additional information is signaled only for the color component 0.

banding_in_component_present_flag equal to 1 indicates banding in the color component plane is
present. banding_in_component_present_flag equal to 0 indicates banding in the color component plane
is not present.

max_band_width_minus_4 plus 4 specifies the typical maximum banding width in color component
plane in the current frame in samples of component plane.

max_band_step_minus_1 plus 1 specifies the typical maximum difference between two consecutive
bands in color component plane in the current frame.

band_units_information_present_flag equal to 1 indicates that additional information hints per band
unit are present. band_units_information_present_flag equal to 0 indicates that no additional information
on banding presence for band units is present.

num_band_units_rows_minus_1 plus 1 specifies the number of band units rows.

num_band_units_cols_minus_1 plus 1 specifies the number of band units columns.

varying_size_band_units_flag equal to 1 indicates that band units of varying size are used with unit
sizes specified by syntax elements vert_size_in_band_blocks_minus_1[ r ] and
horz_size_in_band_blocks_minus_1[ c ]. varying_size_band_units_flag equal to 0 indicates that band units
of uniform size are used.

band_block_in_luma_samples specifies the horizontal and vertical size of the band block in samples of
component 0 as 16 << band_block_in_luma_samples.

vert_size_in_band_blocks_minus_1 plus 1 specifies the size of the r-th band unit row as
bandBlockInSamples * (vert_size_in_band_blocks_minus_1[ r ] + 1 ) in component 0 samples when
varying_size_band_units_flag is equal to 1.



AV2 Specification                                                                           Page 314 of 1169
    horz_size_in_band_blocks_minus_1 plus 1 specifies the size of the c-th band unit column as
    bandBlockInSamples * (horz_size_in_band_blocks_minus_1[ c ] + 1 ) in component 0 samples when
    varying_size_band_units_flag is equal to 1.

    Band units boundaries are aligned across components, taking into account possible component
    subsampling.

    banding_in_band_unit_present_flag equal to 1 indicates banding is present in band unit in row r,
    column c. banding_in_band_unit_present_flag[ r ][ c ] equal to 0 indicates that banding is not present in
    band unit in row r, column c.

```

<a id="s-6-16-9"></a>

#### § 6.16.9 Metadata ICC profile semantics

```text
§   6.16.9. Metadata ICC profile semantics

    icc_profile_data_payload_bytes shall be bytes containing data corresponding to a profile from the
    International Color Consortium.

    The variable ICCmajorVer is set equal to icc_profile_data_payload_bytes[ 8 ] and the variable ICCminorVer
    is set equal to icc_profile_data_payload_bytes[ 9 ] >> 4.

    icc_profile_data_payload_bytes contains data with syntax and semantics specified according to the
    interpretation of ICCmajorVer and ICCminorVer as follows:

         ICCmajorVer          ICCminorVer                                       Interpretation

     4                    2                 Major profile 4 and minor profile 2 version as specified in ISO 15076-1

     4                    3                 Major profile 4 and minor profile 3 version as specified in ISO 15076-1

     4                    4                 Major profile 4 and minor profile 4 version as specified in ISO 15076-1

     5                    0                 Major profile 5 and minor profile 0 version as specified in ISO 20677


    Values of ICCmajorVer and ICCminorVer that are not listed are unspecified or specified by other means.

```

<a id="s-6-16-10"></a>

#### § 6.16.10 Metadata scan type semantics

```text
§   6.16.10. Metadata scan type semantics

    This metadata allows decoded frames to be interpreted as either progressive or interlaced content.

    These values have no normative effect on the decoding process which is still frame based.

    The prefix mps stands for metadata picture structure.

    mps_pic_struct_type indicates whether a picture should be displayed as a frame or as one or more fields
    and, for the display of frames when equal_picture_interval is equal to 1, whether such frame should be
    repeated or not when output on certain devices.

    The interpretation of mps_pic_struct_type is specified in Table 6.18:

                        Table 6.18: mps_pic_struct_type values and picture output interpretations

     Value           Indicated picture output             Elemental                             Restrictions
                                                            Units

     0       Frame                                    1                 ci_scan_type_idc shall be equal to 1

     1       Top field                                1                 ci_scan_type_idc shall be equal to 2

     2       Bottom field                             1                 ci_scan_type_idc shall be equal to 2



    AV2 Specification                                                                                                 Page 315 of 1169
     3       Top field, bottom field in that order        2   ci_scan_type_idc shall be equal to 3

     4       Bottom field, top field in that order        2   ci_scan_type_idc shall be equal to 3

     5       Top field, bottom field, top field           3   ci_scan_type_idc shall be equal to 3
             repeated, in that order

     6       Bottom field, top field, bottom field        3   ci_scan_type_idc shall be equal to 3
             repeated, in that order

     7       Frame doubling                               2   ci_scan_type_idc shall be equal to 1 and
                                                              equal_picture_interval shall be equal to 1

     8       Frame tripling                               3   ci_scan_type_idc shall be equal to 1 and
                                                              equal_picture_interval shall be equal to 1

     9       Top field paired with previous bottom        1   ci_scan_type_idc shall be equal to 2
             field in output order

     10      Bottom field paired with previous top        1   ci_scan_type_idc shall be equal to 2
             field in output order

     11      Top field paired with next bottom field in   1   ci_scan_type_idc shall be equal to 2
             output order

     12      Bottom field paired with next top field in   1   ci_scan_type_idc shall be equal to 2
             output order


    Values of mps_pic_struct_type above 12 are reserved for future use by AOMedia and shall not be present
    in bitstreams conforming to this specification.

    Decoders shall ignore reserved values of mps_pic_struct_type.

    It is a requirement of bitstream conformance that when mps_pic_struct_type is present that only one of
    the following conditions, for all pictures in the current CVS, is true: – The value of mps_pic_struct_type is
    equal to 0, 7 or 8. – The value of mps_pic_struct_type is equal to 1, 2, 9, 10, 11 or 12. – The value of
    mps_pic_struct_type is equal to 3, 4, 5 or 6.

    mps_source_scan_type_idc specifies the scan type with the same semantics as for ci_scan_type_idc.

    mps_duplicate_flag indicates whether the current picture should be indicated as a duplicate of a
    previous picture in output order. When mps_duplicate_flag is equal to 1 the current picture is indicated to
    be a duplicate of the previous picture. When mps_duplicate_flag is equal to 0 the current picture is not
    indicated to be a duplicate of the previous picture.

```

<a id="s-6-16-11"></a>

#### § 6.16.11 Metadata temporal point info semantics

```text
§   6.16.11. Metadata temporal point info semantics

    It is a requirement of bitstream conformance that metadata_type equal to
    METADATA_TYPE_TEMPORAL_POINT_INFO shall only appear in an OBU with obu_type equal to
    OBU_METADATA_SHORT.


      NOTE: A metadata_type of METADATA_TYPE_TEMPORAL_POINT_INFO is only allowed in OBUs
      with obu_type equal to OBU_METADATA_SHORT to make parsing simpler for application layers.


    frame_presentation_time specifies the presentation time of the frame in clock ticks DispCT counted
    from the presentation time of the previous random access point for the operating point that is being
    decoded if the current frame is a leading frame or is associated with a random access point. It specifies
    the presentation time of the frame in clock ticks DispCT counted from the presentation time of the most




    AV2 Specification                                                                                      Page 316 of 1169
    recent random access point if the current frame is not a leading frame and is not associated with a
    random access point.

```

<a id="s-6-16-12"></a>

#### § 6.16.12 Metadata user data unregistered semantics

```text
§   6.16.12. Metadata user data unregistered semantics

    uuid_iso_iec_11578 specifies a UUID value that conforms to the procedures in Annex A of ISO/IEC
    11578:1996.

    user_data_payload_byte specifies a byte of data whose structure and meaning are determined by the
    UUID. This standard does not specify or restrict the format or interpretation of the
    user_data_payload_byte payload bytes.

```

<a id="s-6-16-13"></a>

#### § 6.16.13 Metadata decoded frame hash semantics

```text
§   6.16.13. Metadata decoded frame hash semantics

    This metadata contains hash values that are calculated for the output frames. Generation of hash values
    should use the procedure below to ensure the correct interpretation of those values.

    Output frames are prepared by the output process specified in § 7.21.1 Output process.

    Let bitDepth, w, h, subX, subY be the values of the corresponding local variables at the end of the output
    process.

    The hash is computed on the cropped frame dimensions as specified by w and h.

    If has_grain is equal to 0, let decodedSamples[0]/decodedSamples[1]/decodedSamples[2] be the values of
    OutY/OutU/OutV generated by the intermediate output preparation process specified in § 7.21.2
    Intermediate output preparation process.

    If has_grain is equal to 1, let decodedSamples[0]/decodedSamples[1]/decodedSamples[2] be the values of
    OutY/OutU/OutV at the end of the output process.


      NOTE:       It is legal to set has_grain equal to 1 even if the sequence is not using film grain.


    Prior to computing the hash, decoded sample values are converted to byte arrays as follows.

     numPlanes = is_monochrome ? 1 : 3
     for (planeIdx = 0; planeIdx < numPlanes; planeIdx++) {
         if (planeIdx == 0) {
             planeWidth = w
             planeHeight = h
         } else {
             planeWidth = (w + subX) >> subX
             planeHeight = (h + subY) >> subY
         }
         byteIdx = 0
         for (row = 0; row < planeHeight; row++) {
             for (col = 0; col < planeWidth; col++) {
                  sample = decodedSamples[planeIdx][row][col]
                  planeData[planeIdx][byteIdx++] = sample & 0xFF
                  if ( bitDepth > 8 ) {
                      planeData[planeIdx][byteIdx++] = sample >> 8
                  }
             }
         }
         planeDataLength[planeIdx] = byteIdx
     }




    AV2 Specification                                                                                     Page 317 of 1169
Samples are processed in raster scan order (left to right, top to bottom) within each plane. 8-bit samples
(bitDepth equal to 8) are written as a single byte. Samples with bitDepth greater than 8 are written as
two bytes in little-endian order (LSB first, then MSB). For monochrome frames (is_monochrome equal to
1), only the Y plane (planeIdx equal to 0) is processed.

hash_type specifies the hash algorithm used to compute the frame hash.

When hash_type equals 0, the hash is computed using MD5 as specified by [RFC1321]. The MD5
computation is performed as follows:

When per_plane equals 1 (separate hash per plane):

 for (planeIdx = 0; planeIdx < numPlanes; planeIdx++) {
     MD5Init(context)
     MD5Update(context, planeData[planeIdx], planeDataLength[planeIdx])
     MD5Final(plane_hash[planeIdx], context)
 }


When per_plane equals 0 (single hash for all planes):

 MD5Init(context)
 for (planeIdx = 0; planeIdx < numPlanes; planeIdx++) {
     MD5Update(context, planeData[planeIdx], planeDataLength[planeIdx])
 }
 MD5Final(frame_hash, context)


where MD5Init, MD5Update, and MD5Final are the functions defined in [RFC1321].

All other values of hash_type are reserved for future use by AOMedia.

per_plane equal to 1 specifies that the hash is computed separately for each plane. When per_plane is
equal to 0, a single hash is computed for all planes combined.

has_grain equal to 1 specifies that the hash is computed on the decoded frame after film grain synthesis
has been applied according to the film grain synthesis process specified in § 7.21.7 Film grain synthesis
process. When has_grain is equal to 0, the hash is computed on the raw decoded frame.

is_monochrome equal to 1 specifies that the frame has a single plane (monochrome). When
is_monochrome is equal to 0, the frame has 3 planes. This field is only used when per_plane is equal to 1
to determine the number of plane_hash array elements to read.

reserved shall be set to 0 and ignored by decoders. This bit is reserved for future use by AOMedia.

plane_hash[ planeIdx ] is an array containing 16 bytes (128 bits) of hash data for each plane. Each
plane_hash[ planeIdx ] element is computed over the corresponding plane’s samples in raster scan order
using the algorithm specified by hash_type. This array is present when per_plane is equal to 1. When
is_monochrome is equal to 1, only plane_hash[ 0 ] (Y plane) is present. When is_monochrome is equal to
0, three elements are present: plane_hash[ 0 ] for Y, plane_hash[ 1 ] for U, and plane_hash[ 2 ] for V.

frame_hash contains 16 bytes (128 bits) of hash data for the entire frame. When multiple planes are
present, the hash is computed over all planes' samples in plane order (Y, then U, then V) using the
algorithm specified by hash_type. This syntax element is present when per_plane is equal to 0.




AV2 Specification                                                                            Page 318 of 1169
```

<a id="s-6-17"></a>

### § 6.17 Frame header OBU semantics

```text
§   6.17. Frame header OBU semantics
```

<a id="s-6-17-1"></a>

#### § 6.17.1 General frame header semantics

```text
§   6.17.1. General frame header semantics

    It is a requirement of bitstream conformance that a sequence header OBU has been received before a
    frame header.

    If isFirst is equal to 1, it is a requirement of bitstream conformance that SeenFrameHeader is equal to 0.

    If isFirst is equal to 0, it is a requirement of bitstream conformance that SeenFrameHeader is equal to 1.

    frame_header_copy is a syntax structure that contains an identical copy of the bits sent in the
    frame_header for the first tile group.


      NOTE: When a frame header is present for the second tile group onwards, a decoder can choose to
      either read the syntax elements or to simply skip over the bits.


    header_bit[ i ] contains a copy of a bit from the frame_header syntax structure sent with the first tile
    group in the frame.

    It is a requirement of bitstream conformance that header_bit[ i ] is equal to the value of the bit at offset i
    from the start of the frame_header structure sent with the first tile group.


      NOTE: The contents of frame_header are copied bit for bit but this does not include the bits sent
      before frame_header. This means that the duplicate copies have a different bit alignment within bytes
      when compared to the original version.


    TileNum is a variable giving the index (zero-based) of the current tile.

    decode_frame_wrapup is a function call that indicates that the decode frame wrapup process specified
    in § 7.2 Decode frame wrapup process is invoked.

```

<a id="s-6-17-2"></a>

#### § 6.17.2 Frame header info semantics

```text
§   6.17.2. Frame header info semantics

    bridge_frame_ref_idx specifies which reference frame is used in a Bridge frame.


      NOTE: The Bridge frame represents the same temporal instant as its reference frame at a different
      resolution. As such, it inherits the same order hint.


    cur_mfh_id specifies which multi-frame header to use.

    If cur_mfh_id is greater than 0, it is a requirement of bitstream conformance that a multi-frame header
    OBU with mfh_id_minus_1 equal to cur_mfh_id - 1 is present in the bitstream at some point before the
    syntax element cur_mfh_id, or is available through external means.

    seq_header_id_in_frame_header specifies which sequence header is associated with this frame.

    load_sequence_header( id ) specifies that all the syntax elements and variables saved by a previous call
    to save_sequence_header are loaded from the area of memory indexed by id.

    It is a requirement of bitstream conformance that id corresponds to an area of memory that was saved.



    AV2 Specification                                                                               Page 319 of 1169
After the sequence header is loaded, if cur_mfh_id is greater than 0, it is a requirement of bitstream
conformance that all the following are true:

  • mfh_frame_width_minus_1[ cur_mfh_id ] is less than or equal to max_frame_width_minus_1.
  • mfh_frame_height_minus_1[ cur_mfh_id ] is less than or equal to max_frame_height_minus_1.
  • MLayerDependencyMap[ obu_mlayer_id ][ MfhMLayerId[ cur_mfh_id ] ] is equal to 1.
  • TLayerDependencyMap[ obu_mlayer_id ][ obu_tlayer_id ][ MfhTLayerId[ cur_mfh_id ] ] is equal to 1.

FirstPictureInTU is a variable that specifies if this is the first frame unit in a coded extended layer unit
in a temporal unit.

startCVS specifies if this is the start of a new coded video sequence.

activate_layer_configuration_record( id ) specifies that the layer configuration records corresponding
to the given id are activated.

A lcr_local_info syntax structure is activated if lcr_local_id[ obu_xlayer_id ] is equal to id. Otherwise (if
there is no lcr_local_info syntax structure with lcr_local_id[ obu_xlayer_id ] equal to id), a lcr_global_info
syntax structure is activated if the value of lcr_global_config_record_id is equal to id.

ShowExistingFrame equal to 1 indicates the frame indexed by frame_to_show_map_idx is to be output;
ShowExistingFrame equal to 0 indicates that further processing is required.

frame_to_show_map_idx specifies the frame to be output. It is only available if ShowExistingFrame is 1.

derive_sef_order_hint specifies how the order hint for the show existing frame is derived.
derive_sef_order_hint equal to 1 specifies that the order hint is derived from the reference frame.
derive_sef_order_hint equal to 0 specifies that the order hint is explicitly signaled via the syntax element
sef_order_hint.

If derive_sef_order_hint is equal to 1, it is a requirement of bitstream conformance that all of the
following are true:

  • the reference frame at slot frame_to_show_map_idx has not already been shown.
  • RefImplicitOutputFrame[ frame_to_show_map_idx ] is equal to 0.
  • RefImmediateOutputFrame[ frame_to_show_map_idx ] is equal to 0.

sef_order_hint is used to compute OrderHint.

FrameType specifies the type of the frame:

                    FrameType                                      Name of FrameType

 0                                        KEY_FRAME

 1                                        INTER_FRAME

 2                                        INTRA_ONLY_FRAME

 3                                        SWITCH_FRAME


restricted_prediction_switch equal to 1 specifies that all available reference frames will be marked as
restricted.



AV2 Specification                                                                                Page 320 of 1169
  NOTE: This allows future frames to use sample values from both the switch frame and other
  reference frames. However, the other reference frames are marked as restricted to indicate that only
  the sample values can be used, and not any of the other information associated with a reference
  frame. This is needed because switch frames switch between bitstreams so the other information is
  not consistent and cannot be used for parsing syntax elements.


frame_is_inter equal to 1 specifies that the frame is an inter frame and can use inter prediction.
frame_is_inter equal to 0 specifies that the frame is an intra frame and shall use only intra prediction.

long_term_id_plus_1 minus 1 specifies a long term id number for the current frame.

num_key_ref_frames specifies the number of ref_long_term_id syntax elements to be read.

ref_long_term_id[ i ] specifies a value of long term id for a reference frame. It is a requirement of
bitstream conformance that the value of ref_long_term_id[ i ] shall not be equal to (1 <<
long_term_frame_id_bits) - 1.


  NOTE: For RAS frames, the ref_long_term_id is used to restrict the reference frames allowed to just
  the long term reference frames with matching long term ids. Not all long term reference frames need
  to be mentioned in this list, but only the mentioned ones can be used.


  NOTE: It is legal for the RAS frame to use multiple long term reference frames that share the same
  value of long term id.


  NOTE: It is recommended (but not a bitstream constraint), that the ref_long_term_id array does not
  contain duplicates. Duplicate entries have no effect on the decoding process - this note is included to
  ensure that decoders do not assume the values in ref_long_term_id are unique.


immediate_output_frame equal to 1 specifies that this frame shall be immediately queued for output
once decoded. This frame may also be additionally output using SEF OBUs. immediate_output_frame
equal to 0 specifies that this frame should not be immediately queued for output and that the output of
this frame depends on additional syntax elements in the bitstream.

If still_picture is equal to 1, it is a requirement of bitstream conformance that FrameType is equal to
KEY_FRAME and immediate_output_frame is equal to 1.

output_frame_buffers( i ) is a function call that indicates that the output frame buffers process
specified in § 7.21.6 Output frame buffers process is invoked with i as input.

implicit_output_frame equal to 1 specifies that the frame will be output by the output frame buffers
process specified in § 7.21.6 Output frame buffers process. This frame can also be additionally output
using SEF OBUs. implicit_output_frame equal to 0 specifies that the frame is not output using the output
frame buffers process but can be output using SEF OBUs. When not present, the value of
implicit_output_frame is equal to 0.




AV2 Specification                                                                              Page 321 of 1169
  NOTE: Due to the bitstream constraints in AV2, an OLK frame is required to be an implicit output
  frame by itself, or be present together with another output Regular frame in the same coded
  extended layer unit that only depends on the OLK frame. Consequently, when
  monotonic_output_order_flag is equal to 1, the temporal unit containing the OLK will result in a frame
  that is output before any leading frames. It is not legal to use an obu_type that marks this as a
  leading frame. This may result in the Regular frame being shown as the first frame before the OLK at
  an open random access point, potentially with skipped leading frames (and a gap in display time)
  between them.


frame_size_override_flag equal to 0 specifies that the frame size is equal to the size in the sequence
header. frame_size_override_flag equal to 1 specifies that the frame size will either be specified as the
size of one of the reference frames, or computed from the frame_width_minus_1 and
frame_height_minus_1 syntax elements.

order_hint is used to compute OrderHint.

OrderHintLsbs specifies OrderHintBits least significant bits of the expected output order for this frame.

OrderHint specifies the expected output order for this frame.


  NOTE: There is no requirement that OrderHint should reflect the true output order. As a guideline,
  the motion vector prediction is expected to be more accurate if the true output order is used for
  frames that will be shown later. If a frame is never to be shown (e.g., it has been constructed as an
  average of several frames for reference purposes), the encoder is free to choose whichever value of
  OrderHint will give the best compression.


signal_primary_ref_frame specifies that the primary_ref_frame syntax element is present.

disable_cross_frame_cdf_init equal to 1 specifies that the CDF values are set to default values instead
of being taken from a reference frame. disable_cross_frame_cdf_init equal to 0 specifies that the CDF
values can be taken from another reference frame (depending on the value of other syntax elements).


  NOTE: The intention of setting disable_cross_frame_cdf_init equal to 1 is to allow frames to be
  arithmetically decoded in parallel.


primary_ref_frame specifies the reference frame which contains the CDF values and other state that are
loaded at the start of the frame.

It is a requirement of bitstream conformance that when primary_ref_frame is present in the bitstream
primary_ref_frame is either equal to PRIMARY_REF_NONE, or primary_ref_frame is less than
NumTotalRefs.


  NOTE:       NumTotalRefs will be computed later in the decode process.


If primary_ref_frame is not equal to PRIMARY_REF_NONE, it is a requirement of bitstream conformance
that OrderHints[ primary_ref_frame ] is not equal to RESTRICTED_OH.

change_drl equal to 1 indicates that max_drl_bits_minus_1 is changed from the value in the sequence
header.


AV2 Specification                                                                             Page 322 of 1169
max_drl_bits_minus_1 plus 1 specifies the maximum number of times the drl_mode syntax element is
read within read_drl_idx.

flush_implicit_output_frames( ) is a function call that indicates that the flush implicit output frames
process specified in § 7.21.5 Flush implicit output frames process is invoked.

bridge_frame_overwrite_flag equal to 1 specifies that the syntax element refresh_frame_flags is
present. bridge_frame_overwrite_flag equal to 0 specifies that refresh_frame_flags is not present and is
inferred to be equal to 1 << bridge_frame_ref_idx.

has_refresh_frame_flags equal to 1 specifies that the syntax element frame_to_refresh is present.
has_refresh_frame_flags equal to 0 specifies that the syntax element frame_to_refresh is not present and
that refresh_frame_flags is inferred equal to 0.

frame_to_refresh specifies which reference frame slot will be updated with the current frame after it is
decoded.

It is a requirement of bitstream conformance that frame_to_refresh is less than NumRefFrames.

refresh_frame_flags contains a bitmask that specifies which reference frame slots will be updated with
the current frame after it is decoded.

If FrameType is equal to INTRA_ONLY_FRAME and NumRefFrames is greater than 1, it is a requirement
of bitstream conformance that refresh_frame_flags is not equal to (1 << NumRefFrames) - 1.


  NOTE: This restriction encourages encoders to correctly label random access points (by forcing
  FrameType to be equal to KEY_FRAME when an intra frame is used to reset the decoding process).


If IsRegular is equal to 0 (i.e., this is a leading frame), it is a requirement of bitstream conformance that
refresh_frame_flags & OlkRefresh[ i ] is equal to 0 for all i = 0..MAX_NUM_MLAYERS-1.


  NOTE: This restriction forbids leading frames from overwriting frames that will be used by regular
  frames. This is needed to allow random access decoding to operate correctly.


See § 7.23 Reference frame update process for details of the frame update process.

If immediate_output_frame is equal to 0, it is a requirement of bitstream conformance that the value of
refresh_frame_flags is not equal to 0.


  NOTE: This restriction also applies if the value of refresh_frame_flags is inferred from other syntax
  elements.


If obu_type is equal to OBU_RAS_FRAME, refresh_frame_flags must be set to refresh all short term
frames that are present in the current embedded layer or any layer that depends on the current
embedded layer (long term frames may or may not be refreshed).

frame_explicit_ref_frame_map equal to 1 specifies that num_total_refs is present in this frame to
override the default number of reference frames. frame_explicit_ref_frame_map equal to 0 specifies that
num_total_refs is not present and the default number of reference frames is used.




AV2 Specification                                                                               Page 323 of 1169
num_total_refs allows the number of references for this frame to be adjusted from the default values.

If num_total_refs is present, it is a requirement of bitstream conformance that num_total_refs is less than
or equal to ActiveNumRefFrames.

use_bru equal to 1 specifies that this frame does a backwards reference update.

bru_ref specifies which reference is updated.

bru_inactive equal to 1 specifies that the whole frame is inactive.

If use_bru is equal to 1, it is a requirement of bitstream conformance that all the following are true:

  • OrderHint is greater than or equal to RefOrderHint[ i ] for i in the range 0..NumRefFrames-1 where
    RefValid[ i ] is equal to 1,
  • immediate_output_frame is equal to 1,
  • bru_ref is less than NumTotalRefs,
  • RefOrderHint[ ref_frame_idx[ bru_ref ] ] is not equal to RESTRICTED_OH,
  • RefFrameWidth[ ref_frame_idx[ bru_ref ] ] is equal to FrameWidth,
  • RefFrameHeight[ ref_frame_idx[ bru_ref ] ] is equal to FrameHeight,
  • The value of refresh_frame_flags & (1 << ref_frame_idx[ bru_ref ] ) must be non-zero.

get_ref_frames is a function call that indicates the conceptual point where the default ref_frame_idx
values are prepared. When this function is called, the get ref frames process specified in § 7.7 Get ref
frames process is invoked.

get_past_future_cur_ref_lists is a function call that indicates the get past future cur ref lists process
process specified in § 7.8 Get past future cur ref lists process is invoked.

ref_frame_idx[ i ] specifies which reference frames are used by inter frames. It is a requirement of
bitstream conformance that RefValid[ ref_frame_idx[ i ] ] is equal to 1, and that the selected reference
frames match the current frame in bit depth, profile, chroma subsampling, and color space.


  NOTE: Syntax elements indicate a reference (an integer between 0 and 6). These references are
  looked up in the ref_frame_idx array to find the reference frame which is to be used during inter
  prediction. There is no requirement that the values in ref_frame_idx are distinct.


If obu_type is equal to OBU_RAS_FRAME, it is a requirement of bitstream conformance that
long_term_id_in_use( RefLongTermId[ ref_frame_idx[ i ] ] ) is equal to 1.


It is a requirement of bitstream conformance that MLayerDependencyMap[ obu_mlayer_id ]
[ RefMLayerId[ ref_frame_idx[ i ] ] ] is equal to 1.

It is a requirement of bitstream conformance that TLayerDependencyMap[obu_mlayer_id]
[ obu_tlayer_id ][ RefTLayerId[ ref_frame_idx[ i ] ] ] is equal to 1.

If use_bru is equal to 1, it is a requirement of bitstream conformance that the
RefCounter[ref_frame_idx[bru_ref]] is not the same as RefCounter[ref_frame_idx[i]] for any value of i not
equal to bru_ref in the range 0..NumTotalRefs-1.



AV2 Specification                                                                              Page 324 of 1169
  NOTE: This constraint means that it is not legal to store a decoded frame into two reference frames
  via the refresh_frame_flags mechanism, and then only update one of the reference frames via a
  backwards reference update. This means an implementation of a decoder can keep a single copy of
  each decoded frame.


Once the frame size has been determined, it is a requirement of bitstream conformance that all the
following conditions are satisfied for i=0..NumTotalRefs-1:

  • 2 * FrameWidth >= RefFrameWidth[ ref_frame_idx[ i ] ]
  • 2 * FrameHeight >= RefFrameHeight[ ref_frame_idx[ i ] ]
  • FrameWidth <= 16 * RefFrameWidth[ ref_frame_idx[ i ] ]
  • FrameHeight <= 16 * RefFrameHeight[ ref_frame_idx[ i ] ]

use_qtr_precision_mv equal to 1 specifies that motion vectors are specified to quarter pel precision.

allow_high_precision_mv equal to 0 specifies that motion vectors are specified to half pel precision;
allow_high_precision_mv equal to 1 specifies that motion vectors are specified to eighth pel precision.

FrameMvPrecision specifies the default precision used for specifying motion vectors as specified in
Table 6.19:

                             Table 6.19: FrameMvPrecision values and names

               FrameMvPrecision                                Name of FrameMvPrecision

 0                                           MV_PRECISION_EIGHT_PEL

 1                                           MV_PRECISION_FOUR_PEL

 2                                           MV_PRECISION_TWO_PEL

 3                                           MV_PRECISION_ONE_PEL

 4                                           MV_PRECISION_HALF_PEL

 5                                           MV_PRECISION_QUARTER_PEL

 6                                           MV_PRECISION_EIGHTH_PEL

 7                                           NUM_MV_PRECISIONS


frame_enabled_motion_modes specifies which motion modes are allowed in this frame.

use_ref_frame_mvs equal to 1 specifies that motion vector information from a previous frame can be
used when decoding the current frame. use_ref_frame_mvs equal to 0 specifies that this information will
not be used.

tmvp_sample_step_minus_1 plus 1 specifies the step used during temporal motion vector prediction. A
higher step means that motion vectors are projected at fewer locations and the motion field is
interpolated at the locations that have been stepped over.

allow_df_sub_pu equal to 1 specifies that the deblocking filter filters subblock edges within prediction
units. allow_df_sub_pu equal to 0 specifies that the deblocking filter does not filter subblock edges.

TipFrameMode specifies how TIP frames are generated and used as specified in Table 6.20:



AV2 Specification                                                                            Page 325 of 1169
                                   Table 6.20: TipFrameMode values and names

                    TipFrameMode                                   Name of TipFrameMode

 0                                            TIP_FRAME_DISABLED

 1                                            TIP_FRAME_AS_REF

 2                                            TIP_FRAME_AS_OUTPUT



  NOTE: TIP_FRAME_DISABLED means no TIP will be used. TIP_FRAME_AS_REF means individual
  blocks can be coded as TIP blocks. TIP_FRAME_AS_OUTPUT means that the whole frame is
  automatically generated from TIP blocks.


tip_frame_mode equal to 1 specifies that TipFrameMode is equal to TIP_FRAME_AS_REF.
tip_frame_mode equal to 0 specifies that TipFrameMode is equal to TIP_FRAME_DISABLED.

If is_tip_frame() is equal to 1, it is a requirement of bitstream conformance that the computed value for
TipFrameMode is equal to TIP_FRAME_AS_OUTPUT.

allow_tip_hole_fill equal to 1 specifies that holes in the Temporally Interpolated Prediction (TIP) motion
field are filled in using interpolation. allow_tip_hole_fill equal to 0 specifies that holes in the TIP motion
field are not filled.

apply_deblocking_filter_tip specifies if the deblocking filter is applied after computing the TIP frame.

tip_global_wtd_index specifies an index that chooses the weighting factor of the two reference frames
used in TIP.

tip_mv_zero equal to 1 indicates that TipGlobalMv is equal to 0. tip_mv_zero equal to 0 indicates that
additional syntax elements are read to compute TipGlobalMv.

TipGlobalMv is the TIP global motion vector (this provides an offset to the normal TIP motion vectors).

tip_mv_row and tip_mv_col give the absolute value of the TIP global motion vector.

tip_mv_row_sign and tip_mv_col_sign give the sign of the TIP global motion vector.

tip_sharp and tip_regular specify the type of interpolation used in the TIP process.

disable_cdf_update equal to 1 specifies that the CDF update in the symbol decoding process is disabled
and CDFs shall not be modified during decoding of this frame. disable_cdf_update equal to 0 specifies
that CDF updates are enabled and CDFs can be modified during decoding.

qm_index specifies which entry in the qm_y, qm_u, qm_v arrays gives the quantization matrix level for a
particular segment.

It is a requirement of bitstream conformance that qm_index is less than or equal to pic_qm_num_minus_1.

allow_tcq equal to 1 specifies that Trellis Coded Quantization (TCQ) is enabled for this frame. allow_tcq
equal to 0 specifies that TCQ is disabled for this frame.

motion_field_estimation is a function call which indicates that the motion field estimation process in
§ 7.9 Motion field estimation process is invoked.



AV2 Specification                                                                               Page 326 of 1169
setup_tip_motion_field is a function call which indicates that the setup TIP motion field process in
§ 7.10 Setup TIP motion field process is invoked.

fill_tpl_mvs_sample_gap is a function call which indicates that the fill temporal motion vectors sample
gap process specified in § 7.10.5 Fill temporal motion vectors sample gap process is invoked.

OrderHints specifies the expected output order for each reference frame.

CodedLossless is a variable that is equal to 1 when all segments use lossless encoding. In this case, the
deblocking filter, CDEF filter, and loop restoration filters are disabled.

It is a requirement of bitstream conformance that delta_q_present is equal to 0 when CodedLossless is
equal to 1.


  NOTE: In a mixed lossy-lossless encode (when CodedLossless is false and HasLosslessSegment is true), to
  guarantee lossless reconstruction for chroma pixels belonging to a lossless segment and that are
  coded as part of a chroma block covering multiple luma blocks (with potentially different segment_ids),
  the co-located luma block from which the chroma block inherits its segment_id must also be coded in
  lossless mode. There are two scenarios where a chroma block may correspond to multiple luma
  blocks. These two scenarios must be handled as follows:

     • In a chroma merge region, where luma blocks may be split but the chroma block remains unsplit,
       the luma block co-located with the bottom-right corner of the chroma block must be coded in
       lossless mode.
     • In the case of SDP, where luma and chroma blocks may follow different partitioning structures,
       the luma block co-located with the top-left corner of the chroma block must be coded in lossless
       mode.

  A simpler but arguably more restrictive way to achieve lossless chroma coding in a mixed lossy-
  lossless encode is to turn off SDP and restrict the minimum partition width and height to 8.


allow_parity_hiding equal to 1 specifies that this frame can hide the parity of some DC coefficients.

allow_bawp equal to 1 indicates that the syntax element use_bawp can be present. allow_bawp equal to
0 indicates that the syntax element use_bawp is not present. (this means that BAWP cannot be signaled if
allow_bawp is equal to 0.)

allow_warpmv_mode equal to 1 indicates that the syntax element warp_mv can be present.
allow_warpmv_mode equal to 0 indicates that the syntax element warp_mv is not present. (This means
that YMode cannot be equal to WARPMV if allow_warpmv_mode is equal to 0.)

reduced_tx_set greater than 0 specifies that the frame is restricted to a reduced subset of the full set of
transform types.


  NOTE: reduced_tx_set can take values between 0 and 3. The value of reduced_tx_set (along with
  the size of the block and whether the block is inter or intra) is used in get_tx_set to determine a set of
  allowed transform types. The set is used in transform_type to read the luma transform type. The set is
  also used in compute_tx_type to work out the transform type for the current block.




AV2 Specification                                                                              Page 327 of 1169
setup_past_independence is a function call that indicates that this frame can be decoded without
dependence on previous coded frames. When this function is invoked the following takes place:

  • FeatureData[ i ][ j ] and FeatureEnabled[ i ][ j ] are set equal to 0 for i = 0..MAX_SEGMENTS-1 and j
    = 0..SEG_LVL_MAX-1.
  • PrevSegmentIds[ row ][ col ] is set equal to 0 for row = 0..MiRows-1 and col = 0..MiCols-1.
  • PrevGmParams[ ref ][ i ] is set equal to ( ( i % 3 == 2 ) ? 1 << WARPEDMODEL_PREC_BITS : 0 ) for ref =
    0..REFS_PER_FRAME - 1, for i = 0..5.
  • ccso_planes[ plane ] is set equal to 0 for plane = 0..2.

init_non_coeff_cdfs is a function call that initializes the CDF tables which are not used in the coeffs( )
syntax structure. When this function is invoked, the following steps apply:

  • WarpMvCdf is set to a copy of Default_Warp_Mv_Cdf.
  • TipPredModeCdf is set to a copy of Default_Tip_Pred_Mode_Cdf.
  • WarpIdxCdf is set to a copy of Default_Warp_Idx_Cdf.
  • WarpWithMvdCdf is set to a copy of Default_Warp_With_Mvd_Cdf.
  • IsWarpCdf is set to a copy of Default_Is_Warp_Cdf.
  • UseGdfCdf is set to a copy of Default_Use_Gdf_Cdf.
  • BruModeCdf is set to a copy of Default_Bru_Mode_Cdf.
  • CdefIndex0Cdf is set to a copy of Default_Cdef_Index0_Cdf.
  • CdefIndexMinus1With3Cdf is set to a copy of Default_Cdef_Index_Minus1_With3_Cdf.
  • CdefIndexMinus1With4Cdf is set to a copy of Default_Cdef_Index_Minus1_With4_Cdf.
  • CdefIndexMinus1With5Cdf is set to a copy of Default_Cdef_Index_Minus1_With5_Cdf.
  • CdefIndexMinus1With6Cdf is set to a copy of Default_Cdef_Index_Minus1_With6_Cdf.
  • CdefIndexMinus1With7Cdf is set to a copy of Default_Cdef_Index_Minus1_With7_Cdf.
  • CdefIndexMinus1With8Cdf is set to a copy of Default_Cdef_Index_Minus1_With8_Cdf.
  • WarpDeltaPrecisionCdf is set to a copy of Default_Warp_Precision_Cdf.
  • WarpDeltaParamLowCdf is set to a copy of Default_Warp_Delta_Param_Low_Cdf.
  • WarpDeltaParamHighCdf is set to a copy of Default_Warp_Delta_Param_High_Cdf.
  • WarpDeltaParamSignCdf is set to a copy of Default_Warp_Delta_Param_Sign_Cdf.
  • YModeSetCdf is set to a copy of Default_Y_Mode_Set_Cdf.
  • YModeIndexCdf is set to a copy of Default_Y_Mode_Index_Cdf.
  • YModeOffsetCdf is set to a copy of Default_Y_Mode_Offset_Cdf.
  • CwpIdxCdf is set to a copy of Default_Cwp_Idx_Cdf.
  • FscModeCdf is set to a copy of Default_Fsc_Mode_Cdf.
  • MrlIndexCdf is set to a copy of Default_Mrl_Index_Cdf.
  • MrlSecIndexCdf is set to a copy of Default_Mrl_Sec_Index_Cdf.
  • UseDpcmYCdf is set to a copy of Default_Use_Dpcm_Y_Cdf.



AV2 Specification                                                                               Page 328 of 1169
  • DpcmModeYCdf is set to a copy of Default_Dpcm_Mode_Y_Cdf.
  • UseDpcmUvCdf is set to a copy of Default_Use_Dpcm_UV_Cdf.
  • DpcmModeUvCdf is set to a copy of Default_Dpcm_Mode_UV_Cdf.
  • UVModeCflNotAllowedCdf is set to a copy of Default_Uv_Mode_Cfl_Not_Allowed_Cdf.
  • IsCflCdf is set to a copy of Default_Is_Cfl_Cdf.
  • IntrabcCdf is set to a copy of Default_Intrabc_Cdf.
  • IntrabcPrecisionCdf is set to a copy of Default_Intrabc_Precision_Cdf.
  • IntrabcModeCdf is set to a copy of Default_Intrabc_Mode_Cdf.
  • MorphPredCdf is set to a copy of Default_Morph_Pred_Cdf.
  • RegionTypeCdf is set to a copy of Default_Region_Type_Cdf.
  • DipModeCdf is set to a copy of Default_Dip_Mode_Cdf.
  • UseDipCdf is set to a copy of Default_Use_Dip_Cdf.
  • DoSquareSplitCdf is set to a copy of Default_Do_Square_Split_Cdf.
  • DoSplitCdf is set to a copy of Default_Do_Split_Cdf.
  • RectTypeCdf is set to a copy of Default_Rect_Type_Cdf.
  • DoExtPartitionCdf is set to a copy of Default_Do_Ext_Partition_Cdf.
  • DoUneven4wayPartitionCdf is set to a copy of Default_Do_Uneven_4way_Partition_Cdf.
  • SegIdExtFlagCdf is set to a copy of Default_Seg_Id_Ext_Flag_Cdf.
  • SegmentIdCdf is set to a copy of Default_Segment_Id_Cdf.
  • SegmentIdExtCdf is set to a copy of Default_Segment_Id_Ext_Cdf.
  • SegmentIdPredictedCdf is set to a copy of Default_Segment_Id_Predicted_Cdf.
  • If reduced_tx_part_set is equal to 0, TxPartitionTypeCdf is set to a copy of
    Default_Tx_Partition_Type_Cdf.
  • If reduced_tx_part_set is equal to 1, TxPartitionTypeCdf is set to a copy of
    Default_Tx_Partition_Type_Reduced_Cdf.
  • Tx2or3PartitionTypeCdf is set to a copy of Default_Tx_2or3_Partition_Type_Cdf.
  • TxDoPartitionCdf is set to a copy of Default_Tx_Do_Partition_Cdf.
  • LosslessTxSizeCdf is set to a copy of Default_Lossless_Tx_Size_Cdf.
  • LosslessInterTxTypeCdf is set to a copy of Default_Lossless_Inter_Tx_Type_Cdf.
  • SecTxTypeCdf is set to a copy of Default_Sec_Tx_Type_Cdf.
  • CctxTypeCdf is set to a copy of Default_Cctx_Type_Cdf.
  • MostProbableStxSetCdf is set to a copy of Default_Most_Probable_Stx_Set_Cdf.
  • MostProbableStxSetAdstCdf is set to a copy of Default_Most_Probable_Stx_Set_Adst_Cdf.
  • InterpFilterCdf is set to a copy of Default_Interp_Filter_Cdf.
  • UseLocalWarpCdf is set to a copy of Default_Use_Local_Warp_Cdf.
  • UseExtendWarpCdf is set to a copy of Default_Use_Extend_Warp_Cdf.



AV2 Specification                                                                        Page 329 of 1169
  • SingleModeCdf is set to a copy of Default_Single_Mode_Cdf.
  • UseBawpCdf is set to a copy of Default_Use_Bawp_Cdf.
  • UseBawpChromaCdf is set to a copy of Default_Use_Bawp_Chroma_Cdf.
  • ExplicitBawpCdf is set to a copy of Default_Explicit_Bawp_Cdf.
  • ExplicitBawpScaleCdf is set to a copy of Default_Explicit_Bawp_Scale_Cdf.
  • IsJointCdf is set to a copy of Default_Is_Joint_Cdf.
  • CompoundModeNonJointCdf is set to a copy of Default_Compound_Mode_Non_Joint_Cdf.
  • CompoundModeSameRefsCdf is set to a copy of Default_Compound_Mode_Same_Refs_Cdf.
  • UseOptflowCdf is set to a copy of Default_Use_Optflow_Cdf.
  • TipModeCdf is set to a copy of Default_Tip_Mode_Cdf.
  • UseRefinemvCdf is set to a copy of Default_Use_Refinemv_Cdf.
  • DrlModeCdf is set to a copy of Default_Drl_Mode_Cdf.
  • SkipDrlModeCdf is set to a copy of Default_Skip_Drl_Mode_Cdf.
  • TipDrlModeCdf is set to a copy of Default_Tip_Drl_Mode_Cdf.
  • IsInterCdf is set to a copy of Default_Is_Inter_Cdf.
  • CompModeCdf is set to a copy of Default_Comp_Mode_Cdf.
  • SkipModeCdf is set to a copy of Default_Skip_Mode_Cdf.
  • SkipCdf is set to a copy of Default_Skip_Cdf.
  • CompRef0Cdf is set to a copy of Default_Comp_Ref0_Cdf.
  • CompRef1Cdf is set to a copy of Default_Comp_Ref1_Cdf.
  • SingleRefCdf is set to a copy of Default_Single_Ref_Cdf.
  • UseMostProbablePrecisionCdf is set to a copy of Default_Use_Most_Probable_Precision_Cdf.
  • PbMvPrecisionCdf is set to a copy of Default_Pb_Mv_Precision_Cdf.
  • MvJointAdaptiveCdf is set to a copy of Default_Mv_Joint_Adaptive_Cdf.
  • AmvdIndicesCdf is set to a copy of Default_Amvd_Indices_Cdf.
  • JointShellSetCdf[ i ] is set to a copy of Default_Joint_Shell_Set_Cdf for i = 0..MV_CONTEXTS-1.
  • JointShell0Class0Cdf[ i ] is set to a copy of Default_Joint_Shell0_Class0_Cdf for i =
    0..MV_CONTEXTS-1.
  • JointShell1Class0Cdf[ i ] is set to a copy of Default_Joint_Shell1_Class0_Cdf for i =
    0..MV_CONTEXTS-1.
  • JointShell3Class0Cdf[ i ] is set to a copy of Default_Joint_Shell3_Class0_Cdf for i =
    0..MV_CONTEXTS-1.
  • JointShell4Class0Cdf[ i ] is set to a copy of Default_Joint_Shell4_Class0_Cdf for i =
    0..MV_CONTEXTS-1.
  • JointShell5Class0Cdf[ i ] is set to a copy of Default_Joint_Shell5_Class0_Cdf for i =
    0..MV_CONTEXTS-1.




AV2 Specification                                                                           Page 330 of 1169
  • JointShell6Class0Cdf[ i ] is set to a copy of Default_Joint_Shell6_Class0_Cdf for i =
    0..MV_CONTEXTS-1.
  • JointShell0Class1Cdf[ i ] is set to a copy of Default_Joint_Shell0_Class1_Cdf for i =
    0..MV_CONTEXTS-1.
  • JointShell1Class1Cdf[ i ] is set to a copy of Default_Joint_Shell1_Class1_Cdf for i =
    0..MV_CONTEXTS-1.
  • JointShell3Class1Cdf[ i ] is set to a copy of Default_Joint_Shell3_Class1_Cdf for i =
    0..MV_CONTEXTS-1.
  • JointShell4Class1Cdf[ i ] is set to a copy of Default_Joint_Shell4_Class1_Cdf for i =
    0..MV_CONTEXTS-1.
  • JointShell5Class1Cdf[ i ] is set to a copy of Default_Joint_Shell5_Class1_Cdf for i =
    0..MV_CONTEXTS-1.
  • JointShell6Class1Cdf[ i ] is set to a copy of Default_Joint_Shell6_Class1_Cdf for i =
    0..MV_CONTEXTS-1.
  • JointShellLastTwoClassesCdf[ i ] is set to a copy of Default_Joint_Shell_Last_Two_Classes_Cdf for i =
    0..MV_CONTEXTS-1.
  • ShellOffsetLowClassCdf[ i ] is set to a copy of Default_Shell_Offset_Low_Class_Cdf for i =
    0..MV_CONTEXTS-1.
  • ShellOffsetClass2Cdf[ i ] is set to a copy of Default_Shell_Offset_Class2_Cdf for i =
    0..MV_CONTEXTS-1.
  • ShellOffsetOtherClassCdf[ i ] is set to a copy of Default_Shell_Offset_Other_Class_Cdf for i =
    0..MV_CONTEXTS-1.
  • ColMvGreaterCdf[ i ] is set to a copy of Default_Col_Mv_Greater_Cdf for i = 0..MV_CONTEXTS-1.
  • ColMvIndexCdf[ i ] is set to a copy of Default_Col_Mv_Index_Cdf for i = 0..MV_CONTEXTS-1.
  • JmvdScaleModeCdf is set to a copy of Default_Jmvd_Scale_Mode_Cdf.
  • JmvdAdaptiveScaleModeCdf is set to a copy of Default_Jmvd_Adaptive_Scale_Mode_Cdf.
  • PaletteYModeCdf is set to a copy of Default_Palette_Y_Mode_Cdf.
  • IdentityRowYCdf is set to a copy of Default_Identity_Row_Y_Cdf.
  • PaletteYSizeCdf is set to a copy of Default_Palette_Y_Size_Cdf.
  • PaletteSize2YColorCdf is set to a copy of Default_Palette_Size_2_Y_Color_Cdf.
  • PaletteSize3YColorCdf is set to a copy of Default_Palette_Size_3_Y_Color_Cdf.
  • PaletteSize4YColorCdf is set to a copy of Default_Palette_Size_4_Y_Color_Cdf.
  • PaletteSize5YColorCdf is set to a copy of Default_Palette_Size_5_Y_Color_Cdf.
  • PaletteSize6YColorCdf is set to a copy of Default_Palette_Size_6_Y_Color_Cdf.
  • PaletteSize7YColorCdf is set to a copy of Default_Palette_Size_7_Y_Color_Cdf.
  • PaletteSize8YColorCdf is set to a copy of Default_Palette_Size_8_Y_Color_Cdf.
  • DeltaQCdf is set to a copy of Default_Delta_Q_Cdf.
  • IntraTxTypeLongCdf is set to a copy of Default_Intra_Tx_Type_Long_Cdf.
  • InterTxTypeLongCdf is set to a copy of Default_Inter_Tx_Type_Long_Cdf.


AV2 Specification                                                                            Page 331 of 1169
  • IsLongSideDctCdf is set to a copy of Default_Is_Long_Side_Dct_Cdf.
  • IntraTxTypeSet1Cdf is set to a copy of Default_Intra_Tx_Type_Set1_Cdf.
  • IntraTxTypeSet2Cdf is set to a copy of Default_Intra_Tx_Type_Set2_Cdf.
  • InterTxTypeSet1Cdf is set to a copy of Default_Inter_Tx_Type_Set1_Cdf.
  • InterTxTypeSet2Cdf is set to a copy of Default_Inter_Tx_Type_Set2_Cdf.
  • InterTxTypeSet3Cdf is set to a copy of Default_Inter_Tx_Type_Set3_Cdf.
  • InterTxTypeSet4Cdf is set to a copy of Default_Inter_Tx_Type_Set4_Cdf.
  • InterTxTypeIndexSet1Cdf is set to a copy of Default_Inter_Tx_Type_Index_Set1_Cdf.
  • InterTxTypeIndexSet2Cdf is set to a copy of Default_Inter_Tx_Type_Index_Set2_Cdf.
  • InterTxTypeOffsetSet1Cdf is set to a copy of Default_Inter_Tx_Type_Offset_Set1_Cdf.
  • InterTxTypeOffsetSet2Cdf is set to a copy of Default_Inter_Tx_Type_Offset_Set2_Cdf.
  • InterIntraCdf is set to a copy of Default_Inter_Intra_Cdf.
  • WarpInterIntraCdf is set to a copy of Default_Warp_Inter_Intra_Cdf.
  • CflSignCdf is set to a copy of Default_Cfl_Sign_Cdf.
  • WedgeInterIntraCdf is set to a copy of Default_Wedge_Inter_Intra_Cdf.
  • CompGroupIdxCdf is set to a copy of Default_Comp_Group_Idx_Cdf.
  • CompoundTypeCdf is set to a copy of Default_Compound_Type_Cdf.
  • InterIntraModeCdf is set to a copy of Default_Inter_Intra_Mode_Cdf.
  • WedgeQuadCdf is set to a copy of Default_Wedge_Quad_Cdf.
  • WedgeAngleCdf is set to a copy of Default_Wedge_Angle_Cdf.
  • WedgeDist1Cdf is set to a copy of Default_Wedge_Dist1_Cdf.
  • WedgeDist2Cdf is set to a copy of Default_Wedge_Dist2_Cdf.
  • CflAlphaCdf is set to a copy of Default_Cfl_Alpha_Cdf.
  • CflIndexCdf is set to a copy of Default_Cfl_Index_Cdf.
  • CflMhDirCdf is set to a copy of Default_Cfl_Mh_Dir_Cdf.
  • CflMhccpCdf is set to a copy of Default_Cfl_Mhccp_Cdf.
  • UseAmvdCdf is set to a copy of Default_Use_Amvd_Cdf.
  • CcsoBlkCdf is set to a copy of Default_Ccso_Blk_Cdf.
  • UseWienerNsCdf is set to a copy of Default_Use_Wiener_Ns_Cdf.
  • WienerNsLengthCdf is set to a copy of Default_Wiener_Ns_Length_Cdf.
  • WienerNsUvSymCdf is set to a copy of Default_Wiener_Ns_Uv_Sym_Cdf.
  • WienerNsBaseCdf is set to a copy of Default_Wiener_Ns_Base_Cdf.
  • UsePcWienerCdf is set to a copy of Default_Use_Pc_Wiener_Cdf.
  • FlexRestorationTypeCdf is set to a copy of Default_Flex_Restoration_Type_Cdf.




AV2 Specification                                                                         Page 332 of 1169
init_coeff_cdfs( ) is a function call that initializes the CDF tables used in the coeffs( ) syntax structure.
When this function is invoked, the following steps apply:

  • The variable idx is derived as follows:

       ◦ If base_q_idx is less than or equal to 90, idx is set equal to 0.
       ◦ Otherwise, if base_q_idx is less than or equal to 140, idx is set equal to 1.
       ◦ Otherwise, if base_q_idx is less than or equal to 190, idx is set equal to 2.
       ◦ Otherwise, idx is set equal to 3.
  • The cumulative distribution function arrays are reset to default values as follows:

       ◦ TxbSkipCdf is set to a copy of Default_Txb_Skip_Cdf[ idx ].
       ◦ EobPt16Cdf is set to a copy of Default_Eob_Pt_16_Cdf[ idx ].
       ◦ EobPt32Cdf is set to a copy of Default_Eob_Pt_32_Cdf[ idx ].
       ◦ EobPt64Cdf is set to a copy of Default_Eob_Pt_64_Cdf[ idx ].
       ◦ EobPt128Cdf is set to a copy of Default_Eob_Pt_128_Cdf[ idx ].
       ◦ EobPt256Cdf is set to a copy of Default_Eob_Pt_256_Cdf[ idx ].
       ◦ EobPt512Cdf is set to a copy of Default_Eob_Pt_512_Cdf[ idx ].
       ◦ EobPt1024Cdf is set to a copy of Default_Eob_Pt_1024_Cdf[ idx ].
       ◦ EobExtraCdf is set to a copy of Default_Eob_Extra_Cdf[ idx ].
       ◦ DcSignCdf is set to a copy of Default_Dc_Sign_Cdf[ idx ].
       ◦ VTxbSkipCdf is set to a copy of Default_V_Txb_Skip_Cdf[ idx ].
       ◦ CoeffBaseEobCdf is set to a copy of Default_Coeff_Base_Eob_Cdf[ idx ].
       ◦ CoeffBaseLfEobCdf is set to a copy of Default_Coeff_Base_Lf_Eob_Cdf[ idx ].
       ◦ CoeffBaseCdf is set to a copy of Default_Coeff_Base_Cdf[ idx ].
       ◦ CoeffBaseLfCdf is set to a copy of Default_Coeff_Base_Lf_Cdf[ idx ].
       ◦ CoeffBasePhCdf is set to a copy of Default_Coeff_Base_Ph_Cdf[ idx ].
       ◦ CoeffBrCdf is set to a copy of Default_Coeff_Br_Cdf[ idx ].
       ◦ CoeffBrLfCdf is set to a copy of Default_Coeff_Br_Lf_Cdf[ idx ].
       ◦ CoeffBrUvCdf is set to a copy of Default_Coeff_Br_Uv_Cdf[ idx ].
       ◦ CoeffBaseLfUvCdf is set to a copy of Default_Coeff_Base_Lf_Uv_Cdf[ idx ].
       ◦ CoeffBaseLfEobUvCdf is set to a copy of Default_Coeff_Base_Lf_Eob_Uv_Cdf[ idx ].
       ◦ CoeffBaseUvCdf is set to a copy of Default_Coeff_Base_Uv_Cdf[ idx ].
       ◦ CoeffBaseEobUvCdf is set to a copy of Default_Coeff_Base_Eob_Uv_Cdf[ idx ].
       ◦ CoeffBaseBobCdf is set to a copy of Default_Coeff_Base_Bob_Cdf[ idx ].
       ◦ CoeffBrIdtxCdf is set to a copy of Default_Coeff_Br_Idtx_Cdf[ idx ].
       ◦ CoeffBaseIdtxCdf is set to a copy of Default_Coeff_Base_Idtx_Cdf[ idx ].
       ◦ IdtxSignCdf is set to a copy of Default_Idtx_Sign_Cdf[ idx ].



AV2 Specification                                                                                Page 333 of 1169
    load_cdfs( ctx ) is a function call that indicates that the CDF tables are loaded from frame context
    number ctx in the range 0 to (NUM_REF_FRAMES - 1). When this function is invoked, a copy of each CDF
    array mentioned in the semantics for init_coeff_cdfs and init_non_coeff_cdfs is loaded from an area of
    memory indexed by ctx. (The memory contents of these frame contexts have been initialized by previous
    calls to save_cdfs).

    blend_cdfs( ctx ) is a function call that indicates that the CDF tables are blended with the contents of
    frame context number ctx in the range 0 to (NUM_REF_FRAMES - 1). When this function is invoked, a
    blend is made of the CDF values for each of the CDF arrays mentioned in the semantics for init_coeff_cdfs
    and init_non_coeff_cdfs.

    The blend works for each CDF of the cdf array in turn by calling the blend_cdf function with a reference to
    the CDF, a reference to the previously saved CDF for context ctx, and the length of each CDF as inputs.

    The blend_cdf function (which updates the CDF with a small amount of the previously saved CDF) is
    specified as:

     blend_cdf( cdf, savedCdf, sz ) {
         for( i = 0; i < sz - 2; i++ ) {
             cdf[ i ] = (1 << 15) -
                        ( ( (1 << 15) - savedCdf[ i ] +
                            7 * ((1 << 15) - cdf[ i ]) + 4) >> 3 )
         }
         i2 = sz - 1
         cdf[ i2 ] = (savedCdf[ i2 ] + 7 * cdf[ i2 ] + 4) >> 3
     }


    load_previous( ) is a function call that indicates that information from a previous frame (denoted by
    prevFrame) may be loaded for use in decoding the current frame. When this function is invoked the
    following ordered steps apply:

     1. The variable prevFrame is set equal to ref_frame_idx[ DerivedPrimaryRefFrame ].
     2. PrevGmParams is set equal to a copy of SavedGmParams[ prevFrame ].

    load_previous_segment_ids( ) is a function call that indicates that a segmentation map from a previous
    frame (denoted by prevFrame) may be loaded for use in decoding the current frame. When this function
    is invoked the segmentation map contained in PrevSegmentIds is set as follows:

     1. The variable prevFrame is set equal to ref_frame_idx[ DerivedPrimaryRefFrame ].
     2. If segmentation_enabled is equal to 1, RefMiCols[ prevFrame ] is equal to MiCols, and
        RefMiRows[ prevFrame ] is equal to MiRows, PrevSegmentIds[ row ][ col ] is set equal to
        SavedSegmentIds[ prevFrame ][ row ][ col ] for row = 0..MiRows-1, for col = 0..MiCols-1.

        Otherwise, PrevSegmentIds[ row ][ col ] is set equal to 0 for row = 0..MiRows-1, for col =
        0..MiCols-1.

```

<a id="s-6-17-3"></a>

#### § 6.17.3 Frame configuration structures

```text
§   6.17.3. Frame configuration structures

```

<a id="s-6-17-3-1"></a>

##### § 6.17.3.1 Frame optical flow refine type semantics

```text
§   6.17.3.1. Frame optical flow refine type semantics

    opfl_refine_type specifies how optical flow refinement is signaled with the same semantics as
    enable_opfl_refine.




    AV2 Specification                                                                            Page 334 of 1169
      NOTE:       It is not possible for opfl_refine_type to be set to REFINE_AUTO.


    opfl_refine_all is used to set the value of opfl_refine_type when it does not fit in a single bit.

```

<a id="s-6-17-3-2"></a>

##### § 6.17.3.2 Screen content params semantics

```text
§   6.17.3.2. Screen content params semantics

    allow_screen_content_tools equal to 1 indicates that intra blocks may use palette encoding;
    allow_screen_content_tools equal to 0 indicates that palette encoding is never used.

    force_integer_mv equal to 1 specifies that motion vectors will always be integers. force_integer_mv
    equal to 0 specifies that motion vectors can contain fractional bits.

```

<a id="s-6-17-3-3"></a>

##### § 6.17.3.3 Intra block copy params semantics

```text
§   6.17.3.3. Intra block copy params semantics

    allow_intrabc equal to 1 indicates that intra block copy can be used in this frame. allow_intrabc equal to
    0 indicates that intra block copy is not allowed in this frame.

    allow_local_intrabc equal to 1 indicates that intra block copy can use a block within the local area in
    this frame as reference. The local area consists of decoded samples, prior to any loop filtering operations,
    from the four most recently decoded 64x64 regions.

    allow_global_intrabc equal to 1 indicates that intra block copy can use a block within the global area in
    this frame as reference. The global area consists of decoded samples, prior to any loop filtering
    operations, from the current and previous superblock rows, excluding the local area.


      NOTE: The eligibility of a reference block in the local or global area for intra block copy is verified
      using is_mv_valid.


    change_bvp_drl equal to 1 indicates that max_bvp_drl_bits_minus_1 is changed from the value in the
    sequence header.

    max_bvp_drl_bits_minus_1 plus 1 specifies the maximum number of times the intrabc_drl_mode syntax
    element is read within read_intrabc_info for blocks using intra block copy.

```

<a id="s-6-17-4"></a>

#### § 6.17.4 Frame size structures

```text
§   6.17.4. Frame size structures

```

<a id="s-6-17-4-1"></a>

##### § 6.17.4.1 Frame size semantics

```text
§   6.17.4.1. Frame size semantics

    frame_width_minus_1 plus one is the width of the frame in luma samples.

    frame_height_minus_1 plus one is the height of the frame in luma samples.

    It is a requirement of bitstream conformance that frame_width_minus_1 is less than or equal to
    max_frame_width_minus_1.

    It is a requirement of bitstream conformance that frame_height_minus_1 is less than or equal to
    max_frame_height_minus_1.

    If FrameIsIntra is equal to 0 (indicating that this frame may use inter prediction), the requirements
    described in the frame size with refs semantics of [section 6.8.6] must also be satisfied.




    AV2 Specification                                                                                Page 335 of 1169
```

<a id="s-6-17-4-2"></a>

##### § 6.17.4.2 Frame size with bridge semantics

```text
§   6.17.4.2. Frame size with bridge semantics

    bridge_frame_width_minus_1 plus 1 specifies the target width of the Bridge frame.

    bridge_frame_height_minus_1 plus 1 specifies the target height of the Bridge frame.


      NOTE: Bridge frames are used to make frames smaller. If the reference frame is already smaller
      than the target size then the frame dimensions are unchanged.

```

<a id="s-6-17-4-3"></a>

##### § 6.17.4.3 Frame size with refs semantics

```text
§   6.17.4.3. Frame size with refs semantics

    For inter frames, the frame size is either set equal to the size of a reference frame, or can be sent
    explicitly.

    found_ref equal to 1 indicates that the frame dimensions can be inferred from reference frame i where i
    is the loop counter in the syntax parsing process for frame_size_with_refs. found_ref equal to 0 indicates
    that the frame dimensions are not inferred from reference frame i.

    It is a requirement of bitstream conformance that RefOrderHint[ ref_frame_idx[ i ] ] is not equal to
    RESTRICTED_OH.

    Once the FrameWidth and FrameHeight have been computed for an inter frame, it is a requirement of
    bitstream conformance that for all values of i in the range 0..(REFS_PER_FRAME - 1), all the following
    conditions are true:

      • 2 * FrameWidth >= RefFrameWidth[ ref_frame_idx[ i ] ]
      • 2 * FrameHeight >= RefFrameHeight[ ref_frame_idx[ i ] ]
      • FrameWidth <= 16 * RefFrameWidth[ ref_frame_idx[ i ] ]
      • FrameHeight <= 16 * RefFrameHeight[ ref_frame_idx[ i ] ]


      NOTE: This is a requirement even if all the blocks in an inter frame are coded using intra
      prediction.

```

<a id="s-6-17-4-4"></a>

##### § 6.17.4.4 Compute image size function semantics

```text
§   6.17.4.4. Compute image size function semantics

    MiCols is the number of 4x4 block columns in the frame.

    MiRows is the number of 4x4 block rows in the frame.

    CropLeft, CropTop, CropWidth, CropHeight express the size of the cropped window to output.

    It is a requirement of bitstream conformance that:

      • CropWidth is greater than 0.
      • CropHeight is greater than 0.

    If Monochrome is equal to 0, it is a requirement of bitstream conformance that:

      • CropLeft is equal to ((CropLeft >> SubsamplingX) << SubsamplingX).
      • CropTop is equal to ((CropTop >> SubsamplingY) << SubsamplingY).



    AV2 Specification                                                                              Page 336 of 1169
```

<a id="s-6-17-5"></a>

#### § 6.17.5 Filtering structures

```text
§   6.17.5. Filtering structures

```

<a id="s-6-17-5-1"></a>

##### § 6.17.5.1 Interpolation filter semantics

```text
§   6.17.5.1. Interpolation filter semantics

    is_filter_switchable equal to 1 indicates that the filter selection is signaled at the block level;
    is_filter_switchable equal to 0 indicates that the filter selection is signaled at the frame level.

    interpolation_filter specifies the filter selection used for performing inter prediction:

                    interpolation_filter                             Name of interpolation_filter

                             0                                                EIGHTTAP

                             1                                           EIGHTTAP_SMOOTH

                             2                                            EIGHTTAP_SHARP

                             3                                                BILINEAR

                             4                                              SWITCHABLE


```

<a id="s-6-17-5-2"></a>

##### § 6.17.5.2 Deblocking filter params semantics

```text
§   6.17.5.2. Deblocking filter params semantics

    apply_deblocking_filter is an array containing flags that specify if the deblocking filter is applied for a
    particular plane and direction. Different values of apply_deblocking_filter from the array are used
    depending on the image plane being filtered, and the edge direction (vertical or horizontal) being filtered.

    df_delta_q_present[ i ] equal to 1 means that df_delta_q[ i ] syntax element for the deblocking filter is
    present. df_delta_q_present[ i ] equal to 0 means that the df_delta_q[ i ] syntax element is not present.

    df_delta_q[ i ] is used to adjust the deblocking filter strength by adding an offset to the quantizer-based
    index of the threshold tables used by the deblocking filter. The offsets can be set separately for horizontal
    and vertical boundaries of plane 0 (luma) and for boundaries of planes 1 and 2 (chroma).

    The deblocking filter process is described in § 7.17 Deblocking filter process.


      NOTE:       The semantics of allow_df_sub_pu are provided in § 6.17.2 Frame header info semantics.

```

<a id="s-6-17-6"></a>

#### § 6.17.6 Quantization structures

```text
§   6.17.6. Quantization structures

```

<a id="s-6-17-6-1"></a>

##### § 6.17.6.1 Quantization params semantics

```text
§   6.17.6.1. Quantization params semantics

    The residual is specified via decoded coefficients which are adjusted by one of four quantization
    parameters before the inverse transform is applied. The choice depends on the plane (Y or UV) and
    coefficient position (DC/AC coefficient). The dequantization process is specified in § 7.14 Reconstruction
    and dequantization.

    base_q_idx indicates the base frame qindex. This is used for Y AC coefficients and as the base value for
    the other quantizers.

    DeltaQYDc indicates the Y DC quantizer relative to base_q_idx.

    diff_uv_delta equal to 1 indicates that the U and V delta quantizer values are coded separately.
    diff_uv_delta equal to 0 indicates that the U and V delta quantizer values share a common value.

    DeltaQUDc indicates the U DC quantizer relative to base_q_idx.



    AV2 Specification                                                                               Page 337 of 1169
    DeltaQUAc indicates the U AC quantizer relative to base_q_idx.

    DeltaQVDc indicates the V DC quantizer relative to base_q_idx.

    DeltaQVAc indicates the V AC quantizer relative to base_q_idx.

```

<a id="s-6-17-6-2"></a>

##### § 6.17.6.2 Setup QM params semantics

```text
§   6.17.6.2. Setup QM params semantics

    using_qmatrix specifies that the quantizer matrix will be used to compute quantizers.

    pic_qm_num_minus_1 plus 1 specifies the number of qm_y syntax elements present.

    qm_y specifies the level in the quantizer matrix that is to be used for luma plane decoding.

    If qm_y[ i ] is less than NUM_CUSTOM_QMS, it is a requirement of bitstream conformance that
    QmNumPlanes[ qm_y[ i ] ] is equal to NumPlanes.

    If qm_y[ i ] is less than NUM_CUSTOM_QMS and QmMLayerId[ qm_y[ i ] ] is greater than or equal to 0, it
    is a requirement of bitstream conformance that MLayerDependencyMap[ obu_mlayer_id ]
    [ QmMLayerId[ qm_y[ i ] ] ] is equal to 1.

    If qm_y[ i ] is less than NUM_CUSTOM_QMS and QmMLayerId[ qm_y[ i ] ] is greater than or equal to 0, it
    is a requirement of bitstream conformance that TLayerDependencyMap[ obu_mlayer_id ][ obu_tlayer_id ]
    [ QmTLayerId[ qm_y[ i ] ] ] is equal to 1.

    qm_uv_same_as_y specifies that qm_u and qm_v match qm_y.

    qm_u specifies the level in the quantizer matrix that is to be used for chroma U plane decoding.

    If qm_u[ i ] is less than NUM_CUSTOM_QMS, it is a requirement of bitstream conformance that
    QmNumPlanes[ qm_u[ i ] ] is equal to NumPlanes.

    If qm_u[ i ] is less than NUM_CUSTOM_QMS and QmMLayerId[ qm_u[ i ] ] is greater than or equal to 0,
    it is a requirement of bitstream conformance that MLayerDependencyMap[ obu_mlayer_id ]
    [ QmMLayerId[ qm_u[ i ] ] ] is equal to 1.

    If qm_u[ i ] is less than NUM_CUSTOM_QMS and QmMLayerId[ qm_u[ i ] ] is greater than or equal to 0,
    it is a requirement of bitstream conformance that TLayerDependencyMap[obu_mlayer_id][ obu_tlayer_id ]
    [ QmTLayerId[ qm_u[ i ] ] ] is equal to 1.

    qm_v specifies the level in the quantizer matrix that is to be used for chroma V plane decoding.

    If qm_v[ i ] is less than NUM_CUSTOM_QMS, it is a requirement of bitstream conformance that
    QmNumPlanes[ qm_v[ i ] ] is equal to NumPlanes.

    If qm_v[ i ] is less than NUM_CUSTOM_QMS and QmMLayerId[ qm_v[ i ] ] is greater than or equal to 0, it
    is a requirement of bitstream conformance that MLayerDependencyMap[ obu_mlayer_id ]
    [ QmMLayerId[ qm_v[ i ] ] ] is equal to 1.

    If qm_v[ i ] is less than NUM_CUSTOM_QMS and QmMLayerId[ qm_v[ i ] ] is greater than or equal to 0, it
    is a requirement of bitstream conformance that TLayerDependencyMap[obu_mlayer_id][ obu_tlayer_id ]
    [ QmTLayerId[ qm_v[ i ] ] ] is equal to 1.




    AV2 Specification                                                                              Page 338 of 1169
```

<a id="s-6-17-6-3"></a>

##### § 6.17.6.3 Delta quantizer semantics

```text
§   6.17.6.3. Delta quantizer semantics

    delta_coded specifies that the delta_q syntax element is present.

    delta_q specifies an offset (relative to base_q_idx) for a particular quantization parameter.

```

<a id="s-6-17-7"></a>

#### § 6.17.7 Segmentation and tiling structures

```text
§   6.17.7. Segmentation and tiling structures

```

<a id="s-6-17-7-1"></a>

##### § 6.17.7.1 Segmentation params semantics

```text
§   6.17.7.1. Segmentation params semantics

    AV2 provides a means of segmenting the image and then applying various adjustments at the segment
    level.

    Up to 16 segments may be specified for any given frame. For each of these segments it is possible to
    specify:

     1. A quantizer (absolute value or delta).
     2. A block skip mode that implies both the use of a (0,0) motion vector and that no residual will be
        coded.
     3. A forced use of global motion vector

    Each of these data values for each segment may be individually updated at the frame level. Where a value
    is not updated in a given frame, the value from a previous frame, indicated by DerivedPrimaryRefFrame,
    persists. The exceptions to this are key frames, intra only frames or other frames where independence
    from past frame values is required (for example to enable error resilience). In such cases all values are
    reset as described in the semantics for setup_past_independence.

    reuse_seg_info equal to 1 indicates that the segment data and enables are reused (from the sequence
    header or multi-frame header). reuse_seg_info equal to 0 indicates that the segment data and enables are
    present in the current syntax structure.

    SegIdPreSkip equal to 1 indicates that the segment id will be read before the skip_flag syntax element.
    SegIdPreSkip equal to 0 indicates that the skip_flag syntax element will be read first.

    LastActiveSegId indicates the highest numbered segment id that has some enabled feature. This is used
    when decoding the segment id to only decode choices corresponding to used segments.

    segmentation_enabled equal to 1 indicates that this frame makes use of the segmentation tool;
    segmentation_enabled equal to 0 indicates that the frame does not use segmentation.

    segmentation_update_map equal to 1 indicates that the segmentation map is updated during the
    decoding of this frame. segmentation_update_map equal to 0 means that the segmentation map from a
    previous frame, indicated by DerivedPrimaryRefFrame, is used.

    segmentation_temporal_update equal to 1 indicates that the updates to the segmentation map are
    coded relative to the existing segmentation map. segmentation_temporal_update equal to 0 indicates that
    the new segmentation map is coded without reference to the existing segmentation map.

```

<a id="s-6-17-7-2"></a>

##### § 6.17.7.2 Tile info semantics

```text
§   6.17.7.2. Tile info semantics

    reuse_tile_info equal to 1 specifies that the tile parameters are reused. reuse_tile_info equal to 0
    specifies that the tile parameters are present.



    AV2 Specification                                                                               Page 339 of 1169
    TileColsLog2 specifies the base 2 logarithm of the desired number of tiles across the frame.

    TileCols specifies the number of tiles across the frame. It is a requirement of bitstream conformance that
    TileCols is less than or equal to MAX_TILE_COLS.

    TileRowsLog2 specifies the base 2 logarithm of the desired number of tiles down the frame.


      NOTE: For small frame sizes the actual number of tiles in the frame may be smaller than the
      desired number because the tile size is rounded up to a multiple of the maximum superblock size.


    TileRows specifies the number of tiles down the frame. It is a requirement of bitstream conformance that
    TileRows is less than or equal to MAX_TILE_ROWS.

    MiColStarts is an array specifying the start column (in units of 4x4 luma samples) for each tile across
    the image.

    MiRowStarts is an array specifying the start row (in units of 4x4 luma samples) for each tile down the
    image.

    context_update_tile_id specifies which tile to use for the CDF update. It is a requirement of bitstream
    conformance that context_update_tile_id is less than TileCols * TileRows.

    tile_size_bytes_minus_1 is used to compute TileSizeBytes.

    TileSizeBytes specifies the number of bytes needed to code each tile size.

```

<a id="s-6-17-7-3"></a>

##### § 6.17.7.3 Tile params semantics

```text
§   6.17.7.3. Tile params semantics

    uniform_tile_spacing_flag equal to 1 means that the tiles are roughly uniformly spaced across the
    frame. (All tiles are roughly the same size except for the ones at the right and bottom edge which can be
    smaller.) uniform_tile_spacing_flag equal to 0 means that the tile sizes are coded.

    increment_tile_cols_log2 is used to compute tileColsLog2.

    increment_tile_rows_log2 is used to compute tileRowsLog2.

    If uniform_tile_spacing_flag is equal to 0, it is a requirement of bitstream conformance that startSb is
    equal to sbCols when the loop writing sbColStarts exits.

    If uniform_tile_spacing_flag is equal to 0, it is a requirement of bitstream conformance that startSb is
    equal to sbRows when the loop writing sbRowStarts exits.


      NOTE: The requirements on startSb ensure that the sizes of each tile add up to the full size of the
      frame when measured in superblocks.


    width_in_sbs_minus_1 specifies the width of a tile minus 1 in units of superblocks.

    height_in_sbs_minus_1 specifies the height of a tile minus 1 in units of superblocks.

    maxTileHeightSb specifies the maximum height (in units of superblocks) that can be used for a tile (to
    avoid making tiles with too much area).




    AV2 Specification                                                                             Page 340 of 1169
```

<a id="s-6-17-7-4"></a>

##### § 6.17.7.4 Quantizer index delta parameters semantics

```text
§   6.17.7.4. Quantizer index delta parameters semantics

    delta_q_present equal to 1 specifies that quantizer index delta values are present in the frame.
    delta_q_present equal to 0 specifies that quantizer index delta values are not present.

    delta_q_res specifies the left shift to be applied to decoded quantizer index delta values.

```

<a id="s-6-17-7-5"></a>

##### § 6.17.7.5 GDF params semantics

```text
§   6.17.7.5. GDF params semantics

    gdf_frame_enable equal to 1 specifies that Guided Detail Filter (GDF) filtering is enabled in the frame.
    gdf_frame_enable equal to 0 specifies that GDF filtering is disabled for this frame.

    gdf_per_block equal to 1 specifies that a block level enable flag is present for Guided Detail Filter (GDF)
    to control GDF on a per-block basis. gdf_per_block equal to 0 specifies that no block level enable flag is
    present and GDF is applied uniformly across the frame.

    gdf_pic_qc_idx specifies an adjustment to the quantizer used in GDF filtering.

    gdf_pic_scale_idx specifies a scaling for the predicted adjustment used in GDF filtering.

```

<a id="s-6-17-7-6"></a>

##### § 6.17.7.6 CDEF params semantics

```text
§   6.17.7.6. CDEF params semantics

    cdef_frame_enable equal to 1 specifies that Constrained Directional Enhancement Filter (CDEF)
    filtering is enabled in the frame. cdef_frame_enable equal to 0 specifies that CDEF filtering is disabled for
    this frame.

    cdef_damping_minus_3 controls the amount of damping in the deringing filter.

    cdef_strengths_minus_1 plus one specifies the number of strengths settings used for CDEF.

    cdef_on_skip_txfm_frame_enable equal to 1 specifies that CDEF filtering is enabled on skipped
    transform blocks. cdef_on_skip_txfm_frame_enable equal to 0 specifies that CDEF filtering is disabled for
    skipped transform blocks.

    cdef_y_pri_zero specifies that cdef_y_pri_strength is equal to 0.

    cdef_uv_pri_zero specifies that cdef_uv_pri_strength is equal to 0.

    cdef_y_pri_strength and cdef_uv_pri_strength specify the strength of the primary filter.

    cdef_y_sec_strength and cdef_uv_sec_strength specify the strength of the secondary filter.

```

<a id="s-6-17-7-7"></a>

##### § 6.17.7.7 Loop restoration params semantics

```text
§   6.17.7.7. Loop restoration params semantics

    tool_index is used to compute FrameRestorationType by choosing one of the enabled tools.

    FrameRestorationType specifies the type of restoration used for each plane as follows:

                   FrameRestorationType                            Name of FrameRestorationType

                            0                                             RESTORE_NONE

                            1                                           RESTORE_PC_WIENER

                            2                                        RESTORE_WIENER_NONSEP

                            3                                           RESTORE_SWITCHABLE




    AV2 Specification                                                                             Page 341 of 1169
UsesLr indicates if any plane uses loop restoration.

frame_filters_on specifies that the Wiener filters are specified at the frame level (instead of being
specified in each loop restoration unit).

temporal_pred_flag specifies that the frame level Wiener filters are copied from a previous reference
frame.

rst_ref_pic_idx specifies which reference to use for the frame level Wiener filters.

If temporal_pred_flag[ plane ] is equal to 1, it is a requirement of bitstream conformance that
rst_ref_pic_idx is less than numRefFrames.

If temporal_pred_flag[ plane ] is equal to 1, it is a requirement of bitstream conformance that
RefFrameFiltersOn[ refIdx ][ refPlane ] is equal to 1.

num_filter_classes_idx specifies an index into Decode_Num_Filter_Classes that gives the number of
classes used in the frame level pixel classified Wiener filter.

lr_luma_use_half_size specifies that luma uses a restoration size of half the maximum size.

lr_luma_use_max_size specifies that luma uses a restoration size of the maximum size.

lr_luma_use_quarter_size specifies that luma uses a restoration size of quarter the maximum size.

lr_chroma_use_half_size specifies that chroma uses a restoration size of half the maximum size.

lr_chroma_use_max_size specifies that chroma uses a restoration size of the maximum size.

lr_chroma_use_quarter_size specifies that chroma uses a restoration size of quarter the maximum size.

LoopRestorationSize[plane] specifies the size of loop restoration units in units of samples in the
current plane.

If usesChromaLr is equal to 1, it is a requirement of bitstream conformance that 64 >> SubsamplingY is less
than or equal to LoopRestorationSize[ 1 ].


  NOTE:       This ensures that restoration units are not smaller than the restoration stripe height.


It is a requirement of bitstream conformance that check_ru_size() is equal to 1, where the function
check_ru_size is defined as:

 check_ru_size() {
   maxPlaneRuSize = Max( LoopRestorationSize[0],
       LoopRestorationSize[1] << Max(SubsamplingX, SubsamplingY) )
   for ( i = 0; i < TileCols - 1; i++ ) {
       tileWidth = (MiColStarts[ i + 1 ] - MiColStarts[ i ]) * MI_SIZE
       if ( tileWidth % maxPlaneRuSize != 0) return 0
   }
   for ( i = 0; i < TileRows - 1; i++ ) {
       tileHeight = (MiRowStarts[ i + 1 ] - MiRowStarts[ i ]) * MI_SIZE
       if( tileHeight % maxPlaneRuSize != 0) return 0
   }
   return 1
 }




AV2 Specification                                                                               Page 342 of 1169
      NOTE:       This check ensures that restoration units do not cross internal tile boundaries.

```

<a id="s-6-17-7-8"></a>

##### § 6.17.7.8 CCSO params semantics

```text
§   6.17.7.8. CCSO params semantics

    ccso_frame_flag equal to 1 specifies that CCSO can be used on this frame. ccso_frame_flag equal to 0
    specifies that CCSO is not enabled for this frame.

    ccso_planes[plane] equal to 1 specifies that Cross Component Sample Offset (CCSO) filtering is enabled
    for a particular plane. ccso_planes[plane] equal to 0 specifies that CCSO filtering is disabled for that
    plane.

    reuse_ccso equal to 1 specifies that the Cross Component Sample Offset (CCSO) parameters are reused
    from a previous decoded frame. reuse_ccso equal to 0 specifies that CCSO parameters are signaled in the
    current frame and not reused from a previous frame.

    sb_reuse_ccso equal to 1 specifies that the Cross Component Sample Offset (CCSO) block level enable
    flags are reused from a previous decoded frame. sb_reuse_ccso equal to 0 specifies that CCSO block level
    enable flags are signaled in the current frame and not reused.

    ccso_ref_idx specifies which reference contains the parameters to reuse.

    SavedCcsoPlanes[i][plane] is defined to be the value of ccso_planes[plane] when
    save_ccso_params(i,plane) was last called.

    SavedCcsoLumaSizeLog2[i][plane] is defined to be the value of CcsoLumaSizeLog2 when
    save_ccso_params(i,plane) was last called.

    When ccso_ref_idx is present in the bitstream the following requirements apply:

      • It is a requirement of bitstream conformance that ccso_ref_idx[plane] is less than NumTotalRefs.
      • It is a requirement of bitstream conformance that SavedCcsoPlanes[ idx ][ plane ] is equal to 1.
      • It is a requirement of bitstream conformance that RefOrderHint[ idx ] is not equal to
        RESTRICTED_OH.

    When ccso_ref_idx is present in the bitstream and sb_reuse_ccso[plane] is equal to 1, the following
    requirements apply:

      • It is a requirement of bitstream conformance that RefMiRows[ idx ] is equal to MiRows.
      • It is a requirement of bitstream conformance that RefMiCols[ idx ] is equal to MiCols.
      • It is a requirement of bitstream conformance that SavedCcsoLumaSizeLog2[ idx ] is equal to
        CcsoLumaSizeLog2.
      • It is a requirement of bitstream conformance that CcsoLumaSizeLog2 is equal to
        CCSO_LUMA_SIZE_LOG2.

    load_ccso_params is a function call defined in § 7.23 Reference frame update process.

    ccso_bo_only specifies that a smaller set of CCSO parameters is present.

    ccso_quant_idx and ccso_scale_idx specify the quantization index and scaling for CCSO filtering.




    AV2 Specification                                                                                Page 343 of 1169
    ccso_ext_filter specifies the CCSO filter type.

    It is a requirement of bitstream conformance that ccso_ext_filter is not equal to 7.

    ccso_max_band_log2 specifies the base 2 logarithm of the maximum number of bands for CCSO
    filtering.

    It is a requirement of bitstream conformance that 1 << ccso_max_band_log2 is less than or equal to
    CCSO_BAND_NUM.

    ccso_edge_clf is used to reduce the number of classes used within CCSO filtering.

    ccso_offset_idx is used to compute the sample offset by providing an index into the Ccso_Offset table.

```

<a id="s-6-17-8"></a>

#### § 6.17.8 Transform and coding mode structures

```text
§   6.17.8. Transform and coding mode structures

```

<a id="s-6-17-8-1"></a>

##### § 6.17.8.1 TX mode semantics

```text
§   6.17.8.1. TX mode semantics

    tx_mode_select is used to compute TxMode.

    TxMode specifies how the transform size is determined:

                    TxMode                                          Name of TxMode

                        0                                              ONLY_4X4

                        1                                          TX_MODE_LARGEST

                        2                                          TX_MODE_SELECT


    For tx_mode equal to TX_MODE_LARGEST, the inverse transform will use the largest transform size that
    fits inside the block.

    For tx_mode equal to ONLY_4X4, the inverse transform will use only 4x4 transforms.

    For tx_mode equal to TX_MODE_SELECT, the choice of transform size is specified explicitly for each
    block.

```

<a id="s-6-17-8-2"></a>

##### § 6.17.8.2 Skip mode params semantics

```text
§   6.17.8.2. Skip mode params semantics

    SkipModeFrame[ list ] specifies the initial frames to use for compound prediction when skip_mode is
    equal to 1. (These frames are used for motion vector prediction, but may change when an entry is
    selected from the motion vector stack.)

    skip_mode_present equal to 1 specifies that the syntax element skip_mode will be present.
    skip_mode_present equal to 0 specifies that skip_mode will not be used for this frame.

```

<a id="s-6-17-8-3"></a>

##### § 6.17.8.3 Frame reference mode semantics

```text
§   6.17.8.3. Frame reference mode semantics

    reference_select equal to 1 specifies that the mode info for inter blocks contains the syntax element
    comp_mode that indicates whether to use single or compound reference prediction. reference_select
    equal to 0 specifies that all inter blocks will use single prediction.




    AV2 Specification                                                                             Page 344 of 1169
```

<a id="s-6-17-9"></a>

#### § 6.17.9 Global motion structures

```text
§   6.17.9. Global motion structures

```

<a id="s-6-17-9-1"></a>

##### § 6.17.9.1 Global motion params semantics

```text
§   6.17.9.1. Global motion params semantics

    use_global_motion equal to 1 specifies that global motion parameters are present for this frame.
    use_global_motion equal to 0 specifies that no global motion parameters are present.

    our_ref specifies a reference of the current frame. The base warp will be taken from one set of the
    parameters saved for this reference.

    If our_ref is not equal to NumTotalRefs, it is a requirement of bitstream conformance that
    OrderHints[ our_ref ] is not equal to RESTRICTED_OH.

    their_ref specifies a reference that was used by the our_ref reference. The base warp will be taken from
    the warp used by our_ref when it was predicting from their_ref.

    It is a requirement of bitstream conformance that SavedOrderHints[ refIdx ][ their_ref ] is not equal to
    RESTRICTED_OH.

    is_global equal to 1 specifies that global motion parameters are present for a particular reference frame.
    is_global equal to 0 specifies that global motion parameters are not present for this reference frame.

    is_rot_zoom equal to 1 specifies that a particular reference frame uses rotation and zoom global motion.
    is_rot_zoom equal to 0 specifies that a more general affine global motion model is used.

```

<a id="s-6-17-9-2"></a>

##### § 6.17.9.2 Global param semantics

```text
§   6.17.9.2. Global param semantics

    precBits specifies the number of fractional bits used for representing gm_params[ref][idx]. All global
    motion parameters are stored in the model with WARPEDMODEL_PREC_BITS fractional bits, but the
    parameters are encoded with less precision.

```

<a id="s-6-17-9-3"></a>

##### § 6.17.9.3 Decode signed subexp with ref semantics

```text
§   6.17.9.3. Decode signed subexp with ref semantics

      NOTE:       decode_signed_subexp_with_ref will return a value in the range low to high - 1 (inclusive).

```

<a id="s-6-17-9-4"></a>

##### § 6.17.9.4 Decode unsigned subexp with ref semantics

```text
§   6.17.9.4. Decode unsigned subexp with ref semantics

      NOTE:       decode_unsigned_subexp_with_ref will return a value in the range 0 to mx - 1 (inclusive).

```

<a id="s-6-17-9-5"></a>

##### § 6.17.9.5 Decode subexp semantics

```text
§   6.17.9.5. Decode subexp semantics

    subexp_final_bits provide the final bits that are read once the appropriate range has been determined.

    subexp_more_bits equal to 0 specifies that the parameter is in the range mk to mk+a-1.
    subexp_more_bits equal to 1 specifies that the parameter is greater than mk+a-1.

    subexp_bits specifies the value of the parameter minus mk.




    AV2 Specification                                                                              Page 345 of 1169
```

<a id="s-6-17-10"></a>

#### § 6.17.10 Film grain structures

```text
§   6.17.10. Film grain structures

```

<a id="s-6-17-10-1"></a>

##### § 6.17.10.1 Film grain config semantics

```text
§   6.17.10.1. Film grain config semantics

    apply_grain equal to 1 specifies that film grain should be added to this frame. apply_grain equal to 0
    specifies that film grain should not be added.

    fgm_id specifies which film grain model to use.

    It is a requirement of bitstream conformance that FilmGrainPresent[ fgm_id ] is equal to 1.


      NOTE: The film grain model corresponding to fgm_id should be transmitted before it is used by the
      decoding process. See § 7.3.8.8 Film grain OBU availability for the general availability requirements
      for film grain OBUs.


    If apply_grain is equal to 1, it is a requirement of bitstream conformance that all of the following are true:

      • TLayerDependencyMap[obu_mlayer_id][obu_tlayer_id][FgmTLayerId[fgm_id]] is equal to 1,
      • MLayerDependencyMap[obu_mlayer_id][FgmMLayerId[fgm_id]] is equal to 1,
      • FgmChromaIdc[ fgm_id ] is equal to chroma_format_idc.

    grain_seed specifies the initialization value for the pseudo-random numbers generator used during film
    grain synthesis.

    load_grain_model(idx) is a function call that indicates that all syntax elements read in film_grain_model
    should be set equal to the values stored in an area of memory indexed by idx.

```

<a id="s-6-17-10-2"></a>

##### § 6.17.10.2 Film grain model semantics

```text
§   6.17.10.2. Film grain model semantics

    chroma_scaling_from_luma specifies that the film grain model scaling for the chroma component is
    inferred from the film grain model scaling for the luma component.

    num_y_points specifies the number of points for the piece-wise linear scaling function of the luma
    component.

    It is a requirement of bitstream conformance that num_y_points is less than or equal to 14.

    point_value_increment_bits_minus_1 plus 1 specifies the number of bits in the syntax element
    point_y_value (and corresponding chroma syntax elements point_cb_value and point_cr_value, depending
    on the context).

    point_scaling_bits_minus_5 plus 5 specifies the number of bits in the syntax element point_y_scaling
    (and corresponding chroma syntax elements point_cb_scaling and point_cr_scaling, depending on the
    context).

    point_y_value[ i ] represents the x (luma value) coordinate for the i-th point of the piecewise linear
    scaling function for luma component. The values are signaled on the scale of 0..255. (In case of 10 bit
    video, these values correspond to luma values divided by 4.)

    If i is greater than 0, it is a requirement of bitstream conformance that point_y_value[ i ] is greater than
    point_y_value[ i - 1 ] and less than 256. (this ensures the x coordinates are specified in increasing order).




    AV2 Specification                                                                              Page 346 of 1169
  NOTE: This conformance requirement refers to the final values of point_y_value after the addition
  of point_y_value[ i - 1 ].


point_y_scaling[ i ] represents the scaling (output) value for the i-th point of the piecewise linear
scaling function for luma component.

num_cb_points specifies the number of points for the piece-wise linear scaling function of the cb
component.

It is a requirement of bitstream conformance that num_cb_points is less than or equal to 14.

point_cb_value[ i ] represents the x coordinate for the i-th point of the piece-wise linear scaling function
for cb component. The values are signaled on the scale of 0..255.

If i is greater than 0, it is a requirement of bitstream conformance that point_cb_value[ i ] is greater than
point_cb_value[ i - 1 ] and less than 256.

point_cb_scaling[ i ] represents the scaling (output) value for the i-th point of the piecewise linear
scaling function for cb component.

num_cr_points specifies the number of points for the piece-wise linear scaling function of the cr
component.

It is a requirement of bitstream conformance that num_cr_points is less than or equal to 14.

If subX is equal to 1 and subY is equal to 1 and num_cb_points is equal to 0, it is a requirement of
bitstream conformance that num_cr_points is equal to 0.

If subX is equal to 1 and subY is equal to 1 and num_cb_points is not equal to 0, it is a requirement of
bitstream conformance that num_cr_points is not equal to 0.


  NOTE: These requirements ensure that for 4:2:0 chroma subsampling, film grain noise will be
  applied to both chroma components, or to neither. There is no restriction for 4:2:2 or 4:4:4 chroma
  subsampling.


point_cr_value[ i ] represents the x coordinate for the i-th point of the piece-wise linear scaling function
for cr component. The values are signaled on the scale of 0..255.

If i is greater than 0, it is a requirement of bitstream conformance that point_cr_value[ i ] is greater than
point_cr_value[ i - 1 ] and less than 256.

point_cr_scaling[ i ] represents the scaling (output) value for the i-th point of the piecewise linear
scaling function for cr component.

grain_scaling_minus_8 represents the shift – 8 applied to the grain values, which are obtained by a
multiplication of the grain template value with the scaling function value. The grain_scaling_minus_8 can
take values of 0..3 and determines the range and quantization step of the film grain.

ar_coeff_lag specifies the number of auto-regressive coefficients for luma and chroma.

bits_per_ar_coeff_y_minus_5 plus 5 specifies the number of bits in the syntax element ar_coeffs_y.



AV2 Specification                                                                              Page 347 of 1169
bits_per_ar_coeff_cb_minus_5 plus 5 specifies the number of bits in the syntax element ar_coeffs_cb.

bits_per_ar_coeff_cr_minus_5 plus 5 specifies the number of bits in the syntax element ar_coeffs_cr.

ar_coeffs_y[ i ] specifies auto-regressive coefficients used for the Y plane.

ar_coeffs_cb[ i ] specifies auto-regressive coefficients used for the U plane.

ar_coeffs_cr[ i ] specifies auto-regressive coefficients used for the V plane.

ar_coeff_shift_minus_6 specifies the range of the auto-regressive coefficients. Values of 0, 1, 2, and 3
correspond to the ranges for auto-regressive coefficients of [-2, 2), [-1, 1), [-0.5, 0.5) and [-0.25, 0.25)
respectively.

grain_scale_shift specifies how much the Gaussian random numbers are scaled down before the start of
the grain template generation process.

cb_mult represents a multiplier for the cb component used in derivation of the input index to the cb
component scaling function.

cb_luma_mult represents a multiplier for the average luma component used in derivation of the input
index to the cb component scaling function.

cb_offset represents an offset used in derivation of the input index to the cb component scaling function.

cr_mult represents a multiplier for the cr component used in derivation of the input index to the cr
component scaling function.

cr_luma_mult represents a multiplier for the average luma component used in derivation of the input
index to the cr component scaling function.

cr_offset represents an offset used in derivation of the input index to the cr component scaling function.

overlap_flag equal to 1 indicates that the overlap between film grain blocks shall be applied.
overlap_flag equal to 0 indicates that the overlap between film grain blocks shall not be applied.

clip_to_restricted_range equal to 1 indicates that clipping to the restricted (studio) range shall be
applied to the sample values after adding the film grain. clip_to_restricted_range equal to 0 indicates that
clipping to the full range shall be applied to the sample values after adding the film grain.

fg_mc_identity is used to adjust the clipping range for the video after adding the film grain. In
particular, fg_mc_identity equal to 1 specifies that the chroma clipping range is equal to the luma clipping
range when the clip_to_restricted_range is equal to 1.

film_grain_block_size equal to 0 indicates that when the film grain is applied to the reconstructed
samples, a film grain block size of 16 by 16 is used. film_grain_block_size equal to 1 indicates that a film
grain block size of 32 by 32 is used.


  NOTE: The 16 by 16 and 32 by 32 numbers do not take into account the increase in the block size
  when the overlap_flag is equal to 1.




AV2 Specification                                                                               Page 348 of 1169
```

<a id="s-6-18"></a>

### § 6.18 Tile group OBU semantics

```text
§   6.18. Tile group OBU semantics
    is_first_tile_group equal to 1 specifies that this is the first Tile Group for the current frame.
    is_first_tile_group equal to 0 specifies that this is not the first Tile Group for the current frame.

    It is a requirement of bitstream conformance that SeenFrameHeader is not equal to is_first_tile_group.

    frame_header_present_flag equal to 1 specifies that the frame header is present.
    frame_header_present_flag equal to 0 specifies that the frame header is not present.

    NumTiles specifies the total number of tiles in the frame.

    tile_start_and_end_present_flag equal to 1 specifies that the tg_start and tg_end syntax elements are
    present to indicate which tiles are contained in this Tile Group. tile_start_and_end_present_flag equal to 0
    specifies that tg_start and tg_end are not present and this Tile Group covers the entire frame (i.e.,
    tg_start is inferred to be 0 and tg_end is inferred to be NumTiles - 1).

    tg_start specifies the zero-based index of the first tile in the current Tile Group.

    It is a requirement of bitstream conformance that the value of tg_start is equal to the value of TileNum at
    the point that tile_group_payload is invoked.

    tg_end specifies the zero-based index of the last tile in the current Tile Group.

    It is a requirement of bitstream conformance that the value of tg_end is greater than or equal to tg_start.

    It is a requirement of bitstream conformance that the value of tg_end for the last tile group in each frame
    is equal to NumTiles - 1.


      NOTE: These requirements ensure that conceptually all tile groups are present and received in
      order for the purposes of specifying the decode process.


    bru_tile_active equal to 0 specifies that a whole tile is inactive. bru_tile_active equal to 1 specifies that
    the bru_mode syntax element is present for each superblock in a tile.

```

<a id="s-6-19"></a>

### § 6.19 Tile group payload semantics

```text
§   6.19. Tile group payload semantics
```

<a id="s-6-19-1"></a>

#### § 6.19.1 General tile group payload semantics

```text
§   6.19.1. General tile group payload semantics

    frame_end_update_cdf is a function call that indicates that the frame CDF arrays are set equal to the
    saved CDFs. This process is described in § 7.5 Frame end update CDF process.

    tile_size_minus_1 is used to compute tileSize.

    tileSize specifies the size in bytes of the next coded tile.


      NOTE: This size includes any padding bytes if added by the exit process for the Symbol decoder.
      The size does not include the bytes used for tile_size_minus_1 or syntax elements sent before
      tile_size_minus_1. For the last tile in the tile group, tileSize is computed instead of being read and
      includes the OBU trailing bits.




    AV2 Specification                                                                                 Page 349 of 1169
    decode_frame_wrapup is a function call that invokes the decode frame wrapup process specified in § 7.2
    Decode frame wrapup process.

```

<a id="s-6-19-2"></a>

#### § 6.19.2 Tile-level structures

```text
§   6.19.2. Tile-level structures

```

<a id="s-6-19-2-1"></a>

##### § 6.19.2.1 Decode tile semantics

```text
§   6.19.2.1. Decode tile semantics

    clear_left_context is a function call that indicates that some arrays are initialized. When this function is
    invoked the arrays WarpBankSize, WarpBankStart, RefMvBankSize, RefMvBankStart, LeftLevelContext,
    LeftDcContext, LeftMiSizes, and LeftSegPredContext are initialized as follows:

     for (i = 0; i < MiRows; i++) {
         for (plane = 0; plane < 3; plane++) {
             LeftDcContext[ plane ][ i ] = 0
             LeftLevelContext[ plane ][ i ] = 0
         }
         LeftSegPredContext[ i ] = 0
     }
     sbSize4 = Num_4x4_Blocks_High[ SbSize ]
     numSbs = (MiRows + sbSize4 - 1) / sbSize4
     for (i = 0; i < numSbs * sbSize4; i++) {
         LeftMiSizes[ 0 ][ i ] = BLOCK_256X256
         LeftMiSizes[ 1 ][ i ] = BLOCK_256X256
     }
     for(ref = 0; ref < REFS_PER_FRAME; ref++) {
         WarpBankSize[ ref ] = 0
         WarpBankStart[ ref ] = 0
     }
     for(ref = 0; ref < BANK_REFS_PER_FRAME; ref++) {
         RefMvBankSize[ ref ] = 0
         RefMvBankStart[ ref ] = 0
     }


    clear_above_context is a function call that indicates that some arrays used to determine the
    probabilities are initialized. When this function is invoked the arrays AboveLevelContext,
    AboveDcContext, AboveMiSizes, and AboveSegPredContext are initialized as follows:

     for (i = 0; i < MiCols; i++) {
         for (plane = 0; plane < 3; plane++) {
             AboveDcContext[ plane ][ i ] = 0
             AboveLevelContext[ plane ][ i ] = 0
         }
         AboveSegPredContext[ i ] = 0
     }
     sbSize4 = Num_4x4_Blocks_Wide[ SbSize ]
     numSbs = (MiCols + sbSize4 - 1) / sbSize4
     for (i = 0; i < numSbs * sbSize4; i++) {
         AboveMiSizes[ 0 ][ i ] = BLOCK_256X256
         AboveMiSizes[ 1 ][ i ] = BLOCK_256X256
     }


    TreeType specifies which syntax elements are present as follows:

                        TreeType                                     Name of TreeType

                           0                                           SHARED_PART

                           1                                            LUMA_PART

                           2                                           CHROMA_PART




    AV2 Specification                                                                             Page 350 of 1169
    When TreeType is equal to LUMA_PART, syntax elements related to the luma plane are present. When
    TreeType is equal to CHROMA_PART, syntax elements related to the chroma plane are present.
    Otherwise (TreeType is equal to SHARED_PART), both luma and chroma syntax elements can be present.

    ReadDeltas specifies whether the current block may read delta values for the quantizer index. If the
    entire superblock is skipped the delta values are not read, otherwise delta values for the quantizer index
    are read on the first block of a superblock. If delta_q_present is equal to 0, no delta values are read for
    the quantizer index.

    bru_mode specifies the type of superblock as specified in Table 6.21:

                                   Table 6.21: bru_mode values and interpretations

                        bru_mode                                     Name of bru_mode

                           0                                           BRU_INACTIVE

                           1                                           BRU_SUPPORT

                           2                                            BRU_ACTIVE



      NOTE: bru_mode is also used outside BRU frames to determine if the syntax elements are parsed.
      In bridge frames, syntax is inferred, so bru_mode is BRU_INACTIVE. In normal frames, syntax is
      parsed, so bru_mode is BRU_ACTIVE.

```

<a id="s-6-19-2-2"></a>

##### § 6.19.2.2 Reset reference motion vector bank function semantics

```text
§   6.19.2.2. Reset reference motion vector bank function semantics

    WarpBankHits counts how many times the WarpBankParams have been searched in the superblock.

    RefMvBankHits counts how many times update_ref_mv_bank has been called in the superblock.

    RefMvUnitHits counts how many times update_ref_mv_bank has been called since the last time the
    current block was aligned to a unit boundary. The unit size is defined relative to the superblock size such
    that a grid of 8 by 8 units fits within the superblock.

    RefMvRemainHits defines how many calls to update_ref_mv_bank are allowed. This variable decreases
    when update_ref_mv_bank is called, but can be increased if a large block is processed that is aligned to a
    unit boundary.

```

<a id="s-6-19-2-3"></a>

##### § 6.19.2.3 Clear block decoded flags function semantics

```text
§   6.19.2.3. Clear block decoded flags function semantics

    BlockDecoded is an array which stores one boolean value per 4x4 sample block per plane in the current
    superblock, plus a border of one 4x4 sample block on all sides of the superblock. Except for the borders,
    a value of 1 in BlockDecoded indicates that the corresponding 4x4 sample block has been decoded. The
    borders are used when computing above-right and below-left availability along the top and left edges of
    the superblock.

```

<a id="s-6-19-3"></a>

#### § 6.19.3 Partition structures

```text
§   6.19.3. Partition structures

```

<a id="s-6-19-3-1"></a>

##### § 6.19.3.1 Decode partition semantics

```text
§   6.19.3.1. Decode partition semantics

    The parameter hasChroma specifies that this partition contains one or more blocks with chroma mode
    information.



    AV2 Specification                                                                             Page 351 of 1169
The parameter chromaOffset specifies whether the minimum size for chroma blocks has been reached.
chromaOffset equal to 0 specifies that the minimum size has not been reached (in this case the chroma
block will be the same size as the luma block). chromaOffset equal to 1 specifies that the minimum size
has been reached (in this case the chroma block has stopped splitting so may be a different size to the
luma block).

If chromaOffset is equal to 1 and hasChroma is equal to 1 and TreeType is not equal to LUMA_PART and
NumPlanes is greater than 1, it is a requirement of bitstream conformance that r is less than MiRows or c
is less than MiCols.


  NOTE: This requirement ensures that chroma info is always present. To satisfy this requirement,
  only certain partition choices can be made near the edge.


If r is less than MiRows or c is less than MiCols, then if hasChroma is equal to 1 it is a requirement of
bitstream conformance that get_plane_residual_size( chromaOffset ? ChromaMiSize : subSize, 1 ) is not
equal to BLOCK_INVALID.


  NOTE: This requirement of bitstream conformance applies to the values of variables chromaOffset,
  ChromaMiSize, and subSize at the point just before the line if ( partition == PARTITION_NONE ) {.


ChromaMiRow is a variable holding the vertical location of the chroma block in units of 4x4 luma
samples.

ChromaMiCol is a variable holding the horizontal location of the chroma block in units of 4x4 luma
samples.

ChromaMiSize is a variable holding the size of the chroma block with values having the same
interpretation for the variable subSize. The size corresponds to the amount of luma samples that are
covered by the chroma block.

The variable partition specifies how a block is partitioned:

                partition                                       Name of partition

                    0                                           PARTITION_NONE

                    1                                           PARTITION_HORZ

                    2                                           PARTITION_VERT

                    3                                          PARTITION_HORZ_3

                    4                                          PARTITION_VERT_3

                    5                                          PARTITION_HORZ_4A

                    6                                          PARTITION_HORZ_4B

                    7                                          PARTITION_VERT_4A

                    8                                          PARTITION_VERT_4B

                    9                                           PARTITION_SPLIT



  NOTE: PARTITION_HORZ_3 and PARTITION_VERT_3 split into four parts by first splitting in a ratio
  1:2:1, and then splitting the middle section in the perpendicular direction.



AV2 Specification                                                                            Page 352 of 1169
The variable subSize is computed from partition and indicates the size of the component blocks within
this block as specified in Table 6.22:

                              Table 6.22: subSize values for different partition types

                    subSize                                         Name of subSize

                      0                                               BLOCK_4X4

                      1                                               BLOCK_4X8

                      2                                               BLOCK_8X4

                      3                                               BLOCK_8X8

                      4                                               BLOCK_8X16

                      5                                               BLOCK_16X8

                      6                                              BLOCK_16X16

                      7                                              BLOCK_16X32

                      8                                              BLOCK_32X16

                      9                                              BLOCK_32X32

                      10                                             BLOCK_32X64

                      11                                             BLOCK_64X32

                      12                                             BLOCK_64X64

                      13                                             BLOCK_64X128

                      14                                             BLOCK_128X64

                      15                                            BLOCK_128X128

                      16                                            BLOCK_128X256

                      17                                            BLOCK_256X128

                      18                                            BLOCK_256X256

                      19                                              BLOCK_4X16

                      20                                              BLOCK_16X4

                      21                                              BLOCK_8X32

                      22                                              BLOCK_32X8

                      23                                             BLOCK_16X64

                      24                                             BLOCK_64X16

                      25                                              BLOCK_4X32

                      26                                              BLOCK_32X4

                      27                                              BLOCK_8X64

                      28                                              BLOCK_64X8



  NOTE: When a partition splits into blocks of different sizes, the first and final blocks will be of size
  subSize.


The dimensions of these blocks are given in width, height order (e.g. BLOCK_8X16 corresponds to a block
that is 8 samples wide, and 16 samples high).




AV2 Specification                                                                              Page 353 of 1169
    ChromaFollowsLuma is a variable that is used to decide whether the chroma partitioning follows luma.
    The chroma partitioning follows luma if luma is split and none of the split partitions contains a block
    smaller than 32 by 32.

    ChromaPartitionKnown is an array that records where the chroma partitioning is already known (as it
    is forced to follow the luma partitioning).

    region_type equal to INTRA_REGION indicates that the luma partition tree is sent first, followed by
    information about a single chroma block. All blocks in this case will be intra blocks.

```

<a id="s-6-19-3-2"></a>

##### § 6.19.3.2 Read partition semantics

```text
§   6.19.3.2. Read partition semantics

    do_split equal to 1 specifies that the block is to be split further. do_split equal to 0 specifies that no
    further splitting is performed.

    do_square_split equal to 1 specifies that the block is split into 4 square parts. do_square_split equal to 0
    specifies that the block is not split into 4 square parts.

    rect_type specifies the direction in which the block is to be split. rect_type is equal to RECT_HORZ for a
    horizontal cut. rect_type is equal to RECT_VERT for a vertical cut.

    do_ext_partition equal to 1 specifies that extended partitions are used and the block is split into four
    parts. do_ext_partition equal to 0 specifies that the block is split into two parts.

    do_uneven_4way_partition equal to 1 specifies that an uneven partition is used when splitting the block
    into four parts. do_uneven_4way_partition equal to 0 specifies that the uneven 4-way partition is not used
    for the block.

    uneven_4way_partition_type specifies the type of uneven partition.

    Rect_Part_Table is a lookup table for finding the chosen partition.

```

<a id="s-6-19-4"></a>

#### § 6.19.4 Block decoding structures

```text
§   6.19.4. Block decoding structures

```

<a id="s-6-19-4-1"></a>

##### § 6.19.4.1 Decode block semantics

```text
§   6.19.4.1. Decode block semantics

    MiRow is a variable holding the vertical location of the block in units of 4x4 luma samples.

    MiCol is a variable holding the horizontal location of the block in units of 4x4 luma samples.

    MiSize is a variable holding the size of the block with values having the same interpretation for the
    variable subSize.

    HasChroma is a variable that specifies whether chroma information is coded for this block.

    Variable AvailU is equal to 0 if the information from the block above cannot be used on the luma plane;
    AvailU is equal to 1 if the information from the block above can be used on the luma plane.

    Variable AvailL is equal to 0 if the information from the block to the left cannot be used on the luma
    plane; AvailL is equal to 1 if the information from the block to the left can be used on the luma plane.

    Variables AvailUChroma and AvailLChroma have the same significance as AvailU and AvailL, but on the
    chroma planes.




    AV2 Specification                                                                                 Page 354 of 1169
    SubMvs contains motion vectors for each 4x4 subblock. SubMvs are initialized in decode block, but can
    get adjusted if the block is predicted with a warped prediction.

    The function call to motion_field_motion_vector_storage indicates that the motion field motion vector
    storage process specified in § 7.22 Motion field motion vector storage process is invoked.

    After all the syntax elements have been read for the block, if is_inter is equal to 0, it is a requirement of
    bitstream conformance that seg_feature_active(SEG_LVL_SKIP) is equal to 0.

    After the local variables bw4 and bh4 have been computed in the decode block syntax, it is a requirement
    of bitstream conformance that bw4 is less than or equal to bh4 * MaxPbAspectRatio, and that bh4 is less
    than or equal to bw4 * MaxPbAspectRatio.

```

<a id="s-6-19-5"></a>

#### § 6.19.5 Mode information structures

```text
§   6.19.5. Mode information structures

```

<a id="s-6-19-5-1"></a>

##### § 6.19.5.1 Mode info semantics

```text
§   6.19.5.1. Mode info semantics

    This switches between different ways of reading the mode info for different frame types.

```

<a id="s-6-19-5-2"></a>

##### § 6.19.5.2 BRU mode info semantics

```text
§   6.19.5.2. BRU mode info semantics

    This syntax is used for inactive and support BRU blocks.

```

<a id="s-6-19-5-3"></a>

##### § 6.19.5.3 Intra frame mode info semantics

```text
§   6.19.5.3. Intra frame mode info semantics

    This syntax is used when coding an intra block within an intra frame.

    use_intrabc equal to 1 specifies that intra block copy is used for this block. use_intrabc equal to 0
    specifies that intra block copy is not used.

```

<a id="s-6-19-5-4"></a>

##### § 6.19.5.4 Read intra block copy semantics

```text
§   6.19.5.4. Read intra block copy semantics

    This syntax is used when coding a motion vector for intra block copy.

    intrabc_mode equal to 1 indicates that there is no motion vector difference. intrabc_mode equal to 0
    indicates that a motion vector difference is present.

    intrabc_drl_mode is used to select a predicted motion vector from the stack.

    intrabc_precision is used to decide the motion vector precision for intra block copy.

    morph_pred equal to 1 specifies that morphological prediction (which tries to adjust the brightness of
    the samples to match the context) is used for this block. morph_pred equal to 0 specifies that
    morphological prediction is not used.

    If morph_pred is equal to 1, it is a requirement of bitstream conformance that is_offset_mv_valid( -1, -1 )
    is equal to 1.




    AV2 Specification                                                                               Page 355 of 1169
    The function is_offset_mv_valid is defined as:

     is_offset_mv_valid( dx, dy ) {
         offsetMv[0] = BlockMvs[0][0] + dy * 8
         offsetMv[1] = BlockMvs[0][1] + dx * 8
         return is_mv_valid( offsetMv )
     }



      NOTE: This constraint ensures that the extra reference pixels fetched are also valid for intra block
      copy prediction.

```

<a id="s-6-19-5-5"></a>

##### § 6.19.5.5 Read intra Y mode semantics

```text
§   6.19.5.5. Read intra Y mode semantics

    use_dpcm_y equal to 1 specifies that Differential Pulse Code Modulation (DPCM) is used for luma
    prediction. use_dpcm_y equal to 0 specifies that DPCM is not used for luma.

    dpcm_mode_y is used to compute the direction for intra prediction when using DPCM.

    y_mode_set equal to 0 specifies that y_mode_index is present. y_mode_set equal to 1 specifies that
    y_second_mode is present.

    y_mode_index and y_mode_offset are used to send the first set of YMode choices.

    y_second_mode is used to send the second set of YMode choices.

    fsc_mode is used to control if the block uses forward skip coding of the coefficients and the type of
    transform.

    mrl_index specifies the distance of the reference samples used for intra prediction.

    mrl_sec_index equal to 1 specifies that the block uses a secondary intra prediction. mrl_sec_index equal
    to 0 specifies that only primary intra prediction is used.

    YMode specifies the direction of intra prediction filtering:

                    YMode                                          Name of YMode

                        0                                             DC_PRED

                        1                                              V_PRED

                        2                                              H_PRED

                        3                                             D45_PRED

                        4                                            D135_PRED

                        5                                            D113_PRED

                        6                                            D157_PRED

                        7                                            D203_PRED

                        8                                             D67_PRED

                        9                                          SMOOTH_PRED

                        10                                         SMOOTH_V_PRED

                        11                                         SMOOTH_H_PRED

                        12                                          PAETH_PRED




    AV2 Specification                                                                            Page 356 of 1169
    AngleDeltaY is computed from y_mode_index, y_mode_offset, and y_second_mode to produce the final
    luma angle offset value, which may be positive or negative.

```

<a id="s-6-19-5-6"></a>

##### § 6.19.5.6 Read intra UV mode semantics

```text
§   6.19.5.6. Read intra UV mode semantics

    use_dpcm_uv equal to 1 specifies that Differential Pulse Code Modulation (DPCM) is used for chroma
    prediction. use_dpcm_uv equal to 0 specifies that DPCM is not used for chroma.

    dpcm_mode_uv is used to compute the direction for intra prediction when using DPCM.

    is_cfl equal to 1 specifies that chroma from luma (CFL) prediction is used for chroma components. is_cfl
    equal to 0 specifies that CFL prediction is not used.

    uv_mode and uv_mode_idx are used to compute the UVMode.

    It is a requirement of bitstream conformance that uv_mode_idx is less than or equal to 5.

    UVMode specifies the chrominance intra prediction mode using values with the same interpretation as in
    the semantics for YMode, with an additional mode UV_CFL_PRED.

                        UVMode                                        Name of UVMode

                          0                                              DC_PRED

                          1                                               V_PRED

                          2                                               H_PRED

                          3                                              D45_PRED

                          4                                              D135_PRED

                          5                                              D113_PRED

                          6                                              D157_PRED

                          7                                              D203_PRED

                          8                                              D67_PRED

                          9                                            SMOOTH_PRED

                          10                                          SMOOTH_V_PRED

                          11                                          SMOOTH_H_PRED

                          12                                            PAETH_PRED

                          13                                           UV_CFL_PRED


    AngleDeltaUV is computed from uv_mode and may be positive or negative.

```

<a id="s-6-19-5-7"></a>

##### § 6.19.5.7 Intra segment ID semantics

```text
§   6.19.5.7. Intra segment ID semantics

    Lossless is a variable which, if equal to 1, indicates that the block is coded using a special reversible
    transform designed for encoding frames that are bit-identical with the original frames.

```

<a id="s-6-19-5-8"></a>

##### § 6.19.5.8 Read segment ID semantics

```text
§   6.19.5.8. Read segment ID semantics

    seg_id_ext_flag and segment_id specify which segment is associated with the current intra block being
    decoded. It is first read from the stream, and then postprocessed based on the predicted segment id.




    AV2 Specification                                                                              Page 357 of 1169
    It is a requirement of bitstream conformance that the postprocessed value of segment_id (i.e., the value
    returned by neg_deinterleave) is in the range 0 to LastActiveSegId (inclusive of endpoints).

```

<a id="s-6-19-5-9"></a>

##### § 6.19.5.9 Skip mode semantics

```text
§   6.19.5.9. Skip mode semantics

    skip_mode equal to 1 indicates that this block will use some default settings (that correspond to
    compound prediction) and so most of the mode info is skipped. skip_mode equal to 0 indicates that the
    mode info is not skipped.

```

<a id="s-6-19-5-10"></a>

##### § 6.19.5.10 Skip semantics

```text
§   6.19.5.10. Skip semantics

    skip_flag equal to 0 indicates that there can be some transform coefficients to read for this block;
    skip_flag equal to 1 indicates that there are no transform coefficients.

```

<a id="s-6-19-5-11"></a>

##### § 6.19.5.11 Quantizer index delta semantics

```text
§   6.19.5.11. Quantizer index delta semantics

    delta_q_abs specifies the absolute value of the quantizer index delta value being decoded. If delta_q_abs
    is equal to DELTA_Q_SMALL, the value is encoded using delta_q_rem_bits and delta_q_abs_bits.

    delta_q_rem_bits and delta_q_abs_bits encode the absolute value of the quantizer index delta value
    being decoded, where the absolute value of the quantizer index delta value is of the form:

     (1 << delta_q_rem_bits) + delta_q_abs_bits + 1


    delta_q_sign_bit equal to 0 indicates that the quantizer index delta value is positive; delta_q_sign_bit
    equal to 1 indicates that the quantizer index delta value is negative.

```

<a id="s-6-19-6"></a>

#### § 6.19.6 Transform and quantization structures

```text
§   6.19.6. Transform and quantization structures

```

<a id="s-6-19-6-1"></a>

##### § 6.19.6.1 TX size semantics

```text
§   6.19.6.1. TX size semantics

    lossless_tx_size equal to 1 specifies that a 4x4 or larger transform size is used for a lossless block.
    lossless_tx_size equal to 0 specifies that the transform size is constrained for lossless coding.

    TxSize specifies the transform size to be used for this block:

                        TxSize                                       Name of TxSize

                          0                                              TX_4X4

                          1                                              TX_8X8

                          2                                             TX_16X16

                          3                                             TX_32X32

                          4                                             TX_64X64

                          5                                              TX_4X8

                          6                                              TX_8X4

                          7                                              TX_8X16

                          8                                              TX_16X8

                          9                                             TX_16X32

                         10                                             TX_32X16

                         11                                             TX_32X64



    AV2 Specification                                                                              Page 358 of 1169
                          12                                            TX_64X32

                          13                                            TX_4X16

                          14                                            TX_16X4

                          15                                            TX_8X32

                          16                                            TX_32X8

                          17                                            TX_16X64

                          18                                            TX_64X16

                          19                                            TX_4X32

                          20                                            TX_32X4

                          21                                            TX_8X64

                          22                                            TX_64X8

                          23                                            TX_4X64

                          24                                            TX_64X4

                          255                                          TX_INVALID



      NOTE: TxSize is determined for skipped intra blocks because TxSize controls the granularity of the
      intra prediction.

```

<a id="s-6-19-6-2"></a>

##### § 6.19.6.2 Block TX size semantics

```text
§   6.19.6.2. Block TX size semantics

    LumaTxSizes is an array that holds the luma transform sizes.

    LumaTxMiddle is an array that records whether the transform block was from the middle of a transform
    partition. (This information is important for intra prediction as top-right and bottom-left values are
    marked unavailable for middle blocks.)

```

<a id="s-6-19-6-3"></a>

##### § 6.19.6.3 Read TX partition semantics

```text
§   6.19.6.3. Read TX partition semantics

    tx_do_partition equal to 1 specifies that the block is split into smaller transform sizes. tx_do_partition
    equal to 0 specifies that the block is not split any more.

    tx_partition_type and tx_2or3_partition_type are used to indicate the transform partition.

    txPartition specifies the transform partition as specified in Table 6.23:

                                      Table 6.23: txPartition values and names

                        txPartition                                 Name of txPartition

                            0                                       TX_PARTITION_NONE

                            1                                       TX_PARTITION_SPLIT

                            2                                       TX_PARTITION_HORZ

                            3                                       TX_PARTITION_VERT

                            4                                       TX_PARTITION_HORZ4

                            5                                       TX_PARTITION_VERT4

                            6                                       TX_PARTITION_HORZ5




    AV2 Specification                                                                              Page 359 of 1169
                             7                                        TX_PARTITION_VERT5


    It is a requirement of bitstream conformance that the return value of the function set_tx_size is not equal
    to TX_INVALID.

```

<a id="s-6-19-7"></a>

#### § 6.19.7 Motion vector and prediction structures

```text
§   6.19.7. Motion vector and prediction structures

```

<a id="s-6-19-7-1"></a>

##### § 6.19.7.1 Inter frame mode info semantics

```text
§   6.19.7.1. Inter frame mode info semantics

    This reads syntax elements for blocks within an inter frame.

```

<a id="s-6-19-7-2"></a>

##### § 6.19.7.2 Inter segment ID semantics

```text
§   6.19.7.2. Inter segment ID semantics

    seg_id_predicted equal to 1 specifies that the segment_id is taken from the segmentation map.
    seg_id_predicted equal to 0 specifies that the syntax element segment_id is parsed.


      NOTE: It is allowed for seg_id_predicted to be equal to 0 even if the value coded for the segment_id
      is equal to predictedSegmentId.

```

<a id="s-6-19-7-3"></a>

##### § 6.19.7.3 Is inter semantics

```text
§   6.19.7.3. Is inter semantics

    is_inter equal to 0 specifies that the block is an intra block; is_inter equal to 1 specifies that the block is
    an inter block.


      NOTE: When intra block copy is used within an inter frame, the syntax element is_inter is read as 0,
      but then modified to equal 1 as the motion vector prediction uses the IsInters array to detect blocks
      with motion vectors and intra block copy includes motion vectors.


      NOTE:       The semantics of use_intrabc are provided in § 6.19.5.3 Intra frame mode info semantics.

```

<a id="s-6-19-7-4"></a>

##### § 6.19.7.4 Intra block mode info semantics

```text
§   6.19.7.4. Intra block mode info semantics

    This syntax is used when coding an intra block within an inter frame.

```

<a id="s-6-19-7-5"></a>

##### § 6.19.7.5 Inter block mode info semantics

```text
§   6.19.7.5. Inter block mode info semantics

    This syntax is used when coding an inter block.

    tip_pred_mode is used to compute the YMode when using TIP.

    is_warp specifies that the YMode is either WARPMV or WARP_NEWMV.

    warp_mv specifies that the YMode is set to WARPMV.

    use_amvd specifies that an asymmetric motion vector difference is used.

    single_mode, is_joint, compound_mode_non_joint, and compound_mode_same_refs specify how the
    motion vector used by inter prediction is obtained. An offset is added to compute YMode as follows:

                   YMode             Name of YMode

                        14           NEARMV




    AV2 Specification                                                                                Page 360 of 1169
                    15           GLOBALMV

                    16           NEWMV

                    17           WARPMV

                    18           WARP_NEWMV

                    19           NEAR_NEARMV

                    20           NEAR_NEWMV

                    21           NEW_NEARMV

                    22           GLOBAL_GLOBALMV

                    23           NEW_NEWMV

                    24           JOINT_NEWMV



  NOTE:       The intra modes take values 0 to 13 so these YMode values start at 14.


use_optflow specifies that optical flow is used for this block.

use_bawp equal to 1 specifies that BAWP is used for this block for luma samples.

explicit_bawp equal to 1 specifies that BAWP scaling factor is based on OrderHints.

explicit_bawp_scale specifies the sign for BAWP scaling factor delta based on OrderHints.

use_bawp_chroma equal to 1 specifies that BAWP is used for this block for chroma samples.

warp_idx equal to 0 specifies that a particular warp reference candidate is used to compute the warp
parameters.

warpmv_with_mvd specifies that a motion vector difference is present which will be used to compute the
warp parameters.

jmvd_scale_mode specifies a parameter used while scaling motion vectors in joint mode.

use_most_probable_precision equal to 1 specifies that the frame level precision is used for motion
vectors. use_most_probable_precision equal to 0 specifies that the syntax element pb_mv_precision is
read to determine the precision.

pb_mv_precision is used to compute the precision for motion vectors.

cwp_idx is used to compute the compound weighting factor.

interp_filter specifies the type of filter used in inter prediction. Values 0..3 are allowed with the same
interpretation as for interpolation_filter.


  NOTE: The syntax element interpolation_filter from the frame header info can specify the type of
  filter to be used for the whole frame. If it is set to SWITCHABLE then the interp_filter syntax element
  is read from the bitstream for every inter block.




AV2 Specification                                                                              Page 361 of 1169
    When all the syntax elements have been read in the inter block mode info syntax, if use_bru is equal to 1,
    it is a requirement of bitstream conformance that:

      • RefFrame[0] is not equal to bru_ref
      • RefFrame[1] is not equal to bru_ref

    When all the syntax elements have been read in the inter block mode info syntax, if use_bru is equal to 1
    and RefFrame[0] is equal to TIP_FRAME, it is a requirement of bitstream conformance that:

      • ClosestPast is not equal to bru_ref
      • ClosestFuture is not equal to bru_ref

```

<a id="s-6-19-7-6"></a>

##### § 6.19.7.6 Read warp delta semantics

```text
§   6.19.7.6. Read warp delta semantics

    warp_delta_precision equal to 1 specifies that high precision warp parameters are used for the block.
    warp_delta_precision equal to 0 specifies that standard precision warp parameters are used.

    warp_delta_param_low, warp_delta_param_high, and warp_delta_param_sign are used to compute a
    warp parameter as an offset from the predicted value.

```

<a id="s-6-19-7-7"></a>

##### § 6.19.7.7 Read drl idx semantics

```text
§   6.19.7.7. Read drl idx semantics

    RefMvIdx specifies which candidate in the RefStackMv is used.

    RefMvIdx0 specifies which candidate in the RefStack0Mvs is used.

    RefMvIdx1 specifies which candidate in the RefStack1Mvs is used.

    drl_mode is a bit sent for candidates in the motion vector stack to indicate if they are used. drl_mode
    equal to 0 means to use the current value of idx. drl_mode equal to 1 says to continue searching. DRL
    stands for "Dynamic Reference List".

```

<a id="s-6-19-7-8"></a>

##### § 6.19.7.8 DIP mode info semantics

```text
§   6.19.7.8. DIP mode info semantics

    use_dip is a bit specifying whether or not data driven intra prediction can be used.

    dip_mode and dip_transpose are parameters used in the data driven intra prediction process.

```

<a id="s-6-19-7-9"></a>

##### § 6.19.7.9 Ref frames semantics

```text
§   6.19.7.9. Ref frames semantics

    tip_mode equal to 1 specifies that Temporally Interpolated Prediction (TIP) is used for the block.
    tip_mode equal to 0 specifies that TIP is not used and regular inter prediction is applied.

    comp_mode equal to 1 specifies that compound prediction is used for the block, blending predictions
    from two reference frames. comp_mode equal to 0 specifies that single reference prediction is used.

                    comp_mode                                     Name of comp_mode

                        0                                         SINGLE_REFERENCE

                        1                                       COMPOUND_REFERENCE


    SINGLE_REFERENCE indicates that the inter block uses only a single reference frame to generate
    motion compensated prediction.



    AV2 Specification                                                                            Page 362 of 1169
    COMPOUND_REFERENCE indicates that the inter block uses compound mode.

    RefFrame[ 0 ] specifies which frame is used to compute the predicted samples for this block:

                        RefFrame[ 0 ]                                        Name of ref_frame

                             7                                                   TIP_FRAME

                             8                                                  INTRA_FRAME



      NOTE: Values from 0 to 6 are also allowed, but do not have a name. These values correspond to
      using different inter frames for reference.


    RefFrame[ 1 ] specifies which additional frame is used in compound prediction:

            RefFrame[ 1 ]                                        Name of ref_frame

                  -1                                   NONE (this block uses single prediction)

                   8                             INTRA_FRAME (this block uses inter intra prediction)



      NOTE: Values from 0 to 6 are also allowed, but do not have a name. These values correspond to
      using different inter frames for reference.

```

<a id="s-6-19-7-10"></a>

##### § 6.19.7.10 Read compound ref semantics

```text
§   6.19.7.10. Read compound ref semantics

    If read_compound_ref is called, it is a requirement of bitstream conformance that NumTotalRefs is greater
    than 0.

    comp_ref equal to 1 means that reference ref is used for inter prediction by this block.

```

<a id="s-6-19-7-11"></a>

##### § 6.19.7.11 Read single ref semantics

```text
§   6.19.7.11. Read single ref semantics

    If read_single_ref is called, it is a requirement of bitstream conformance that NumTotalRefs is greater
    than 0.

    single_ref equal to 1 means that reference ref is used for inter prediction by this block.

```

<a id="s-6-19-7-12"></a>

##### § 6.19.7.12 Assign MV semantics

```text
§   6.19.7.12. Assign MV semantics

    mv_sign equal to 0 means that the motion vector difference is positive; mv_sign equal to 1 means that
    the motion vector difference is negative.

    It is a requirement of bitstream conformance that whenever assign_mv returns,
    is_mv_valid( BlockMvs[0] ) is equal to 1, where is_mv_valid is defined as:

     is_mv_valid( mv ) {
         if ( !use_intrabc ) {
             return 1
         }
         bw = Block_Width[ MiSize ]
         bh = Block_Height[ MiSize ]
         bottomBorder = (mv[ 0 ] & 7) != 0 ? 1 : 0
         rightBorder = (mv[ 1 ] & 7) != 0 ? 1 : 0
         deltaRow = mv[ 0 ] >> 3
         deltaCol = mv[ 1 ] >> 3



    AV2 Specification                                                                                   Page 363 of 1169
      srcTopEdge = MiRow * MI_SIZE + deltaRow
      srcLeftEdge = MiCol * MI_SIZE + deltaCol
      srcBottomEdge = srcTopEdge + bh + bottomBorder
      srcRightEdge = srcLeftEdge + bw + rightBorder
      if (HasChroma) {
          srcLeftEdge = ChromaMiCol * MI_SIZE + deltaCol
          srcTopEdge = ChromaMiRow * MI_SIZE + deltaRow
      }
      if ( srcTopEdge < MiRowStart * MI_SIZE ||
            srcLeftEdge < MiColStart * MI_SIZE ||
            srcBottomEdge > MiRowEnd * MI_SIZE ||
            srcRightEdge > MiColEnd * MI_SIZE ) {
          return 0
      }
      if ( allow_local_intrabc ) {
          tmpCol = MiCol
          tmpRow = MiRow
          if ( (!enable_sdp || !FrameIsIntra) && HasChroma) {
               bw = Block_Width[ ChromaMiSize ]
               tmpCol = ChromaMiCol
               bh = Block_Height[ ChromaMiSize ]
               tmpRow = ChromaMiRow
          }
          tmpTopEdge = tmpRow * MI_SIZE + deltaRow
          tmpLeftEdge = tmpCol * MI_SIZE + deltaCol
          tmpBottomEdge = tmpTopEdge + bh - 1 + bottomBorder
          tmpRightEdge = tmpLeftEdge + bw - 1 + rightBorder
          if (check_valid_local_ibc(tmpLeftEdge, tmpTopEdge) &&
               check_valid_local_ibc(tmpRightEdge, tmpBottomEdge)) {
               return 1
          }
      }
      if (!allow_global_intrabc) {
          return 0
      }
      sbH = Block_Height[ SbSize ]
      activeSbRow = (MiRow * MI_SIZE) / sbH
      activeSb64Col = (MiCol * MI_SIZE) >> 6
      srcSbRow = (srcBottomEdge - 1) / sbH
      srcSb64Col = (srcRightEdge - 1) >> 6
      activeSb64Row = (MiRow * MI_SIZE) >> 6
      isBottomLeft = (activeSb64Col & 1) == 0 && (activeSb64Row & 1) == 1
      if (AllowExtraIBCRange && isBottomLeft) {
          sb64Residual = -1
      } else {
          sb64Residual = 0
      }
      totalSb64PerRow = ((MiColEnd - MiColStart - 1) >> 4) + 1
      activeSb64 = activeSbRow * totalSb64PerRow + activeSb64Col
      srcSb64 = srcSbRow * totalSb64PerRow + srcSb64Col
      if ( srcSb64 >= activeSb64 - INTRABC_DELAY_SB64 - sb64Residual) {
          return 0
      }
      gradient = INTRABC_DELAY_SB64 + (Block_Width[ SbSize ] / 64)
      wfOffset = gradient * (activeSbRow - srcSbRow)
      if ( srcSbRow > activeSbRow ||
            srcSb64Col >= activeSb64Col - INTRABC_DELAY_SB64 +
                          wfOffset - sb64Residual ) {
          return 0
      }
      return 1
 }



  NOTE: The purpose of this function is to constrain the motion vectors used for intra BC in order
  that the data is fetched from parts of the tile that have already been decoded.




AV2 Specification                                                                         Page 364 of 1169
      NOTE: The constraints when allow_local_intrabc is equal to 1 are intended to allow an
      implementation that stores the four most recently decoded 64x64 regions of the image in a cache.


    The function check_valid_local_ibc (which checks if a location is within the allowed intra block copy
    buffers) is specified as:

     check_valid_local_ibc( x, y ) {
         if ( (!enable_sdp || !FrameIsIntra) && HasChroma) {
             actCol = ChromaMiCol
             actRow = ChromaMiRow
         } else {
             actCol = MiCol
             actRow = MiRow
         }
         if (x >= actCol * MI_SIZE && y >= actRow * MI_SIZE) {
             return 0
         }
         if ( !IBCCoded[y >> MI_SIZE_LOG2][x >> MI_SIZE_LOG2] ) {
             return 0
         }
         bufCol = x >> IBC_BUFFER_SIZE_LOG2
         bufRow = y >> IBC_BUFFER_SIZE_LOG2
         bufIdx = ibc_buffer_index(bufRow, bufCol)
         inCurrent = bufCol == IBCBufferCurCol && bufRow == IBCBufferCurRow
         if (!inCurrent) {
             if ( !IBCBufferValid[bufIdx] ||
                    bufCol != IBCBufferCol[bufIdx] ||
                    bufRow != IBCBufferRow[bufIdx] ) {
                  return 0
             }
         }
         if ( bufIdx == ibc_buffer_index(IBCBufferCurRow, IBCBufferCurCol) ) {
             if (!inCurrent) {
                  coloY = (y & (IBC_BUFFER_SIZE - 1)) |
                           (IBCBufferCurRow << IBC_BUFFER_SIZE_LOG2)
                  coloX = (x & (IBC_BUFFER_SIZE - 1)) |
                           (IBCBufferCurCol << IBC_BUFFER_SIZE_LOG2)
                  if ( IBCCoded[ coloY >> MI_SIZE_LOG2 ][ coloX >> MI_SIZE_LOG2 ] ) {
                       return 0
                  }
             }
         }
         return 1
     }


    get_warp_motion_vector is a function call that indicates the get warp motion vector process specified in
    § 7.12.2.2 Get warp motion vector process is invoked.

```

<a id="s-6-19-7-13"></a>

##### § 6.19.7.13 Read motion mode semantics

```text
§   6.19.7.13. Read motion mode semantics

    use_extend_warp equal to 1 means that EXTENDWARP is used.

    use_local_warp equal to 1 means that LOCALWARP is used.

```

<a id="s-6-19-7-14"></a>

##### § 6.19.7.14 Read inter intra semantics

```text
§   6.19.7.14. Read inter intra semantics

    inter_intra equal to 1 specifies that an inter prediction is blended with an intra prediction.

    warp_inter_intra equal to 1 specifies that an inter prediction is blended with an intra prediction for a
    WARPMV block.




    AV2 Specification                                                                                Page 365 of 1169
    interintra_mode specifies the type of intra prediction to be used:

                                          Table 6.24: interintra_mode values and names

                        interintra_mode                                  Name of interintra_mode

                                 0                                             II_DC_PRED

                                 1                                              II_V_PRED

                                 2                                             II_H_PRED

                                 3                                          II_SMOOTH_PRED


    wedge_interintra equal to 1 specifies that wedge blending is used. wedge_interintra equal to 0 specifies
    that intra blending is used.

```

<a id="s-6-19-7-15"></a>

##### § 6.19.7.15 Read compound type semantics

```text
§   6.19.7.15. Read compound type semantics

    comp_group_idx equal to 0 indicates that the compound_type syntax element is not present and that an
    averaging scheme is used for blending. comp_group_idx equal to 1 indicates that the compound_type
    syntax element is present.

    compound_type specifies how the two predictions are blended together:

                        compound_type                                    Name of compound_type

                                 0                                         COMPOUND_WEDGE

                                 1                                        COMPOUND_DIFFWTD

                                 2                                        COMPOUND_AVERAGE

                                 3                                         COMPOUND_INTRA



      NOTE: COMPOUND_AVERAGE and COMPOUND_INTRA cannot be directly signaled with the
      compound_type syntax element but are inferred from other syntax elements.


    wedge_sign specifies the sign of the wedge blend.

    mask_type specifies the type of mask to be used during blending:

                        mask_type                                        Name of mask_type

                             0                                              UNIFORM_45

                             1                                            UNIFORM_45_INV


```

<a id="s-6-19-7-16"></a>

##### § 6.19.7.16 Read refine mv semantics

```text
§   6.19.7.16. Read refine mv semantics

    use_refinemv indicates that motion vector refinement is used for this block.

    DecidedAgainstRefinemv indicates that use_refinemv was originally set to 1 in the bitstream, but later
    cleared due to incompatible compound weights. In this case the reference code does not apply motion
    vector refinement, but uses a different interpolation filter.




    AV2 Specification                                                                              Page 366 of 1169
```

<a id="s-6-19-7-17"></a>

##### § 6.19.7.17 Read wedge mode semantics

```text
§   6.19.7.17. Read wedge mode semantics

    wedge_quad and wedge_angle are used to specify the wedge angle.

    wedge_dist1 specifies the distance to the wedge for angles where a distance of 0 is allowed.

    wedge_dist2 specifies the distance to the wedge for angles where a distance of 0 is not allowed.

    wedgeAngle gives the angle of the wedge as specified in Table 6.25:

                                       Table 6.25: wedgeAngle values and names

                        wedgeAngle                                 Name of wedgeAngle

                            0                                           WEDGE_0

                            1                                           WEDGE_14

                            2                                           WEDGE_27

                            3                                           WEDGE_45

                            4                                           WEDGE_63

                            5                                           WEDGE_90

                            6                                          WEDGE_117

                            7                                          WEDGE_135

                            8                                          WEDGE_153

                            9                                          WEDGE_166

                            10                                         WEDGE_180

                            11                                         WEDGE_194

                            12                                         WEDGE_207

                            13                                         WEDGE_225

                            14                                         WEDGE_243

                            15                                         WEDGE_270

                            16                                         WEDGE_297

                            17                                         WEDGE_315

                            18                                         WEDGE_333

                            19                                         WEDGE_346


```

<a id="s-6-19-7-18"></a>

##### § 6.19.7.18 MV semantics

```text
§   6.19.7.18. MV semantics

    MvCtx is used to determine which CDFs to use for the motion vector syntax elements.

    mv_joint specifies which components of the motion vector difference are non-zero:

           mv_joint                  Name of mv_joint             Changes row              Changes col

               0                      MV_JOINT_ZERO                   No                       No

               1                     MV_JOINT_HNZVZ                   No                       Yes

               2                     MV_JOINT_HZVNZ                   Yes                      No

               3                     MV_JOINT_HNZVNZ                  Yes                      Yes




    AV2 Specification                                                                          Page 367 of 1169
    The motion vector difference is added to the PredMvs to compute the final motion vector in BlockMvs.

    shell_set, shell_class, and joint_shell_last_two_classes are used to specify the class of the motion
    vector difference. A higher class means that the motion vector difference represents a larger update.

    shell_offset_low_class is used to compute shellClassOffset when shell_class is equal to 0 or 1.

    shell_offset_class2 and shell_offset_class2_high are used to compute shellClassOffset when
    shell_class is equal to 2.

    shell_offset_other_class is used to compute shellClassOffset when shell_class is greater than 2.

    col_mv_greater is used as part of a truncated unary coding for the variable col.

    col_remainder is used to increment the variable col if the maximum unary value has been reached.

    shellIndex is the sum of both motion vector components.

    col_mv_index specifies which component of the motion vector will be computed based on the known
    sum. The other component will be set equal to the variable col.

```

<a id="s-6-19-7-19"></a>

##### § 6.19.7.19 MV component semantics

```text
§   6.19.7.19. MV component semantics

    amvd_index is used to compute the size of the motion vector difference via a table lookup.

```

<a id="s-6-19-7-20"></a>

##### § 6.19.7.20 Compute prediction semantics

```text
§   6.19.7.20. Compute prediction semantics

    The prediction for inter and inter intra blocks is triggered within compute_prediction. However, intra
    prediction is done at the transform block granularity so predict_intra is also called from transform_block.

    predW and predH are variables containing the smallest size that can be used for inter prediction. (This
    size may be increased for chroma blocks if not all blocks use inter prediction.)

    predict_inter is a function call that indicates the conceptual point where inter prediction happens. When
    this function is called, the inter prediction process specified in § 7.13.3 Inter prediction process is
    invoked.

    predict_intra is a function call that indicates the conceptual point where intra prediction happens. When
    this function is called, the intra prediction process specified in § 7.13.2 Intra prediction process is
    invoked.

    wedge_mask is a function call that indicates the wedge mask process specified in § 7.13.3.27 Wedge
    mask process is invoked.

    intra_mode_variant_mask is a function call that indicates the intra mode variant mask process specified
    in § 7.13.3.29 Intra mode variant mask process is invoked.

    mask_blend is a function call that indicates the mask blend process specified in § 7.13.3.30 Mask blend
    process is invoked.




    AV2 Specification                                                                            Page 368 of 1169
      NOTE: The predict_inter, predict_intra, wedge_mask, intra_mode_variant_mask, mask_blend
      functions do not affect the syntax decode process. predict_inter does affect the SubMvs array which
      is used by the motion vector prediction process, but motion vector prediction is not required for
      syntax decode.


      NOTE: The chroma residual block size is always at least 4 in width and height. This means that no
      transform width or height smaller than 4 is required. As such, a chroma residual may actually cover
      several luma blocks.

```

<a id="s-6-19-7-21"></a>

##### § 6.19.7.21 Residual semantics

```text
§   6.19.7.21. Residual semantics

    The residual consists of a number of transform blocks.

    If the block is wider or higher than 64 luma samples, then the residual is split into 64 by 64 chunks.

```

<a id="s-6-19-7-22"></a>

##### § 6.19.7.22 Transform block semantics

```text
§   6.19.7.22. Transform block semantics

    reconstruct is a function call that indicates the conceptual point where inverse transform and
    reconstruction happens. When this function is called, the reconstruction process specified in § 7.14.3
    Reconstruct process is invoked.

    predict_palette is a function call that indicates the conceptual point where palette prediction happens.
    When this function is called, the palette prediction process specified in § 7.13.4 Palette prediction process
    is invoked.

    predict_chroma_from_luma is a function call that indicates the conceptual point where predicting
    chroma from luma happens. When this function is called, the predict chroma from luma process specified
    in § 7.13.5 Predict chroma from luma process is invoked.

    DeblockingTxSizes is an array that stores the transform size for each plane and position for use in
    deblocking filtering. DeblockingTxSizes[ plane ][ row ][ col ] stores the transform size where row and col
    are in units of 4x4 samples.


      NOTE:       The transform size is always equal for planes 1 and 2.

```

<a id="s-6-19-7-23"></a>

##### § 6.19.7.23 Coefficients semantics

```text
§   6.19.7.23. Coefficients semantics

    TxTypes is an array which stores at a 4x4 luma sample granularity the transform type to be used.


      NOTE: The transform type is only read for luma transform blocks, the chroma uses the transform
      type for a corresponding luma block. Chroma blocks will only use transform types that have been
      written for the current residual block.


    Quant is an array storing the quantised coefficients for the current transform block.

    It is a requirement of bitstream conformance that the values written into Quant are greater than -1 << 20
    and less than 1 << 20.

    QuantSign is an array storing the sign of the quantised coefficients for the current transform block, or
    zero for zero coefficients.


    AV2 Specification                                                                              Page 369 of 1169
  NOTE: It is possible for QuantSign[pos] to be not equal to zero when Quant[pos] is equal to zero as
  the quantised coefficients can wrap around.


all_zero equal to 1 specifies that all coefficients are zero.

eob_extra and eob_extra_bit specify the position of the last non-zero coefficient by being used to
compute the variable eob.

cctx_type specifies the angle for the cross component transform:

                    cctx_type                                      Name of cctx_type

                       0                                              CCTX_NONE

                       1                                               CCTX_45

                       2                                               CCTX_30

                       3                                               CCTX_60

                       4                                            CCTX_MINUS45

                       5                                            CCTX_MINUS30

                       6                                            CCTX_MINUS60


eob_pt_16, eob_pt_32, eob_pt_64, eob_pt_128, eob_pt_256, eob_pt_512, eob_pt_1024,
eob_pt_256_extra, eob_pt_512_extra, eob_pt_1024_extra: syntax elements used to compute eob.

It is a requirement of bitstream conformance that eob_pt_512_extra is not equal to 3.

eob is a variable that indicates the index of the end of block. This index is equal to one plus the index of
the last non-zero coefficient.

coeff_base_eob is a syntax element used to compute the base level of the last non-zero coefficient.


  NOTE:       The base level is set to coeff_base_eob plus 1 because this coefficient is known to be non-
  zero.


coeff_base_bob is a syntax element used to compute the base level of the first non-zero coefficient.

coeff_base specifies the base level of a coefficient.

coeff_base_idtx specifies the base level of a coefficient when using forward skip coding.

idtx_sign specifies the sign of the coefficients when using forward skip coding.

dc_sign specifies the sign of the DC coefficient.

dc_sign_horz_vert specifies the sign of the DC coefficients when using horizontal or vertical transform
classes.

sign_bit specifies the sign of a non-zero AC coefficient.

coeff_br specifies an increment to the coefficient.

coeff_br_idtx specifies an increment to the coefficient when using forward skip coding.


AV2 Specification                                                                               Page 370 of 1169
    AboveLevelContext and LeftLevelContext are arrays that store at a 4 sample granularity the
    cumulative sum of coefficient levels.

    AboveDcContext and LeftDcContext are arrays that store at a 4 sample granularity 2 bits signaling the
    sign of the DC coefficient (zero being counted as a separate sign).

```

<a id="s-6-19-7-24"></a>

##### § 6.19.7.24 Read quantized coefficient semantics

```text
§   6.19.7.24. Read quantized coefficient semantics

    q_length_bit is used to specify the prefix of the extra bits required to code the coefficient.

    golomb_length_bit is used to compute the number of extra bits required to code the coefficient.

    If length is equal to 20, it is a requirement of bitstream conformance that golomb_length_bit is equal to 1.

    coeff_rem specifies the values of the extra bits.

```

<a id="s-6-19-7-25"></a>

##### § 6.19.7.25 Read CFL alphas semantics

```text
§   6.19.7.25. Read CFL alphas semantics

    cfl_mhccp and cfl_index specify how the chroma from luma parameters are prepared:

                                         Table 6.26: cfl_index values and names

                    cfl_index                                          Name of cfl_index

                        0                                               CFL_EXPLICIT

                        1                                             CFL_DERIVED_ALPHA

                        2                                                 CFL_MULTI


    cfl_mh_dir specifies a direction used by MHCCP.

    cfl_alpha_signs contains the sign of the alpha values for U and V packed together into a single syntax
    element with 8 possible values as specified in Table 6.27: (The combination of two zero signs is prohibited
    as it is redundant with DC intra prediction.)

                                Table 6.27: cfl_alpha_signs values and sign interpretations

                 cfl_alpha_signs                      Name of signU                        Name of signV

                            0                        CFL_SIGN_ZERO                         CFL_SIGN_NEG

                            1                        CFL_SIGN_ZERO                         CFL_SIGN_POS

                            2                         CFL_SIGN_NEG                         CFL_SIGN_ZERO

                            3                         CFL_SIGN_NEG                         CFL_SIGN_NEG

                            4                         CFL_SIGN_NEG                         CFL_SIGN_POS

                            5                         CFL_SIGN_POS                         CFL_SIGN_ZERO

                            6                         CFL_SIGN_POS                         CFL_SIGN_NEG

                            7                         CFL_SIGN_POS                         CFL_SIGN_POS


    signU contains the sign of the alpha value for the U component:

                    signU                                              Name of signU

                        0                                              CFL_SIGN_ZERO




    AV2 Specification                                                                                Page 371 of 1169
                            1                                      CFL_SIGN_NEG

                            2                                      CFL_SIGN_POS


    signV contains the sign of the alpha value for the V component with the same interpretation as for signU.

    cfl_alpha_u contains the absolute value of alpha minus one for the U component.

    cfl_alpha_v contains the absolute value of alpha minus one for the V component.

    CflAlphaU contains the signed value of the alpha component for the U component.

    CflAlphaV contains the signed value of the alpha component for the V component.

```

<a id="s-6-19-8"></a>

#### § 6.19.8 Coding tools structures

```text
§   6.19.8. Coding tools structures

```

<a id="s-6-19-8-1"></a>

##### § 6.19.8.1 Palette mode info semantics

```text
§   6.19.8.1. Palette mode info semantics

    has_palette_y is a boolean value specifying whether a palette is encoded for the Y plane.

    palette_size_y_minus_2 is used to compute PaletteSizeY.

    PaletteSizeY is a variable holding the Y plane palette size.

    use_palette_color_cache_y, if equal to 1, indicates that for a particular palette entry in the luma palette,
    the cached entry is used.

    palette_colors_y is an array holding the Y plane palette colors.

    palette_num_extra_bits_y is used to calculate the number of bits used to store each palette delta value
    for the luma palette.

    palette_delta_y is a delta value for the luma palette.

```

<a id="s-6-19-8-2"></a>

##### § 6.19.8.2 Transform type semantics

```text
§   6.19.8.2. Transform type semantics

    set specifies the transform set.

                 is_inter              set                          Name of transform set

                Don’t care             0                               TX_SET_DCTONLY

                Don’t care             1                               TX_SET_WIDE_64

                Don’t care             2                               TX_SET_HIGH_64

                Don’t care             3                               TX_SET_WIDE_32

                Don’t care             4                               TX_SET_HIGH_32

                        0              5                               TX_SET_INTRA_1

                        0              6                               TX_SET_INTRA_2

                        1              5                               TX_SET_INTER_1

                        1              6                               TX_SET_INTER_2

                        1              7                               TX_SET_DCT_IDTX

                        1              8                           TX_SET_DCT_IDTX_IDDCT




    AV2 Specification                                                                             Page 372 of 1169
    lossless_inter_tx_type is used to specify the transform type for 4 by 4 lossless inter transform blocks.

    is_long_side_dct equal to 1 specifies that the long side of a block uses Discrete Cosine Transform (DCT).
    is_long_side_dct equal to 0 specifies that the long side uses an alternative transform.

    inter_tx_type and inter_tx_type_offset specify the transform type for inter blocks.

    intra_tx_type is used in the computation of the transform type for intra blocks. The transform type
    depends on intra_tx_type and the intra direction for the block.

    sec_tx_type specifies the secondary transform type.

    most_probable_stx_set is used to compute the kernel used for the secondary transform.

```

<a id="s-6-19-8-3"></a>

##### § 6.19.8.3 Palette tokens semantics

```text
§   6.19.8.3. Palette tokens semantics

    palette_direction equal to 0 specifies that the palette is read row by row. palette_direction equal to 1
    specifies that the palette is read column by column.

    identity_row_y equal to 0 specifies that each sample is coded individually. identity_row_y equal to 1
    specifies that each line of luma samples in the block contains a constant color. identity_row_y equal to 2
    specifies that each line is copied from the previous line.

    It is a requirement of bitstream conformance that i is greater than 0 if identity_row_y is equal to 2.


      NOTE: When palette direction is equal to 0, the lines mentioned in identity_row_y refer to rows.
      When direction is equal to 1, the lines refer to columns.


    color_index_map_y holds the index in palette_colors_y for the block’s Y plane top left sample.

    palette_color_idx_y holds the index in ColorOrder for a sample in the block’s Y plane.

```

<a id="s-6-19-8-4"></a>

##### § 6.19.8.4 Palette color context function semantics

```text
§   6.19.8.4. Palette color context function semantics

    ColorOrder is an array holding the mapping from an encoded index to the palette. ColorOrder is ranked
    in order of frequency of occurrence of each color in the neighborhood of the current block, weighted by
    closeness to the current block.

    ColorContextHash is a variable derived from the distribution of colors in the neighborhood of the
    current block, which is used to determine the probability context used to decode palette_color_idx_y and
    palette_color_idx_uv.

```

<a id="s-6-19-9"></a>

#### § 6.19.9 Filtering structures

```text
§   6.19.9. Filtering structures

```

<a id="s-6-19-9-1"></a>

##### § 6.19.9.1 Read CDEF semantics

```text
§   6.19.9.1. Read CDEF semantics

    cdef_idx specifies which CDEF filtering parameters are used for a particular 64 by 64 block. A value of -1
    means that CDEF is disabled for that block.

    cdef_index0 specifies that cdef_idx is equal to 0.

    cdef_index_minus_1 plus 1 specifies the value of cdef_idx.




    AV2 Specification                                                                              Page 373 of 1169
```

<a id="s-6-19-9-2"></a>

##### § 6.19.9.2 Read CCSO semantics

```text
§   6.19.9.2. Read CCSO semantics

    ccso_blk equal to 1 specifies that CCSO filtering is enabled for a particular plane and CCSO block.
    ccso_blk equal to 0 specifies that CCSO is disabled for that block.

```

<a id="s-6-19-9-3"></a>

##### § 6.19.9.3 Read GDF semantics

```text
§   6.19.9.3. Read GDF semantics

    use_gdf equal to 1 specifies that Guided Detail Filter (GDF) is enabled for a particular block. use_gdf
    equal to 0 specifies that GDF is disabled for that block.

```

<a id="s-6-19-9-4"></a>

##### § 6.19.9.4 Read loop restoration semantics

```text
§   6.19.9.4. Read loop restoration semantics

    This contains syntax for any new restoration units that are covered.

```

<a id="s-6-19-9-5"></a>

##### § 6.19.9.5 Read loop restoration unit semantics

```text
§   6.19.9.5. Read loop restoration unit semantics

    use_wiener_ns equal to 1 specifies that the non-separable Wiener filter is used for loop restoration.
    use_wiener_ns equal to 0 specifies that the non-separable Wiener filter is not used.

    use_pc_wiener equal to 1 specifies that the pixel classified Wiener filter is used for loop restoration.
    use_pc_wiener equal to 0 specifies that the pixel classified filter is not used.

    flex_restoration_type equal to 1 specifies that a particular enabled loop restoration tool is used for the
    restoration unit. flex_restoration_type equal to 0 specifies that the restoration tool is not used for this
    unit.

```

<a id="s-6-19-9-6"></a>

##### § 6.19.9.6 Read Wiener NS semantics

```text
§   6.19.9.6. Read Wiener NS semantics

    matchIndices is used to determine the reference values for the Wiener coefficients.

    use_alt_group equal to 0 specifies that the predicted group is used. use_alt_group equal to 1 specifies
    that a different group to the predicted group is used.

    group_bit is used when there is more than one alternative group.

    merged_param equal to 1 specifies that a previous set of parameters is used for loop restoration.
    merged_param equal to 0 specifies that new parameters are signaled for this restoration unit.

    use_bank indicates that a particular bank of parameters is used for loop restoration.

    wiener_ns_length is used to compute the number of coefficients to read.

    wiener_ns_uv_sym equal to 1 specifies that the chroma filter is symmetric and fewer coefficients need to
    be signaled. wiener_ns_uv_sym equal to 0 specifies that the chroma filter is asymmetric and all
    coefficients are signaled.

    wiener_ns_base is used to compute the base level of a coefficient.

    wiener_ns_rem is used to provide an increment for a coefficient.

                                                                                       ↑ Back to Table of Contents




    AV2 Specification                                                                              Page 374 of 1169
```
