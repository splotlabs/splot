# AV2 v1.0.0 — Annex D: Multistream composition process (informative)

<!-- Verbatim mirror of the AOM AV2 v1.0.0 specification (© Alliance for Open Media). The PDF is normative; this is a faithful `pdftotext -layout` copy. See [./README.md](./README.md) and [./index.md](./index.md). Do not hand-edit: regenerate via scripts/spec/regenerate-av2-spec.sh. -->

<a id="s-annex-d"></a>

## Annex D: Multistream composition process (informative)

```text
§   Annex D: Multistream composition process (informative)
```

<a id="s-annex-d-1"></a>

### Annex D.1 General

```text
§   D.1.General
    This annex describes the composition process for combining two or more decoded frames into a single
    output frame using the spatial layout specified by the ats_multistream_info or
    ats_multistream_with_alpha_info syntax structure. This process applies when
    ats_atlas_segment_mode_idc[ xAId ] is equal to MULTISTREAM_ATLAS or
    MULTISTREAM_ALPHA_ATLAS.

    It is recommended that decoders support the process when the multistream atlas information syntax is
    present in the bitstream. However, this annex is marked as informative because supporting the
    composition process or implementing it according to the description is not mandatory for a conformant
    decoder.

    Throughout this annex, let xlayerId be equal to GLOBAL_XLAYER_ID and let xAId be equal to
    atlas_segment_id[ xlayerId ].

    The input to this process is:

      • The representation description of the atlas segment (i.e., ats_atlas_segment_mode_idc),
      • Two or more decoded frames that are associated with the same time instance,
      • The extended layer identifier (i.e., obu_xlayer_id) for each of the decoded frames,
      • The chroma subsampling format for each of the decoded frames,
      • The multistream atlas information syntax structure that is associated with the decoded frames (i.e.,
        ats_multistream_info( xlayerId, xAId ) or ats_multistream_with_alpha_info( xlayerId, xAId )).

    The output of this process is the composited frame.

    The process consists of the following ordered steps:

     1. The chroma format determination process specified in Annex D.2 Chroma format determination
        process is invoked. The chroma subsampling format for the decoded frames is provided as input. The
        outputs are the variables subX and subY.
     2. The array initialization process specified in Annex D.3 Array initialization process is invoked.
        ats_msi_width[ xlayerId ][ xAId ], ats_msi_height[ xlayerId ][ xAId ], subX and subY are provided as
        the width, height, subX and subY inputs, respectively. The outputs are the arrays compositeFrameY,
        compositeFrameU, and compositeFrameV.
     3. For each value of i in the range of 0 ... ats_msi_num_atlas_segments_minus_1[ xlayerId ][ xAId ], the
        following ordered steps are performed:

          1. The variable segXLayerId is set equal to ats_msi_input_stream_id[ xlayerId ][ xAId ][ i ]
          2. If ats_atlas_segment_mode_idc[ xAId ] equals MULTISTREAM_ATLAS or
             ats_msi_alpha_segment_flag[ xlayerId ][ xAId ][ i ] equals 0, the spatial mapping process specified
             in Annex D.4 Spatial mapping process is invoked. The decoded frame associated with the
             extended layer identifier segXLayerId, compositeFrameY, compositeFrameU, compositeFrameV,
             ats_msi_width[ xlayerId ][ xAId ], ats_msi_height[ xlayerId ][ xAId ], i, subX, subY are provided as



    AV2 Specification                                                                             Page 1115 of 1169
             input. The outputs are modified arrays of compositeFrameY, compositeFrameU, and
             compositeFrameV values.
          3. Otherwise, the following ordered steps apply:

               1. The variable iAlpha is set equal to i
               2. The variable segXLayerIdAlpha is set equal to ats_msi_input_stream_id[ xlayerId ][ xAId ]
                  [ iAlpha ]
               3. The variable i is incremented by 1
               4. The variable segXLayerId is set equal to ats_msi_input_stream_id[ xlayerId ][ xAId ][ i ]
               5. The spatial mapping process specified in Annex D.5 Spatial mapping with alpha process is
                  invoked. The decoded frame associated with the extended layer identifier segXLayerId, the
                  decoded alpha frame associated with the extended layer identifier segXLayerIdAlpha, the
                  value of BitDepth for the decoded alpha frame associated with the extended layer identifier
                  segXLayerIdAlpha, compositeFrameY, compositeFrameU, compositeFrameV,
                  ats_msi_width[ xlayerId ][ xAId ], ats_msi_height[ xlayerId ][ xAId ], i, iAlpha, subX, subY are
                  provided as input. The outputs are modified arrays of compositeFrameY, compositeFrameU,
                  and compositeFrameV values.


      NOTE: The normative syntax constrains ats_msi_alpha_segment_flag to 0 for the last segment (i
      equal to ats_msi_num_atlas_segments_minus_1), ensuring that an alpha segment is always followed
      by its paired texture segment.


      NOTE: All decoded frames should be converted to the same rendering format prior to being input
      to this process. The conversion process is outside the scope of this annex. But the (non-alpha) input
      frames should be represented using the same dynamic range, color format, color subsampling and
      bit-depth.


```

<a id="s-annex-d-2"></a>

### Annex D.2 Chroma format determination process

```text
§   D.2.Chroma format determination process
    This section defines the process of determining the chroma subsampling factors.

    The input to this process is the chroma subsampling format for the decoded frames.

    The outputs of this process are the variables subX and subY.

    The process consists of the following ordered steps:

     1. If the chroma subsampling format corresponds to a 4:2:0 subsampling format, then the variable subX
        is set equal to 1 and the variable subY is set equal to 1
     2. Otherwise, if the chroma subsampling format corresponds to a 4:2:2 subsampling format, then the
        variable subX is set equal to 1 and the variable subY is set equal to 0
     3. Otherwise, if the chroma subsampling format corresponds to a 4:4:4 subsampling format, then the
        variable subX is set equal to 0 and the variable subY is set equal to 0.
     4. Otherwise (the chroma subsampling format does not correspond to a 4:2:0, 4:2:2 or 4:4:4
        subsampling format), the variable subX is set equal to 0 and the variable subY is set equal to 0.




    AV2 Specification                                                                               Page 1116 of 1169
```

<a id="s-annex-d-3"></a>

### Annex D.3 Array initialization process

```text
§   D.3.Array initialization process
    This section defines the process of initializing a frame array.

    The input to this process is:

      • The variables width and height that indicate the dimensions of the array to be initialized,
      • The variables subX and subY that indicate the chroma subsampling format of the initialized array.

    The outputs of this process are the arrays initializedFrameY, initializedFrameU and initializedFrameV.

    The process consists of the following ordered steps:

     1. The background color determination process specified in Annex D.3.1 Background color
        determination process is invoked. ats_msi_background_red_value[ xlayerId ][ xAId ],
        ats_msi_background_green_value[ xlayerId ][ xAId ] and ats_msi_background_blue_value[ xlayerId ]
        [ xAId ] are provided as the redValue, greenValue, and blueValue inputs. The outputs are the variables
        backgroundValueY, backgroundValueU, and backgroundValueV
     2. The array initializedFrameY is width samples across by height samples down. The sample at location
        x samples across and y samples down is given by initializedFrameY[ y ][ x ] = backgroundValueY.
     3. The array initializedFrameU is 'width >> subX' samples across by 'height >> subY' samples down.
        The sample at location x samples across and y samples down is given by initializedFrameU[ y ][ x ] =
        backgroundValueU.
     4. The array initializedFrameV is 'width >> subX' samples across by 'height >> subY' samples down.
        The sample at location x samples across and y samples down is given by initializedFrameV[ y ][ x ] =
        backgroundValueV.

```

<a id="s-annex-d-3-1"></a>

#### Annex D.3.1 Background color determination process

```text
§   D.3.1.Background color determination process

    This section defines the process of determining the background color for the composited frame.

    The inputs to this process are the variables redValue, greenValue, and blueValue.

    The outputs of this process are the variables backgroundValueY, backgroundValueU, and
    backgroundValueV.

    The process consists of the following ordered steps:

     1. The values Y, U and V are determined that correspond to red, green and blue values specified by
        redValue, greenValue and blueValue, respectively.
     2. The variable backgroundValueY is set equal to Y
     3. The variable backgroundValueU is set equal to U
     4. The variable backgroundValueV is set equal to V


      NOTE: The determination of the background color depends on the dynamic range, color space, bit-
      depth, and/or other characteristics used by the implementation of the composite frame format.




    AV2 Specification                                                                            Page 1117 of 1169
```

<a id="s-annex-d-4"></a>

### Annex D.4 Spatial mapping process

```text
§   D.4.Spatial mapping process
    This section defines the spatial mapping process.

    The inputs to this process are:

      • A decoded frame that is stored in arrays inputY, inputU, and inputV,
      • A decoded frame width and decoded frame height that are stored in the variables inputWidth and
        inputHeight, respectively,
      • A composite frame that is stored in arrays compositeFrameY, compositeFrameU, and
        compositeFrameV,
      • A composite frame width and composite frame height that are stored in the variables
        compositeFrameWidth and compositeFrameHeight, respectively,
      • A segment index that is stored in the variable segIdx,
      • A chroma subsampling format that is stored in the variables subX and subY,

    The outputs of this process are the modified arrays compositeFrameY, compositeFrameU, and
    compositeFrameV. The process consists of the following ordered steps:

     1. The array initialization process specified in Annex D.3 Array initialization process is invoked. The
        ats_msi_segment_width[ xlayerId ][ xAId ][ segIdx ], ats_msi_segment_height[ xlayerId ][ xAId ]
        [ segIdx ], subX, and subY are provided as the width, height, and chroma subsampling format inputs,
        respectively. The outputs are the arrays resampledFrameY, resampledFrameU, and
        resampledFrameV.
     2. The resampling process specified in Annex D.5.1 Frame resampling process is invoked. The arrays
        inputY, inputU, and inputV, and the variables inputWidth, inputHeight, resampledFrameY,
        resampledFrameU, resampledFrameV, ats_msi_segment_width[ xlayerId ][ xAId ][ segIdx ],
        ats_msi_segment_height[ xlayerId ][ xAId ][ segIdx ], subX, and subY are provided as input. The
        outputs are the modified arrays resampledFrameY, resampledFrameU, and resampledFrameV.
     3. The arrays compositeFrameY, compositeFrameU, and compositeFrameV are then updated as follows:

     topLeftPosX = ats_msi_segment_top_left_pos_x[ xlayerId ][ xAId ][ segIdx ]
     topLeftPosY = ats_msi_segment_top_left_pos_y[ xlayerId ][ xAId ][ segIdx ]
     width = min( ats_msi_segment_width[ xlayerId ][ xAId ][ segIdx ], compositeFrameWidth - topLeftPosX )
     height = min( ats_msi_segment_height[ xlayerId ][ xAId ][ segIdx ], compositeFrameHeight - topLeftPosY )

     for( x = 0; x < width; x++ ) {
         for( y = 0; y < height; y++ ) {
             compositeFrameY[ y + topLeftPosY ] [ x + topLeftPosX ] = resampledFrameY[ y ][ x ]
         }
     }

     topLeftPosX = topLeftPosX >> subX
     topLeftPosY = topLeftPosY >> subY
     width = width >> subX
     height = height >> subY

     for( x=0; x < width; x++ ) {
         for( y = 0; y < height; y++ ) {
             compositeFrameU[ y + topLeftPosY ] [ x + topLeftPosX ] = resampledFrameU[ y ][ x ]
             compositeFrameV[ y + topLeftPosY ] [ x + topLeftPosX ] = resampledFrameV[ y ][ x ]
         }
     }




    AV2 Specification                                                                               Page 1118 of 1169
```

<a id="s-annex-d-5"></a>

### Annex D.5 Spatial mapping with alpha process

```text
§   D.5.Spatial mapping with alpha process
    This section defines the spatial mapping process with an alpha frame.

    The inputs to this process are:

      • A decoded frame that is stored in arrays inputY, inputU, and inputV,
      • A decoded frame width and decoded frame height that are stored in the variables inputWidth and
        inputHeight, respectively,
      • A decoded alpha frame that is stored in array alphaY,
      • A decoded alpha frame width and decoded alpha frame height that are stored in the variables
        alphaWidth and alphaHeight, respectively,
      • A decoded alpha frame bitdepth that is stored in the variable bitdepthAlpha,
      • A composite frame that is stored in arrays compositeFrameY, compositeFrameU, and
        compositeFrameV,
      • A composite frame width and composite frame height that are stored in the variables
        compositeFrameWidth and compositeFrameHeight, respectively,
      • A segment index that is stored in the variable segIdx,
      • An alpha segment index that is stored in the variable segIdxAlpha,
      • A chroma subsampling format that is stored in the variables subX and subY,

    The outputs of this process are the modified arrays compositeFrameY, compositeFrameU, and
    compositeFrameV. The process consists of the following ordered steps:

     1. The array initialization process specified in Annex D.3 Array initialization process is invoked. The
        ats_msi_segment_width[ xlayerId ][ xAId ][ segIdx ], ats_msi_segment_height[ xlayerId ][ xAId ]
        [ segIdx ], subX, and subY are provided as the width, height, and chroma subsampling format inputs,
        respectively. The outputs are the arrays resampledFrameY, resampledFrameU, and
        resampledFrameV.
     2. The resampling process specified in Annex D.5.1 Frame resampling process is invoked. The arrays
        inputY, inputU, and inputV, and the variables inputWidth, inputHeight, resampledFrameY,
        resampledFrameU, resampledFrameV, ats_msi_segment_width[ xlayerId ][ xAId ][ segIdx ],
        ats_msi_segment_height[ xlayerId ][ xAId ][ segIdx ], subX, and subY are provided as input. The
        outputs are the modified arrays resampledFrameY, resampledFrameU, and resampledFrameV.
     3. The array resampleAlphaFrameY is ats_msi_segment_width[ xlayerId ][ xAId ][ segIdxAlpha ] samples
        across by ats_msi_segment_height[ xlayerId ][ xAId ][ segIdxAlpha ] samples down. The sample at
        location x samples across and y samples down is given by resampleAlphaFrameY[ y ][ x ] = 1.
     4. The resampling process specified in Annex D.5.2 Monochrome frame resampling process is invoked.
        The array alphaY and the variables alphaWidth, alphaHeight, resampleAlphaFrameY,
        ats_msi_segment_width[ xlayerId ][ xAId ][ segIdxAlpha ], ats_msi_segment_height[ xlayerId ][ xAId ]
        [ segIdxAlpha ] are provided as input. The outputs are the modified array resampleAlphaFrameY.
     5. The arrays compositeFrameY, compositeFrameU, and compositeFrameV are then updated as follows:

     topLeftPosX = ats_msi_segment_top_left_pos_x[ xlayerId ][ xAId ][ segIdx ]
     topLeftPosY = ats_msi_segment_top_left_pos_y[ xlayerId ][ xAId ][ segIdx ]
     width = min( ats_msi_segment_width[ xlayerId ][ xAId ][ segIdx ], compositeFrameWidth - topLeftPosX )



    AV2 Specification                                                                               Page 1119 of 1169
     height = min( ats_msi_segment_height[ xlayerId ][ xAId ][ segIdx ], compositeFrameHeight - topLeftPosY )

     alphaTopLeftPosX = ats_msi_segment_top_left_pos_x[ xlayerId ][ xAId ][ segIdxAlpha ]
     alphaTopLeftPosY = ats_msi_segment_top_left_pos_y[ xlayerId ][ xAId ][ segIdxAlpha ]
     alphaWidth = ats_msi_segment_width[ xlayerId ][ xAId ][ segIdxAlpha ]
     alphaHeight = ats_msi_segment_height[ xlayerId ][ xAId ][ segIdxAlpha ]
     alphaMax = 1 << bitdepthAlpha

     for( x = 0; x < width; x++ ) {
         for( y = 0; y < height; y++ ) {
             ax = x + topLeftPosX - alphaTopLeftPosX
             ay = y + topLeftPosY - alphaTopLeftPosY
             alpha = (ax >= 0 && ax < alphaWidth && ay >= 0 && ay < alphaHeight ) ?
                        resampleAlphaFrameY[ ay ] [ ax ] : alphaMax
             temp = ( alphaMax - alpha ) * compositeFrameY[ y + topLeftPosY ] [ x + topLeftPosX ] + alpha *
     resampledFrameY[ y ][ x ]
             compositeFrameY[ y + topLeftPosY ] [ x + topLeftPosX ] = Round2(temp, bitdepthAlpha)
         }
     }

     uvTopLeftPosX = topLeftPosX >> subX
     uvTopLeftPosY = topLeftPosY >> subY
     width = width >> subX
     height = height >> subY

     for( x = 0; x < width; x++ ) {
         for( y = 0; y < height; y++ ) {
             ax = (x << subX) + topLeftPosX - alphaTopLeftPosX
             ay = (y << subY) + topLeftPosY - alphaTopLeftPosY
             alpha = (ax >= 0 && ax < alphaWidth && ay >= 0 && ay < alphaHeight ) ?
                        resampleAlphaFrameY[ ay ] [ ax ] : alphaMax
             temp = ( alphaMax - alpha ) * compositeFrameU[ y + uvTopLeftPosY ] [ x + uvTopLeftPosX ] + alpha *
     resampledFrameU[ y ][ x ]
             compositeFrameU[ y + uvTopLeftPosY ] [ x + uvTopLeftPosX ] = Round2(temp, bitdepthAlpha)
             temp = ( alphaMax - alpha ) * compositeFrameV[ y + uvTopLeftPosY ] [ x + uvTopLeftPosX ] + alpha *
     resampledFrameV[ y ][ x ]
             compositeFrameV[ y + uvTopLeftPosY ] [ x + uvTopLeftPosX ] = Round2(temp, bitdepthAlpha)
         }
     }


```

<a id="s-annex-d-5-1"></a>

#### Annex D.5.1 Frame resampling process

```text
§   D.5.1.Frame resampling process
    This section is a placeholder for the frame resampling process. The actual resampling process is outside
    the scope of this annex.

    The input to this process is:

      • An input frame that is stored in the arrays inputY, inputU, and inputV,
      • An input frame width and input frame height that are stored in the variables inputWidth and
        inputHeight, respectively,
      • An output frame that is stored in the variables resampledFrameY, resampledFrameU, and
        resampledFrameV,
      • An output frame width and output frame height that is stored in the variables outputWidth and
        outputHeight, respectively,
      • A chroma subsampling format that is stored in the variables subX and subY.

    The outputs of this process are arrays of modified resampledFrameY, resampledFrameU, and
    resampledFrameV values.




    AV2 Specification                                                                              Page 1120 of 1169
```

<a id="s-annex-d-5-2"></a>

#### Annex D.5.2 Monochrome frame resampling process

```text
§   D.5.2.Monochrome frame resampling process
    This section is a placeholder for the monochrome frame resampling process. The actual resampling
    process is outside the scope of this annex.

    The input to this process is:

      • An input frame that is stored in the array inputY,
      • An input frame width and input frame height that are stored in the variables inputWidth and
        inputHeight, respectively,
      • An output frame that is stored in the variables resampledFrameY,
      • An output frame width and output frame height that are stored in the variables outputWidth and
        outputHeight, respectively,

    The output of this process is an array of modified resampledFrameY values.

                                                                                   ↑ Back to Table of Contents




    AV2 Specification                                                                         Page 1121 of 1169
```
