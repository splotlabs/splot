# AV2 v1.0.0 — Annex A: Profiles, levels, and tiers

<!-- Verbatim mirror of the AOM AV2 v1.0.0 specification (© Alliance for Open Media). The PDF is normative; this is a faithful `pdftotext -layout` copy. See [./README.md](./README.md) and [./index.md](./index.md). Do not hand-edit: regenerate via scripts/spec/regenerate-av2-spec.sh. -->

<a id="s-annex-a"></a>

## Annex A: Profiles, levels, and tiers

```text
§   Annex A: Profiles, levels, and tiers
§   A.1.General
    This annex specifies profiles, levels, and tiers that collectively define the conformance requirements for
    AV2 bitstreams and decoders.

    A profile specifies the allowed coding tools, chroma formats, and bit depths that a conforming coded
    video sequence or coded multistream video sequence shall satisfy. Further information is provided in
    Annex A.2 Profiles.

    A level and tier combination defines constraints on picture size, display rate, decoding rate, and bitrate
    that a conforming coded video sequence or coded multistream video sequence shall not exceed. Further
    information is provided in Annex A.4 Levels.

§   A.2.Profiles
    The AV2 profiles supported in this version of this specification are defined in Table A.1. A profile specifies
    the allowed coding tools, chroma formats, bit depths, and interoperability point that a conforming coded
    video sequence or coded multistream video sequence shall satisfy. An interoperability point indicates the
    layering capabilities of the bitstream, and it is explicitly determined by the profile identifier for all
    profiles except the Configurable profile. The Configurable profile indicates that a bitstream does not
    conform to any of the other defined profiles, and additional information is needed to determine its
    constraints.

    Decoders are required to support one or more profiles to claim conformance with the AV2 video coding
    standard.


      NOTE: This version of this specification specifies one toolset, the Main toolset. This includes all
      coding tools defined in this specification. Future versions of this specification may define additional
      toolsets using the extensibility mechanisms of AV2.


    A coded video sequence signals its profile via seq_profile_idc in the associated sequence header. A coded
    multistream video sequence may signal its aggregate profile via multistream_profile_idc in the MSDO
    OBU. Both use the same value space, as specified in Table A.1.

                                            Table A.1: AV2 profile definitions

       Profile label      seq_profile_idc or             chroma_format_idc         bit_depth_idc   Interoperability
                        multistream_profile_idc                                                         point

     Main_420_10_IP0    0                         CHROMA_FORMAT_400,               0 or 1          0
                                                  CHROMA_FORMAT_420

     Main_420_10_IP1    1                         CHROMA_FORMAT_400,               0 or 1          1
                                                  CHROMA_FORMAT_420

     Main_420_10_IP2    2                         CHROMA_FORMAT_400,               0 or 1          2
                                                  CHROMA_FORMAT_420

     Main_422_10_IP1    3                         CHROMA_FORMAT_400,               0 or 1          1
                                                  CHROMA_FORMAT_420,
                                                  CHROMA_FORMAT_422




    AV2 Specification                                                                                  Page 1101 of 1169
 Main_444_10_IP1        4                          CHROMA_FORMAT_400,                       0 or 1              1
                                                   CHROMA_FORMAT_420,
                                                   CHROMA_FORMAT_444

 Reserved               5-30

 Configurable           31                         CHROMA_FORMAT_400,                       -                   -
                                                   CHROMA_FORMAT_420,
                                                   CHROMA_FORMAT_422,
                                                   CHROMA_FORMAT_444


For example, if seq_profile_idc is equal to 3, the coded video sequence conforms to the
"Main_422_10_IP1" profile at Interoperability Point 1, and may use chroma formats 4:0:0, 4:2:0, or 4:2:2
at 8 or 10 bit depth. Similarly, if multistream_profile_idc is equal to 3, the coded multistream video
sequence conforms to the same profile and interoperability point.

For the Configurable profile, the constraints are determined from the chroma_format_idc, bit_depth_idc,
and SeqMaxMlayerCnt syntax elements in the sequence header. Additionally, the multi-sequence
configuration signaling described in Annex A.3 Multi-sequence configurations may be used to convey the
aggregate constraints of a bitstream using the Configurable profile.

The variables ProfileScalingFactor, PicSizeProfileFactor, and BitrateProfileFactor are derived from the
profile as defined in Table A.2 and are used in the level and tier constraints specified in Annex A.4 Levels
and Annex E: Decoder model. For the Configurable profile, ProfileScalingFactor and the related variables
need to be determined based on the characteristics of the chosen configuration.

        Table A.2: Definition of ProfileScalingFactor, PicSizeProfileFactor, and BitrateProfileFactor

          seq_profile_idc or                ProfileScalingFactor         PicSizeProfileFactor              BitrateProfileFactor
        multistream_profile_idc

 0, 1, 2                                0                           15                               1.0

 3                                      1                           20                               1.667

 4                                      2                           30                               2.5

 31                                     -                           -                                -


Interoperability points are defined in Table A.3. An interoperability point specifies the number of
extended and embedded layers a decoder is capable of decoding simultaneously.

                                            Table A.3: AV2 interoperability points

     Interoperability             Number of             Number of             Combination of Extended and             Number of
          Point                Extended Layers        Embedded Layers             Embedded Layers                      Layers

 0                           1-4                  1                       0                                          1-4

 1                           1-4                  1-2                     0                                          1-4

 2                           1-4                  1-3                     0 or 1                                     1-8

 3-14                        Reserved

 15 (max)                    1-31                 1-8                     0 or 1                                     1-248




AV2 Specification                                                                                                   Page 1102 of 1169
where the columns in the table are defined as follows:

  • Number of Extended Layers denotes the number of singlestreams in a coded video sequence or
    coded multistream video sequence. For a coded video sequence, this value is equal to 1. For a coded
    multistream video sequence, when MultiStreamDecoderMode is equal to 1, this value is equal to
    num_streams_minus_2 plus 2. When a global layer configuration record is activated, this value is
    equal to LcrMaxNumXLayerCount. Otherwise, this value is equal to the number of distinct values of
    obu_xlayer_id (excluding GLOBAL_XLAYER_ID) present in the coded multistream video sequence.
  • Number of Embedded Layers denotes the maximum value for seq_max_mlayer_cnt_minus_1 plus 1
    for the coded video sequence or coded multistream video sequence.
  • Combination of Extended and Embedded Layers denotes if a coded video sequence or coded
    multistream video sequence contains more than one extended layer and more than one embedded
    layer. This value is equal to 1 when Number of Extended Layers and Number of Embedded Layers are
    both greater than one. Otherwise, the value is equal to 0. For a coded video sequence, this value is
    equal to 0.
  • Number of Layers denotes the sum of seq_max_mlayer_cnt_minus_1 plus 1 across all singlestreams
    in a coded multistream video sequence. For a coded video sequence, this value is equal to
    seq_max_mlayer_cnt_minus_1 plus 1.


  NOTE: A coded multistream video sequence that contains two extended layers, where the first
  extended layer contains two embedded layers and the second extended layer contains three
  embedded layers, will have "Number of Extended Layers" equal to 2, "Number of Embedded Layers"
  equal to 3, "Combination of Extended and Embedded Layers" equal to 1, and "Number of Layers"
  equal to 5. A coded video sequence that contains two embedded layers will have "Number of
  Extended Layers" equal to 1, "Number of Embedded Layers" equal to 2, "Combination of Extended
  and Embedded Layers" equal to 0, and "Number of Layers" equal to 2.


For interoperability points 0 through 2, requirements on the presence of OBUs with obu_type equal to
OBU_MSDO (MSDO) and obu_type equal to OBU_LAYER_CONFIGURATION_RECORD (LCR) are given in
the Table A.4. The OBU with obu_type equal to OBU_OPERATING_POINT_SET is optional in all of these
cases.

                         Table A.4: OBU requirements for interoperability points

 IOP       Number of Extended       Number of Embedded        MSDO                             LCR
              Layers > 1                Layers > 1

 0     N                        N/A                      Prohibited         Optional

 0     Y                        N/A                      Required           Optional

 1     N                        N                        Prohibited         Optional

 1     Y                        N                        Required           Optional

 1     N                        Y                        Prohibited         Required (Local)

 2     N                        N                        Prohibited         Optional

 2     Y                        N                        One (or both) of (a) MSDO or (b) Global LCR is required

 2     N                        Y                        Prohibited         Required (Global or Local)

 2     Y                        Y                        One (or both) of (a) MSDO plus Local LCR or (b) Global
                                                         LCR is required




AV2 Specification                                                                                    Page 1103 of 1169
§   A.3.Multi-sequence configurations
    A multi-sequence configuration specifies the collective minimum requirements for coding tools,
    chroma formats, and bit depths needed to decode all coded video sequences within an AV2 bitstream.
    Multi-sequence configurations are particularly relevant for bitstreams using the Configurable profile (see
    Annex A.2 Profiles), where they provide a mechanism to convey the aggregate constraints that are not
    otherwise determined by the profile identifier.

    This specification defines three multi-sequence configurations: "C_Main_420_10", "C_Main_422_10", and
    "C_Main_444_10", as listed in Table A.5. A bitstream can explicitly identify its multi-sequence
    configuration through the lcr_config_idc syntax elements in a LCR OBU, if one is present. Alternatively,
    this information may be implicitly determined from syntax elements within the bitstream, such as the
    chroma_format_idc and bit_depth_idc of each individual coded video sequence.

                                        Table A.5: AV2 multi-sequence configurations

         ConfigurationID       Multi-sequence configuration label      Toolset     BitDepth             Chroma Format

                                                                                   8       10   4:0:0    4:2:0    4:2:2   4:4:4

     0                      C_Main_420_10                             Main         x   x        x       x

     1                      C_Main_422_10                             Main         x   x        x       x         x

     2                      C_Main_444_10                             Main         x   x        x       x                 x

     3-63                   Reserved



                     Table A.6: Allowed syntax element values for multi-sequence configurations

              Multi-sequence           seq_profile_idc                  chroma_format_idc                          bit_depth_idc
            configuration label

     C_Main_420_10                     0..2, 31          CHROMA_FORMAT_400, CHROMA_FORMAT_420                      0..1

     C_Main_422_10                     0..3, 31          CHROMA_FORMAT_400, CHROMA_FORMAT_420,                     0..1
                                                         CHROMA_FORMAT_422

     C_Main_444_10                     0..2, 4, 31       CHROMA_FORMAT_400, CHROMA_FORMAT_420,                     0..1
                                                         CHROMA_FORMAT_444


§   A.4.Levels
    Each operating point contains a syntax element seq_level_idx.

    The following table defines the mapping from the syntax element (which takes integer values) to the
    defined levels:

                                                     Table A.7: Values for level

                           Value of seq_level_idx                                               Level

     0                                                                2.0

     1                                                                2.1

     2                                                                3.0

     3                                                                3.1

     4                                                                4.0




    AV2 Specification                                                                                            Page 1104 of 1169
 5                                                              4.1

 6                                                              5.0

 7                                                              5.1

 8                                                              5.2

 9                                                              5.3

 10                                                             6.0

 11                                                             6.1

 12                                                             6.2

 13                                                             6.3

 14                                                             7.0

 15                                                             7.1

 16                                                             7.2

 17                                                             7.3

 18                                                             8.0

 19                                                             8.1

 20                                                             8.2

 21                                                             8.3

 22-30                                                          Reserved

 31                                                             Maximum parameters


The level defines variables as specified in the following tables:

                                           Table A.8: Values for level

     LevelIdx       Level    MaxPicSize         MaxHSize/MaxVSize          MaxDisplayRate     MaxDecodeRate

                              (Samples)             (Samples)              (Samples/sec)       (Samples/sec)

 0              2.0         147456        640                          4423680              5529600

 1              2.1         278784        880                          8363520              10454400

 2              3.0         665856        1360                         19975680             24969600

 3              3.1         1065024       1720                         31950720             39938400

 4              4.0         2359296       2560                         70778880             77856768

 5              4.1         2359296       2560                         141557760            155713536

 6              5.0         8912896       4975                         267386880            273715200

 7              5.1         8912896       4975                         534773760            547430400

 8              5.2         8912896       4975                         1069547520           1094860800

 9              5.3         8912896       4975                         1069547520           1176502272

 10             6.0         35651584      9951                         1069547520           1176502272

 11             6.1         35651584      9951                         2139095040           2189721600

 12             6.2         35651584      9951                         4278190080           4379443200

 13             6.3         35651584      9951                         4278190080           4706009088

 14             7.0         142606336     19902                        4278190080           4706009088

 15             7.1         142606336     19902                        8556380160           8758886400



AV2 Specification                                                                                Page 1105 of 1169
 16               7.2         142606336           19902                          17112760320              17517772800

 17               7.3         142606336           19902                          17112760320              18824036352

 18               8.0         530841600           38400                          17112760320              18824036352

 19               8.1         530841600           38400                          34225520640              34910031052

 20               8.2         530841600           38400                          68451041280              69820062105

 21               8.3         530841600           38400                          68451041280              75296145408



                                           Table A.9: Level bitrate and tile constraints

 LevelIdx    Level      MaxHeaderRate     MainMbps        HighMbps      MainCR   HighCR   MaxTiles   MaxTileCols          Example

                              (/sec)       (MBits/            (MBits/
                                            sec)               sec)

 0          2.0         150               1.5             -             2        -        8          4              426x240@30fps

 1          2.1         150               3.0             -             2        -        8          4              640x360@30fps

 2          3.0         150               6.0             -             2        -        16         6              854x480@30fps

 3          3.1         150               10.0            -             2        -        16         6              1280x720@30fps

 4          4.0         300               12.0            30.0          4        4        32         8              1920x1080@30fps

 5          4.1         300               20.0            50.0          4        4        32         8              1920x1080@60fps

 6          5.0         300               30.0            100.0         6        4        64         8              3840x2160@30fps

 7          5.1         300               40.0            160.0         8        4        64         8              3840x2160@60fps

 8          5.2         300               60.0            240.0         8        4        64         8              3840x2160@120fps

 9          5.3         300               60.0            240.0         8        4        64         8              3840x2160@120fps

 10         6.0         300               60.0            240.0         8        4        128        16             7680x4320@30fps

 11         6.1         300               100.0           480.0         8        4        128        16             7680x4320@60fps

 12         6.2         300               160.0           800.0         8        4        128        16             7680x4320@120fps

 13         6.3         300               160.0           800.0         8        4        128        16             7680x4320@120fps

 14         7.0         960               160.0           800.0         8        4        256        32             15360x8640@30fps

 15         7.1         960               200.0           960.0         8        4        256        32             15360x8640@60fps

 16         7.2         960               320.0           1600.0        8        4        256        32             15360x8640@120fps

 17         7.3         960               320.0           1600.0        8        4        256        32             15360x8640@120fps

 18         8.0         960               320.0           1600.0        8        4        512        64             30720x17280@30fps

 19         8.1         960               400.0           1920.0        8        4        512        64             30720x17280@60fps

 20         8.2         960               640.0           3200.0        8        4        512        64             30720x17280@120fps

 21         8.3         960               640.0           3200.0        8        4        512        64             30720x17280@120fps



  NOTE: HighMbps and HighCR values are not defined for levels below level 4.0. seq_tier equal to 1
  can only be signaled for level 4.0 and above.


Bitstream constraints shall be applied at the bitstream level and shall correspond to the tier ID seq_tier
and level ID seq_level_idx signaled in the sequence_header_obu().

A bitstream may contain one or more operating points. It can also represent a sub-bitstream extracted
from a source bitstream containing multiple operating points, based on the operating point indication. In


AV2 Specification                                                                                              Page 1106 of 1169
the latter case, the sub-bitstream may signal different values of the tier ID seq_tier and level ID
seq_level_idx in the sequence_header_obu(), which may be derived from the corresponding ops_tier_flag
and ops_level_idx values signaled in the operating_point_set_obu(). Bitstream constraints shall be applied
to the sub-bitstream according to its own seq_tier and seq_level_idx values.

If MultiStreamDecoderMode is equal to 0, bitstream constraints shall be applied to each substream in the
bitstream according to the seq_tier and seq_level_idx values associated with that substream.

Otherwise, if MultiStreamDecoderMode is equal to 1, the syntax elements
multistream_even_allocation_flag, multistream_large_picture_idc, multistream_level_idx,
multistream_tier, num_streams_minus_2, and sub_xlayer_id[ i ] refer to the values from the most recently
parsed Multi Stream Decoder Operation OBU. The substream level variables MaxPicSizeX, MaxMbpsX,
MaxDisplayRateX, MaxDecodeRateX, MaxHeaderRateX, MaxTilesX, MaxTileColsX and MinCompBasisX
for the bitstream associated with obu_xlayer_id are derived by using the following ordered steps:

 1. The variable ScaleFactorX is derived by:

       ◦ If multistream_even_allocation_flag is equal to 1, ScaleFactorX is set to 4.
       ◦ Otherwise, if multistream_even_allocation_flag is equal to 0 and the obu_xlayer_id value
         associated with the current subbitstream is equal to
         sub_xlayer_id[ multistream_large_picture_idc ], then the ScaleFactorX for that subbitstream is set
         to 1.5.
       ◦ Otherwise (multistream_even_allocation_flag is equal to 0 and the obu_xlayer_id value associated
         with the current subbitstream is not equal to sub_xlayer_id[ multistream_large_picture_idc ]),
         ScaleFactorX is set to 9.
 2. Let MaxPicSize, MaxDisplayRate and MaxDecodeRate, MaxHeaderRate, MainMbps, HighMbps,
    MainCR, HighCR, MaxTiles and MaxTileCols be level variables in the table associated with
    multistream_level_idx. The values for the substream-level variables, MaxVSizeX, MaxHSizeX,
    MaxTileColsX, and MaxHeaderRateX, are determined by looking up the table below, using
    MaxPicSize and ScaleFactorX.

   MaxPicSize           ScaleFactorX    MaxVSizeX     MaxHSizeX          MaxTileColsX     MaxHeaderRateX

 2359296            1.5                1600         896             7                   132

 2359296            4                  960          576             4                   132

 2359296            9                  640          384             3                   132

 8912896            1.5                2560         1472            7                   132

 8912896            4                  1920         1088            4                   132

 8912896            9                  1280         768             3                   132

 35651584           1.5                5120         2280            13                  132

 35651584           4                  3840         2176            8                   132

 35651584           9                  2560         1472            5                   132

 142606336          1.5                10240        5760            26                  132

 142606336          4                  7680         4320            16                  132

 142606336          9                  5120         2880            11                  132

 530841600          1.5                20480        11520           52                  132

 530841600          4                  15360        8640            32                  132



AV2 Specification                                                                             Page 1107 of 1169
 530841600          9              10240           5760            21                 132


 1. The values for the remaining substream level variables MaxPicSizeX, MaxMbpsX, MaxDisplayRateX,
    MaxDecodeRateX, MaxTilesX, MaxTileColsX, and MinCompBasisX are set as follows:

       ◦ MaxPicSizeX = MaxVSizeX * MaxHSizeX
       ◦ MaxMbpsX = multistream_tier == 0 ? (MainMbps / ScaleFactorX) : (HighMbps/ScaleFactorX)
       ◦ MaxDisplayRateX = MaxDisplayRate / ScaleFactorX
       ◦ MaxDecodeRateX = MaxDecodeRate / ScaleFactorX
       ◦ MaxTilesX = MaxTiles / ScaleFactorX
       ◦ MinCompBasisX = multistream_tier == 0 ? MainCR : HighCR.

Let MaxPicSize, MaxDisplayRate, MaxDecodeRate, MaxHeaderRate, MainMbps, HighMbps, MainCR,
HighCR, MaxTiles and MaxTileCols be level variables in the table associated with seq_level_idx, the
additional variables are derived as follows:

  • TileWidth is defined as (MiColEnd - MiColStart) * MI_SIZE
  • TileHeight is defined as (MiRowEnd - MiRowStart) * MI_SIZE
  • RightMostTile is defined as MiColEnd == MiCols
  • MaxTileSizeInLumaSamples is defined as the largest product of TileWidth * TileHeight for all tiles
    within the coded video sequence
  • InloopFilteringEnabled for a particular Frame is set equal to 1 if apply_deblocking_filter[ 0 ] != 0 ||
    apply_deblocking_filter[ 1 ] != 0 || cdef_frame_enable != 0 || ccso_frame_flag != 0 || ccso_planes[ 0 ] !
    = 0 || ccso_planes[ 1 ] != 0 || ccso_planes[ 2 ] != 0 || FrameRestorationType[ 0 ] != RESTORE_NONE
    || FrameRestorationType[ 1 ] != RESTORE_NONE || FrameRestorationType[ 2 ] != RESTORE_NONE
    || gdf_frame_enable != 0. Otherwise, it is set equal to 0.
  • DecodeCount for a particular Frame is set equal to 2 if both allow_global_intrabc is equal to 1 and
    InloopFilteringEnabled is equal to 1. Otherwise, it is set equal to 1.
  • LumaSampleCount for a particular Frame is determined as follows:

       ◦ If (FrameIsIntra) LumaSampleCount is set equal to FrameWidth * FrameHeight.
       ◦ Otherwise, LumaSampleCount is set equal to (max_frame_width_minus_1 + 1) *
         (max_frame_height_minus_1 + 1).
  • The output time of a temporal unit is defined as the time indicated through either the timing
    information OBU, if present, or the timing information that may be indicated through external means.
    The output duration of a temporal unit is defined as the difference between the output time of the
    next temporal unit and the output time of the current temporal unit in display order. For the last
    temporal unit in the bitstream, the output duration from the previous temporal unit is used.
  • TotalDisplayLumaSampleRate is defined as the sum of the LumaSampleCount of all frames with
    immediate_output_frame equal to 1 or implicit_output_frame equal to 1 or ShowExistingFrame equal
    to 1 that belong to the temporal unit, divided by the output duration of the temporal unit.
  • FrameParsingTime for a Frame belonging to Decodable Frame Group (DFG) i and with
    ShowExistingFrame equal to 0 is defined as (Removal[i+1] – Removal[i]) ÷ DecodeCount if
    Removal[i+1] is present. For the Frame belonging to the last DFG in the bitstream,



AV2 Specification                                                                             Page 1108 of 1169
    FrameParsingTime shall be set equal to that of the previous Frame with ShowExistingFrame equal to
    0. The DFG is defined in Section Annex E.3 Decoder model definitions, and the ith DFG removal time
    Removal[i] is defined in Section Annex E.5.4 Removal times in decoding schedule mode.
  • MaxNumFrameHeadersPerSec is set equal to MaxHeaderRate * (1 + (seq_tier<<1))
  • NumFrameHeadersPerSec is defined as the number of OBUs received per second that contain a
    frame_header() and for which the variable CountFrameHeaderForLevelConstraint is equal to 1.
  • CompressedSize is defined for each frame as the total bytes in the OBUs, with obu_type equal to
    OBU_CLOSED_LOOP_KEY, OBU_OPEN_LOOP_KEY, OBU_LEADING_TILE_GROUP,
    OBU_REGULAR_TILE_GROUP, OBU_METADATA_SHORT, OBU_METADATA_GROUP, OBU_SWITCH,
    OBU_LEADING_SEF, OBU_REGULAR_SEF, OBU_LEADING_TIP, OBU_REGULAR_TIP,
    OBU_BRIDGE_FRAME or OBU_RAS_FRAME, related to this frame, minus 128 (to allow for overhead
    of metadata and header data).
  • FrameSymbolCount is defined for each frame as the total number of symbols in the OBUs related to
    this Frame. It is initialized to 0 in the syntax table frame_header(), and accumulated for the frame in
    the parsing process as defined in read_literal(n) and read_symbol(cdf).
  • If seq_tier is equal to 0, MaxMbps is set equal to MainMbps, otherwise MaxMbps is set equal to
    HighMbps.
  • If seq_tier is equal to 0, MinCompBasis is set equal to MainCR, otherwise MinCompBasis is set equal
    to HighCR.

When MultiStreamDecoderMode is equal to 1, the level variables are adjusted as follows:

  • MaxPicSize = Min(MaxPicSize, MaxPicSizeX)
  • MaxMbps = Min(MaxMbps, MaxMbpsX)
  • MaxDisplayRate = Min(MaxDisplayRate, MaxDisplayRateX)
  • MaxDecodeRate = Min(MaxDecodeRate, MaxDecodeRateX)
  • MaxVSize = Min(MaxVSize, MaxVSizeX)
  • MaxHSize = Min(MaxHSize, MaxHSizeX)
  • MaxHeaderRate = Min(MaxHeaderRate, MaxHeaderRateX)
  • MaxTiles = Min(MaxTiles, MaxTilesX)
  • MaxTileCols = Min(MaxTileCols, MaxTileColsX)
  • MinCompBasis = Max(MinCompBasis, MinCompBasisX).

The additional variable MaxLevelRefFrames is derived as follows:

  • If the bitstream contains any frame with DecodeCount equal to 2 and satisfies one of the following
    conditions, MaxLevelRefFrames is set to Min((8 * MaxPicSize) / ((max_frame_width_minus_1 + 1) *
    (max_frame_height_minus_1 + 1)) - 1, (8 << explicit_num_ref_frames)):

       ◦ max_mlayer_id is not equal to 0,
       ◦ at least one of such frames is not coded using OBUs with obu_type equal to
         OBU_CLOSED_LOOP_KEY.
  • Otherwise, MaxLevelRefFrames is set to Min((8 * MaxPicSize) / ((max_frame_width_minus_1 + 1) *
    (max_frame_height_minus_1 + 1)), (8 << explicit_num_ref_frames)).


AV2 Specification                                                                            Page 1109 of 1169
  NOTE: MaxLevelRefFrames in the case of DecodeCount equal to 2, e.g., a frame is encoded with
  both InloopFilteringEnabled and allow_global_intrabc equal to 1, is lowered by 1 to reserve memory
  space in a reference frame buffer that may be used for the reconstruction of the intermediate
  decoded frame associated with this coded frame and prior to the application of any loop filtering
  operations.


When the mapped level ID, LevelIdx is contained in the tables above, it is a requirement of bitstream
conformance that the following constraints hold:

  • FrameWidth * FrameHeight is less than or equal to MaxPicSize
  • FrameWidth is less than or equal to MaxHSize
  • FrameHeight is less than or equal to MaxVSize
  • NumTiles is less than or equal to MaxTiles
  • TileCols is less than or equal to MaxTileCols
  • TileWidth is less than or equal to Tile_Width_Scaling_Factor[ seq_tier ][ LevelIdx ] *
    MAX_TILE_WIDTH / 4 for each tile
  • For each tile, if RightMostTile is equal to 0, then TileWidth is greater than or equal to 64
  • TileWidth * TileHeight is less than or equal to Tile_Area_Scaling_Factor[ seq_tier ][ LevelIdx ] * 4096
    * 2304 / 4 for each tile
  • FrameWidth is greater than or equal to 16
  • FrameHeight is greater than or equal to 16.

When the mapped level ID, LevelIdx is contained in the tables above, it is a requirement of video
bitstream conformance (i.e., still_picture is equal to 0) that the following constraints hold:

  • TotalDisplayLumaSampleRate is less than or equal to MaxDisplayRate
  • NumFrameHeadersPerSec is less than or equal to MaxNumFrameHeadersPerSec
  • NumRefFrames is less than or equal to MaxLevelRefFrames
  • For a particular Frame with ShowExistingFrame equal to 0

       ◦ LumaSampleCount is less than or equal to FrameParsingTime*MaxDecodeRate.
       ◦ NumTiles is less than or equal to Min(MaxTiles, Max(1, MaxTiles * 120 * FrameParsingTime))
       ◦ CompressedSize is less than or equal to Min((LumaSampleCount * PicSizeProfileFactor >> 3) *
         1.25, (FrameParsingTime* MaxDecodeRate * PicSizeProfileFactor >> 3) ÷ MinCompBasis)
       ◦ FrameSymbolCount is less than or equal to FrameParsingTime * MaxDecodeRate *
         PicSizeProfileFactor * ( 8 ÷ (9 * MinCompBasis) + 1 ÷ 48)
  • MaxTileSizeInLumaSamples * NumFrameHeadersPerSec is less than or equal to
    (Tile_Area_Scaling_Factor[ seq_tier ][ LevelIdx ] * 547,430,400 ) / 4. (The number of 547,430,400
    corresponds to the decode luma sample rate of 3840x2160 * 60fps * 1.1)




AV2 Specification                                                                             Page 1110 of 1169
           NOTE: The purpose of this constraint is to ensure that for decode luma sample rates above
           4K60 there is sufficient parallelism for decoder implementations. Parallelism can be chosen by
           the encoder as either tile level parallelism or temporal layer parallelism or a combination
           provided the above constraint holds. The constraint has no effect on levels 5.1 and below.


    If seq_level_idx is equal to 31 (indicating the maximum parameters level), then there are no level-based
    constraints on the bitstream.


      NOTE: The maximum parameters level should only be set for bitstreams that do not conform to any
      other level. Typically this would be used for large resolution still images.


    The buffer model is used to define additional conformance requirements.

    These requirements depend on the following level, tier, and profile dependent variables:

      • MaxBitrate is equal to MaxMbps multiplied by 1,000,000
      • MaxBufferSize is equal to MaxBitrate multiplied by 1 second

§   A.5.Decoder Conformance
    A level X.Y conformant decoder shall be capable of decoding all bitstreams (that can be decoded by the
    general decoding process) that conform to that level.

    In doing so, the decoder shall display output frames according to the display schedule, if indicated by the
    bitstream.


      NOTE: If the level of a bitstream is equal to 31 (indicating the maximum parameters level), the
      decoder should examine the properties of the bitstream in order to determine if it can be decoded.


                                                                                      ↑ Back to Table of Contents




    AV2 Specification                                                                            Page 1111 of 1169
```
