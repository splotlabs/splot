# AV2 v1.0.0 — Annex G: Layer composition and Atlas usage examples (informative)

<!-- Verbatim mirror of the AOM AV2 v1.0.0 specification (© Alliance for Open Media). The PDF is normative; this is a faithful `pdftotext -layout` copy. See [./README.md](./README.md) and [./index.md](./index.md). Do not hand-edit: regenerate via scripts/spec/regenerate-av2-spec.sh. -->

<a id="s-annex-g"></a>

## Annex G: Layer composition and Atlas usage examples (informative)

```text
§   Annex G: Layer composition and Atlas usage examples (informative)
```

<a id="s-annex-g-1"></a>

### Annex G.1 General

```text
§   G.1.General
    This annex provides detailed examples demonstrating how the Layer Configuration Record (LCR) works
    with Atlas Segments to enable complex multi-layer and multi-view content scenarios. The examples
    illustrate practical use cases including viewport-dependent 360-degree video streaming, subpicture
    composition with resampling and cropping, and region-of-interest scalability.

    The Layer Configuration Record (LCR) provides detailed semantic metadata about each layer
    including its type (texture or auxiliary), purpose (alpha, depth, gain map, etc.), view association, and atlas
    segment mapping.

    The Atlas provides geometric metadata: where each layer should be positioned in the final rendered
    output, how layers are composed spatially, and the dimensions of the virtual canvas.

    Together, LCR and Atlas enable decoders and renderers to understand both what each layer represents
    semantically and where it should be placed geometrically. When the coded layer resolution differs from
    the atlas segment dimensions, resampling is required. When only a portion of the decoded layer should
    be used, cropping is applied before spatial mapping.

```

<a id="s-annex-g-2"></a>

### Annex G.2 360-degree viewport-dependent streaming with subpictures

```text
§   G.2.360-degree viewport-dependent streaming with subpictures
    This example demonstrates a 360-degree video streaming application using subpicture-based viewport-
    dependent delivery. The equirectangular projection is divided into spatial subpictures, with the viewport
    region encoded at high quality and peripheral regions at lower quality. This example extends the
    approach to include alpha and depth auxiliary layers for each subpicture, enabling advanced rendering
    techniques.

    Configuration:

      • 9 extended layers representing different spatial subpictures in a 3×3 grid
      • Subpictures arranged to completely cover 3840×1920 equirectangular projection:

           ◦ Extended layer 0: Top-left subpicture (1280×640, low quality)
           ◦ Extended layer 1: Top-center subpicture (1280×640, medium quality)
           ◦ Extended layer 2: Top-right subpicture (1280×640, low quality)
           ◦ Extended layer 3: Middle-left subpicture (1280×640, medium quality)
           ◦ Extended layer 4: Center viewport subpicture (1280×640, HIGH quality)
           ◦ Extended layer 5: Middle-right subpicture (1280×640, medium quality)
           ◦ Extended layer 6: Bottom-left subpicture (1280×640, low quality)
           ◦ Extended layer 7: Bottom-center subpicture (1280×640, medium quality)
           ◦ Extended layer 8: Bottom-right subpicture (1280×640, low quality - back-facing)
      • Each subpicture includes texture, alpha, and depth using embedded layers within each extended
        layer




    AV2 Specification                                                                             Page 1151 of 1169
      • Atlas mode 0 (Enhanced Atlas) composes subpictures into complete equirectangular layout with no
        gaps
      • The 3×3 grid provides symmetrical coverage with the viewport at the natural center position

```

<a id="s-annex-g-2-1"></a>

#### Annex G.2.1 Layer structure

```text
§   G.2.1.Layer structure

    Each extended layer contains three embedded layers:

      • Embedded layer 0: Texture (video content)
      • Embedded layer 1: Alpha channel (for subpicture blending at boundaries)
      • Embedded layer 2: Depth map (for 3D-aware viewport adaptation)

    Total structure:

     Extended layer 0 (top-left subpicture):
       - Embedded layer 0: Texture (1280×640, low quality)
       - Embedded layer 1: Alpha (for smooth subpicture blending)
       - Embedded layer 2: Depth (for parallax-aware rendering)

     Extended layer 1 (top-center subpicture):
       - Embedded layer 0: Texture (1280×640, medium quality)
       - Embedded layer 1: Alpha
       - Embedded layer 2: Depth

     Extended layer 2 (top-right subpicture):
       - Embedded layer 0: Texture (1280×640, low quality)
       - Embedded layer 1: Alpha
       - Embedded layer 2: Depth

     Extended layer 3 (middle-left subpicture):
       - Embedded layer 0: Texture (1280×640, medium quality)
       - Embedded layer 1: Alpha
       - Embedded layer 2: Depth

     Extended layer 4 (center viewport subpicture):
       - Embedded layer 0: Texture (1280×640, HIGH quality)
       - Embedded layer 1: Alpha
       - Embedded layer 2: Depth

     Extended layer 5 (middle-right subpicture):
       - Embedded layer 0: Texture (1280×640, medium quality)
       - Embedded layer 1: Alpha
       - Embedded layer 2: Depth

     Extended layer 6 (bottom-left subpicture):
       - Embedded layer 0: Texture (1280×640, low quality)
       - Embedded layer 1: Alpha
       - Embedded layer 2: Depth

     Extended layer 7 (bottom-center subpicture):
       - Embedded layer 0: Texture (1280×640, medium quality)
       - Embedded layer 1: Alpha
       - Embedded layer 2: Depth

     Extended layer 8 (bottom-right subpicture - back-facing):
       - Embedded layer 0: Texture (1280×640, low quality)
       - Embedded layer 1: Alpha
       - Embedded layer 2: Depth




    AV2 Specification                                                                        Page 1152 of 1169
```

<a id="s-annex-g-2-2"></a>

#### Annex G.2.2 LCR configuration

```text
§   G.2.2.LCR configuration

    In this example, a global LCR is used, carried in the global layer context
    (obu_xlayer_id = GLOBAL_XLAYER_ID = 31), consistent with the atlas also being signaled in the global layer
    context.

    The LCR specifies properties for each embedded layer within each extended layer.
    lcr_local_atlas_id_present_flag enables atlas segment assignment, and lcr_layer_atlas_segment_id maps each
    embedded layer to its target atlas segment — this is how the Enhanced Atlas knows which layer fills each
    segment. Multiple embedded layers within the same extended layer (texture, alpha, depth) all reference
    the same atlas segment, with lcr_priority_order controlling rendering order.

    For extended layer 4 (center viewport subpicture):

     // Extended layer 4 - Local LCR
     lcr_local_atlas_id_present_flag[4] = 1    // Enable atlas segment assignment
     lcr_local_atlas_id[4] = 0                // References atlas with atlas_segment_id = 0

     // Extended layer 4, embedded layer 0 (center viewport texture)
     lcr_layer_type[0][4][0] = TEXTURE_LAYER (0)
     lcr_view_type[0][4][0] = VIEW_CENTER (1)
     lcr_view_id[0][4][0] = 0
     lcr_layer_atlas_segment_id[0][4][0] = 4 // Maps to atlas segment 4 (center cell)
     lcr_priority_order[0][4][0] = 0          // Rendered first (base content)
     lcr_rendering_method[0][4][0] = 0        // Overwrite

     // Extended layer 4, embedded layer 1 (center viewport alpha)
     lcr_layer_type[0][4][1] = AUX_LAYER (1)
     lcr_auxiliary_type[0][4][1] = ALPHA_AUX (0)
     lcr_view_type[0][4][1] = VIEW_CENTER (1)
     lcr_view_id[0][4][1] = 0
     lcr_layer_atlas_segment_id[0][4][1] = 4 // Same segment — auxiliary layer for boundary blending
     lcr_priority_order[0][4][1] = 1
     lcr_rendering_method[0][4][1] = 0        // Overwrite

     // Extended layer 4, embedded layer 2 (center viewport depth)
     lcr_layer_type[0][4][2] = AUX_LAYER (1)
     lcr_auxiliary_type[0][4][2] = DEPTH_AUX (1)
     lcr_view_type[0][4][2] = VIEW_CENTER (1)
     lcr_view_id[0][4][2] = 0
     lcr_layer_atlas_segment_id[0][4][2] = 4 // Same segment — auxiliary depth for 3D rendering
     lcr_priority_order[0][4][2] = 2
     lcr_rendering_method[0][4][2] = 0        // Overwrite

     // Similar LCR configuration for extended layers 0-3, 5-8 (other subpictures)
     // Each subpicture's embedded layers map to their respective atlas segments


    For extended layer 0 (top-left subpicture):

     // Extended layer 0 - Local LCR
     lcr_local_atlas_id_present_flag[0] = 1
     lcr_local_atlas_id[0] = 0

     // Extended layer 0, embedded layer 0 (top-left subpicture texture)
     lcr_layer_type[0][0][0] = TEXTURE_LAYER (0)
     lcr_view_type[0][0][0] = VIEW_CENTER (1)
     lcr_view_id[0][0][0] = 0 // Same view, different spatial region
     lcr_layer_atlas_segment_id[0][0][0] = 0 // Maps to atlas segment 0 (top-left cell)
     lcr_priority_order[0][0][0] = 0
     lcr_rendering_method[0][0][0] = 0        // Overwrite

     // Extended layer 0, embedded layer 1 (top-left subpicture alpha)
     lcr_layer_type[0][0][1] = AUX_LAYER (1)
     lcr_auxiliary_type[0][0][1] = ALPHA_AUX (0)



    AV2 Specification                                                                               Page 1153 of 1169
     lcr_view_type[0][0][1] = VIEW_CENTER (1)
     lcr_view_id[0][0][1] = 0
     lcr_layer_atlas_segment_id[0][0][1] = 0 // Same segment as texture
     lcr_priority_order[0][0][1] = 1
     lcr_rendering_method[0][0][1] = 0        // Overwrite

     // Extended layer 0, embedded layer 2 (top-left subpicture depth)
     lcr_layer_type[0][0][2] = AUX_LAYER (1)
     lcr_auxiliary_type[0][0][2] = DEPTH_AUX (1)
     lcr_view_type[0][0][2] = VIEW_CENTER (1)
     lcr_view_id[0][0][2] = 0
     lcr_layer_atlas_segment_id[0][0][2] = 0 // Same segment as texture
     lcr_priority_order[0][0][2] = 2
     lcr_rendering_method[0][0][2] = 0        // Overwrite

     // Configurations for extended layers 1-3, 5-8 follow same pattern


    Key observations:

      • All subpictures share the same lcr_view_id = 0 (single 360-degree view)
      • Extended layers differentiate spatial subpictures
      • Embedded layers within each extended layer differentiate content types (texture, alpha, depth)
      • Alpha channels enable smooth blending at subpicture boundaries
      • Depth maps enable motion-parallax and 3D-aware rendering

```

<a id="s-annex-g-2-3"></a>

#### Annex G.2.3 Atlas configuration

```text
§   G.2.3.Atlas configuration

    The atlas uses mode 0 (enhanced atlas) with a 3×3 uniform grid that completely covers the 3840×1920
    equirectangular projection with no gaps. All 9 cells are the same size (1280×640), so
    ats_uniform_spacing_flag = 1 applies. With ats_single_region_per_atlas_segment_flag = 1, each of the 9 grid
    regions maps one-to-one to a segment (segments 0–8 in row-major order). The LCR’s
    lcr_layer_atlas_segment_id for each embedded layer references these segment IDs —no stream IDs appear
    in the atlas itself. Assuming the atlas is in the global layer context (xlayerId = GLOBAL_XLAYER_ID = 31)
    and atlas_segment_id = 0:

     // atlas_segment_info_obu() - OBU with obu_xlayer_id = GLOBAL_XLAYER_ID (31)
     atlas_segment_id[31] = 0           // xAId = 0
     ats_atlas_segment_mode_idc[0] = 0 // ENHANCED_ATLAS

     // ats_enhanced_atlas_info(xAId=0) [wrapper defined in companion normative PR] calls ats_region_info then
     ats_region_to_segment_mapping:

     // ats_region_info(xAId=0): 3×3 uniform grid
     ats_num_region_columns_minus_1[0] = 2   // 3 columns
     ats_num_region_rows_minus_1[0] = 2      // 3 rows
     ats_uniform_spacing_flag[0] = 1        // Uniform spacing (all cells equal size)
     ats_region_width_minus_1[0] = 1279     // Each region: 1280 pixels wide
     ats_region_height_minus_1[0] = 639     // Each region: 640 pixels tall
     // AtlasWidth = 1280 × 3 = 3840, AtlasHeight = 640 × 3 = 1920

     // ats_region_to_segment_mapping(xAId=0): 1-to-1 mapping (each region = one segment)
     ats_single_region_per_atlas_segment_flag[0] = 1
     // ats_num_atlas_segments_minus_1[0] = 8 (inferred: NumRegionsInAtlas - 1 = 9 - 1 = 8)

     // Segment IDs are assigned implicitly in row-major order (left→right, top→bottom):
     // Segment 0: region (col=0, row=0) → top-left     canvas position (0, 0),    1280×640
     // Segment 1: region (col=1, row=0) → top-center   canvas position (1280, 0), 1280×640
     // Segment 2: region (col=2, row=0) → top-right    canvas position (2560, 0), 1280×640
     // Segment 3: region (col=0, row=1) → middle-left canvas position (0, 640), 1280×640
     // Segment 4: region (col=1, row=1) → CENTER       canvas position (1280, 640), 1280×640
     // Segment 5: region (col=2, row=1) → middle-right canvas position (2560, 640), 1280×640



    AV2 Specification                                                                               Page 1154 of 1169
     // Segment 6: region (col=0, row=2) → bottom-left canvas position (0, 1280), 1280×640
     // Segment 7: region (col=1, row=2) → bottom-center canvas position (1280, 1280), 1280×640
     // Segment 8: region (col=2, row=2) → bottom-right canvas position (2560, 1280), 1280×640


    The center viewport subpicture (extended layer 4) maps to segment 4 via lcr_layer_atlas_segment_id = 4.
    Note that this example uses 9 extended layers, which requires LCR (not MSDO) since MSDO is limited to
    a maximum of 4 independent streams.

```

<a id="s-annex-g-2-4"></a>

#### Annex G.2.4 Viewport-dependent streaming process

```text
§   G.2.4.Viewport-dependent streaming process

     1. Initial state: Client detects user’s head orientation/gaze direction
     2. Viewport determination: Based on orientation, client determines which subpictures are visible:

           ◦ Front-facing (0°): Extended layer 4 (center viewport) at high priority
           ◦ Immediately adjacent subpictures: Extended layers 1, 3, 5, 7 at medium priority
           ◦ Corner and back-facing subpictures: Extended layers 0, 2, 6, 8 at lower priority
     3. Adaptive fetching:

           ◦ High bandwidth: Fetch all 9 extended layers (complete sphere coverage)

                ▪ Center viewport subpicture (extended layer 4): high quality
                ▪ Adjacent subpictures (extended layers 1, 3, 5, 7): medium quality
                ▪ Corner/back subpictures (extended layers 0, 2, 6, 8): low quality
           ◦ Medium bandwidth: Fetch center + immediately adjacent visible subpictures

                ▪ Skip corner and back-facing subpictures until user rotates toward them
           ◦ Low bandwidth: Fetch center viewport subpicture only

                ▪ Decoder synthesizes peripheral regions from viewport using depth map
     4. Rendering with alpha and depth:

           ◦ Texture layers: Provide base video content for each subpicture
           ◦ Alpha channels: Enable smooth blending at subpicture boundaries

                ▪ Prevents visible seams between subpictures in the 3×3 grid
                ▪ Allows feathering for quality transitions
           ◦ Depth maps: Enable advanced rendering:

                ▪ Motion parallax compensation for head translation
                ▪ View synthesis for missing subpictures (depth-image-based rendering)
                ▪ Foveated rendering (higher quality in gaze direction)
                ▪ Occlusion-aware composition for overlaid UI elements
     5. Head motion tracking:

           ◦ When user rotates head, client dynamically switches which extended layers are fetched
           ◦ Smooth transition enabled by alpha blending between subpictures



    AV2 Specification                                                                             Page 1155 of 1169
           ◦ Depth maps allow temporal interpolation during subpicture switches
           ◦ Complete 3×3 grid coverage with center viewport ensures content available for any viewing
             direction

```

<a id="s-annex-g-2-5"></a>

#### Annex G.2.5 Benefits for 360-degree streaming

```text
§   G.2.5.Benefits for 360-degree streaming

      • Bandwidth efficiency: Only fetch subpictures in or near viewport, reducing bandwidth by 50-80%
        compared to full-sphere streaming
      • Quality adaptation: Viewport receives high quality, periphery receives lower quality
      • Smooth transitions: Alpha channels eliminate subpicture boundary artifacts
      • 3D-aware rendering: Depth maps enable parallax, view synthesis, and occlusion handling
      • Scalable: Can adjust number of subpictures (extended layers) based on content complexity
      • Low latency: Viewport subpicture can be prioritized for fast initial display
      • Subpicture independence: Each extended layer is independently decodable without dependencies
        on other subpictures




    AV2 Specification                                                                          Page 1156 of 1169
                                                         360° Viewport-Dependent Streaming: 3×3 Grid with Center Viewport
                                                                                  Atlas Canvas: 3840×1920 (Equirectangular Projection)


                              Layer 0                                                                   Layer 1                                                                Layer 2
                              Top-Left                                                                Top-Center                                                               Top-Right
                          1280×640 @ (0,0)                                                        1280×640 @ (1280,0)                                                     1280×640 @ (2560,0)
                             Low Qual                                                                  Med Qual                                                                Low Qual
                          Tex+Alpha+Depth                                                           Tex+Alpha+Depth                                                         Tex+Alpha+Depth




                           atlas_seg_id=0                                                             atlas_seg_id=1                                                         atlas_seg_id=2


                              Layer 3                                                          Layer 4 (VIEWPORT)                                                              Layer 5
                             Middle-Left                                                                Center                                                               Middle-Right
                         1280×640 @ (0,640)                                                      1280×640 @ (1280,640)                                                  1280×640 @ (2560,640)




                                                                                                                                                                                                                             1920 pixels
                             Med Qual                                                                                                                                         Med Qual
                                                                                                       HIGH Quality
                          Tex+Alpha+Depth                                                                 Texture                                                           Tex+Alpha+Depth
                                                                                                           Alpha
                                                                                                            Depth


                           atlas_seg_id=3                                                             atlas_seg_id=4                                                         atlas_seg_id=5


                              Layer 6                                                                   Layer 7                                                                Layer 8
                            Bottom-Left                                                             Bottom-Center                                                             Bottom-Right
                        1280×640 @ (0,1280)                                                     1280×640 @ (1280,1280)                                                       (Back-Facing)
                             Low Qual                                                                 Med Qual                                                          1280×640 @ (2560,1280)
                          Tex+Alpha+Depth                                                           Tex+Alpha+Depth                                                             Low Qual
                                                                                                                                                                            Tex+Alpha+Depth



                           atlas_seg_id=6                                                             atlas_seg_id=7                                                         atlas_seg_id=8




                                                                                                      3840 pixels


                                        Symmetrical 3×3 Grid with Center Viewport                                                                                       Layer Configuration (LCR Required - 9 Layers)

 Layout: 9 subpictures in 3 columns × 3 rows (1280×640 each) completely cover 3840×1920 canvas                                     Each extended layer contains 3 embedded layers:
 Center viewport: Layer 4 at position (1,1) encoded at HIGH quality for front-facing view                                             • Embedded layer 0: Texture (TEXTURE_LAYER) - video content
 Adjacent subpictures: Layers 1, 3, 5, 7 (top/left/right/bottom of center) at medium quality                                          • Embedded layer 1: Alpha (AUX_LAYER, ALPHA_AUX) - smooth blending
 Corner subpictures: Layers 0, 2, 6, 8 at low quality (peripheral and back-facing regions)                                            • Embedded layer 2: Depth (AUX_LAYER, DEPTH_AUX) - parallax rendering
 Perfect symmetry: Equal granularity horizontally and vertically; natural center positioning
                                                                                                                                   All subpictures share lcr_view_id = 0 (single 360° view)
 Complete coverage: No gaps; content available for any viewing direction
                                                                                                                                   MSDO cannot be used (limited to 4 streams); requires LCR for 9 layers



                                                                                                          Key Benefits of 3×3 Grid for 360° Streaming

 • Symmetrical coverage: 3×3 grid provides equal horizontal and vertical granularity (better than 3×2 or 2×3)
 • Natural center viewport: Center subpicture (1,1) is the obvious front-facing position surrounded by 8 adjacent subpictures
 • Complete corner coverage: All 4 corners explicitly covered (0=top-left, 2=top-right, 6=bottom-left, 8=bottom-right)
 • Bandwidth efficiency: Fetch high-quality center + visible adjacent subpictures only (60-80% reduction vs full sphere)
 • Quality adaptation: Center at high (1280×640), adjacent at medium, corners at low - matches viewport importance
 • Smooth transitions: Alpha channels eliminate visible seams; enables quality feathering between subpictures
 • 3D-aware rendering: Depth maps enable motion parallax, view synthesis for missing subpictures, occlusion handling
 • Dynamic streaming: As user rotates, fetch different subpictures with priority based on viewing direction
 • Better aspect ratio: 1280×640 subpictures (~2:1) more natural for equirectangular content than taller subpictures
 • Independent decoding: Each extended layer independently decodable; no inter-subpicture dependencies
 • Scalable quality: Can adjust per-subpicture encoding parameters based on importance without changing structure



Figure G.1: 360-degree viewport-dependent streaming using subpictures arranged in a 3×3 grid. Nine
extended layers completely cover the 3840×1920 equirectangular projection with perfect symmetry and
no gaps. The center viewport subpicture (extended layer 4, position 1,1) is encoded at high quality
(1280×640) for the front-facing view. Immediately adjacent subpictures (layers 1, 3, 5, 7) use medium
quality, while corner and back-facing subpictures (layers 0, 2, 6, 8) use low quality. Each subpicture
contains three embedded layers: texture, alpha (for smooth blending), and depth (for parallax and view
synthesis). The symmetrical 3×3 grid layout ensures complete sphere coverage with natural center
viewport positioning, so content is available regardless of viewing direction. Alpha channels eliminate
subpicture boundary artifacts, while depth maps enable 3D-aware rendering and motion parallax
compensation.




AV2 Specification                                                                                                                                                                                                       Page 1157 of 1169
```

<a id="s-annex-g-3"></a>

### Annex G.3 Subpicture composition example

```text
§   G.3.Subpicture composition example
    This example demonstrates a video conferencing application where multiple video sources (participants)
    are composed into a single virtual canvas. The atlas acts as a virtual screen layout manager, positioning
    different layers at different locations to create a multi-party conferencing view. This scenario uses three
    extended layers representing three participants, with one participant requiring resampling:

      • Extended layer 0: Main speaker (high resolution, 1280×1080)
      • Extended layer 1: Participant 2 (encoded at 480×360, upsampled to 640×540)
      • Extended layer 2: Participant 3 (medium resolution, 640×540)

```

<a id="s-annex-g-3-1"></a>

#### Annex G.3.1 LCR configuration

```text
§   G.3.1.LCR configuration

    Each extended layer has its own local LCR. The lcr_local_atlas_id_present_flag enables atlas segment
    assignment, and lcr_layer_atlas_segment_id explicitly maps each embedded layer to its target atlas
    segment. This is the mechanism by which the Enhanced Atlas knows which layer provides content for
    each segment — there are no stream IDs in the atlas itself.

     // Extended layer 0 (main speaker) - Local LCR
     lcr_local_atlas_id_present_flag[0] = 1 // Enable atlas segment assignment for this layer
     lcr_local_atlas_id[0] = 0              // References atlas with atlas_segment_id = 0
     lcr_layer_type[0][0][0] = TEXTURE_LAYER (0)
     lcr_view_type[0][0][0] = VIEW_CENTER (1)
     lcr_layer_atlas_segment_id[0][0][0] = 0 // Maps to atlas segment 0 (full left column)
     lcr_priority_order[0][0][0] = 0          // Single layer per segment; priority order not critical
     lcr_rendering_method[0][0][0] = 0        // Overwrite

     // Extended layer 1 (participant 2) - Local LCR
     lcr_local_atlas_id_present_flag[1] = 1
     lcr_local_atlas_id[1] = 0
     lcr_layer_type[0][1][0] = TEXTURE_LAYER (0)
     lcr_view_type[0][1][0] = VIEW_CENTER (1)
     lcr_layer_atlas_segment_id[0][1][0] = 1 // Maps to atlas segment 1 (top-right cell)
     lcr_priority_order[0][1][0] = 0
     lcr_rendering_method[0][1][0] = 0        // Overwrite

     // Extended layer 2 (participant 3) - Local LCR
     lcr_local_atlas_id_present_flag[2] = 1
     lcr_local_atlas_id[2] = 0
     lcr_layer_type[0][2][0] = TEXTURE_LAYER (0)
     lcr_view_type[0][2][0] = VIEW_CENTER (1)
     lcr_layer_atlas_segment_id[0][2][0] = 2 // Maps to atlas segment 2 (bottom-right cell)
     lcr_priority_order[0][2][0] = 0
     lcr_rendering_method[0][2][0] = 0        // Overwrite



      NOTE: each segment here has exactly one layer assigned to it. However, the Enhanced Atlas allows
      multiple layers to reference the same segment. For example, if participant 2 also had an alpha
      channel layer (for chroma-key compositing), that layer would set lcr_layer_atlas_segment_id = 1 as well,
      with lcr_rendering_method controlling how it composites with the texture layer already mapped to
      segment 1.


    A global LCR describes the overall structure:

     // Global LCR (obu_xlayer_id = 31)
     lcr_global_config_record_id = 1
     lcr_xlayer_map = 0x07 // Extended layers 0, 1, 2 present (bits 0-2 set)




    AV2 Specification                                                                               Page 1158 of 1169
     lcr_global_purpose_id = 6 // Multiview Playback
     lcr_global_atlas_id_present_flag = 1
     lcr_global_atlas_id = 0 // References the global atlas


```

<a id="s-annex-g-3-2"></a>

#### Annex G.3.2 Atlas configuration

```text
§   G.3.2.Atlas configuration

    The global atlas (obu_xlayer_id = 31) uses mode 0 (enhanced atlas) to define the layout as a 2-column ×
    2-row non-uniform grid. Unlike the multistream atlas, no stream IDs appear in the atlas itself — stream-
    to-segment assignment is handled entirely by lcr_layer_atlas_segment_id in each layer’s LCR. The three
    participants map naturally to three grid-derived segments: the main speaker occupies the full left column
    (both rows merged into one segment), while each participant occupies one right-column cell.

     // atlas_segment_info_obu() - OBU with obu_xlayer_id = GLOBAL_XLAYER_ID (31)
     atlas_segment_id[31] = 0           // xAId = 0
     ats_atlas_segment_mode_idc[0] = 0 // ENHANCED_ATLAS

     // ats_enhanced_atlas_info(xAId=0) [wrapper defined in companion normative PR] calls ats_region_info then
     ats_region_to_segment_mapping:

     // ats_region_info(xAId=0): 2×2 non-uniform grid
     ats_num_region_columns_minus_1[0] = 1   // 2 columns
     ats_num_region_rows_minus_1[0] = 1      // 2 rows
     ats_uniform_spacing_flag[0] = 0        // Non-uniform spacing
     ats_column_width_minus_1[0][0] = 1279   // Column 0: 1280 pixels (main speaker)
     ats_column_width_minus_1[0][1] = 639    // Column 1: 640 pixels (participants)
     ats_row_height_minus_1[0][0] = 539      // Row 0: 540 pixels
     ats_row_height_minus_1[0][1] = 539      // Row 1: 540 pixels
     // AtlasWidth = 1280 + 640 = 1920, AtlasHeight = 540 + 540 = 1080

     // ats_region_to_segment_mapping(xAId=0)
     ats_single_region_per_atlas_segment_flag[0] = 0   // Not 1-to-1: main speaker spans 2 rows
     ats_num_atlas_segments_minus_1[0] = 2              // 3 segments

     // Segment 0: Main speaker (left column, spans both rows → 1280×1080)
     // Canvas top-left: (0, 0)
     ats_top_left_region_column[0][0] = 0
     ats_top_left_region_row[0][0] = 0
     ats_bottom_right_region_column_off[0][0] = 0   // Remains in column 0
     ats_bottom_right_region_row_off[0][0] = 1      // Extends to row 1

     // Segment 1: Participant 2 (top-right cell → 640×540)
     // Canvas top-left: (1280, 0)
     ats_top_left_region_column[0][1] = 1
     ats_top_left_region_row[0][1] = 0
     ats_bottom_right_region_column_off[0][1] = 0
     ats_bottom_right_region_row_off[0][1] = 0

     // Segment 2: Participant 3 (bottom-right cell → 640×540)
     // Canvas top-left: (1280, 540)
     ats_top_left_region_column[0][2] = 1
     ats_top_left_region_row[0][2] = 1
     ats_bottom_right_region_column_off[0][2] = 0
     ats_bottom_right_region_row_off[0][2] = 0


    Segment dimensions and canvas positions are derived from the cumulative column widths and row
    heights. The segment IDs (0, 1, 2) are assigned implicitly by index since ats_signaled_atlas_segment_ids_flag
    is not set. These IDs are what each layer’s lcr_layer_atlas_segment_id references.

```

<a id="s-annex-g-3-3"></a>

#### Annex G.3.3 Rendering and adaptive streaming

```text
§   G.3.3.Rendering and adaptive streaming

    The renderer composes the final view by:

     1. Creating a 1920×1080 canvas filled with the background color (if specified)


    AV2 Specification                                                                               Page 1159 of 1169
 2. Decoding each extended layer independently:

       ◦ Extended layer 0 → decoded to 1280×1080 (matches atlas segment)
       ◦ Extended layer 1 → decoded to 480×360 (requires resampling)
       ◦ Extended layer 2 → decoded to 640×540 (matches atlas segment)
 3. Resampling for resolution mismatch (Extended layer 1):

       ◦ Decoded resolution: 480×360
       ◦ Target atlas segment: 640×540
       ◦ Resampling required: upscale by factor of 4/3 horizontally and 3/2 vertically
       ◦ The resampling process is implementation-dependent and outside the scope of this specification.
         One example approach:

           1. Initialize resampled frame buffers (640×540 for Y plane, with appropriate chroma dimensions
              based on subsampling format)
           2. For each output sample position (x, y) in the resampled frame:

                    ▪ Calculate corresponding input position: inputX = x × (inputWidth / outputWidth), inputY
                      = y × (inputHeight / outputHeight)
                    ▪ Apply interpolation filter (e.g., bilinear, bicubic, or Lanczos) using neighboring input
                      samples
                    ▪ Store result in resampled frame buffer
           3. Repeat for U and V chroma planes with subsampling-aware calculations
       ◦ Note: This is one possible implementation. Decoders may use different resampling algorithms
         (nearest-neighbor, bilinear, bicubic, Lanczos, learned upsampling, etc.) based on quality-
         performance tradeoffs
 4. Positioning decoded (and resampled) content according to atlas layout:

       ◦ Layer 0 at position (0, 0) with size 1280×1080 (cumulative: col 0 start, rows 0-1 span)
       ◦ Layer 1 (after resampling to 640×540) at position (1280, 0) (cumulative: col 0 width=1280, row 0
         start)
       ◦ Layer 2 at position (1280, 540) with size 640×540 (cumulative: col 0 width=1280, row 0
         height=540)
 5. Compositing all layers onto the canvas to produce the final 1920×1080 output

Adaptive streaming benefits: This structure enables intelligent bandwidth adaptation:

  • On bandwidth constraints, selectively decode layers (e.g., only main speaker if needed)
  • Different layers can have different quality/resolution/framerate
  • Layout can be reconfigured dynamically by sending new atlas OBUs
  • Participants can join/leave by adding/removing extended layers




AV2 Specification                                                                                   Page 1160 of 1169
    Selective decoding: A mobile client with limited screen space might:

      • Only decode extended layer 0 (main speaker) and display it full-screen
      • Skip decoding layers 1 and 2 to save bandwidth and power
      • The decoder knows this is valid because each extended layer is independently decodable



                                                                      Subpicture Composition - Video Conferencing Layout

                                                                                 Atlas Canvas: 1920 x 1080 pixels (virtual composite screen)




                                                                                                                                                                        Participant 2
                                                                                                                                                                        Extended Layer 1                     540 px
                                                                                                                                                                  Segment 1: atlas_segment_id = 1
                                                                                                                                                                         Position: (1280, 0)
                                                                                                                                                                       Size: 640 x 540 pixels
                                                                                                                                                                         Medium resolution
                                                                      Main Speaker
                                                                       Extended Layer 0
                                                               Segment 0: atlas_segment_id = 0
                                                                          Position: (0, 0)
                                                                     Size: 1280 x 1080 pixels
                                                                    High resolution, high bitrate
                                                                                                                                                                        Participant 3
                                                                                                                                                                        Extended Layer 2                     540 px
                                                                                                                                                                  Segment 2: atlas_segment_id = 2
                                                                                                                                                                       Position: (1280, 540)
                                                                                                                                                                       Size: 640 x 540 pixels
                                                                                                                                                                         Medium resolution




                                                                                1280 px                                                                                       640 px
          Benefits:                                                                                           Atlas Mode:
          • Independent decoding - decode only visible participants                                           Mode 0 (Enhanced Atlas)
          • Adaptive quality - different bitrates per participant                                             Global LCR references atlas_id = 0
          • Dynamic layout - atlas can be updated without re-encoding streams                                 Each layer has local LCR with segment association


    Figure G.2: Subpicture composition for video conferencing. The atlas defines a 1920x1080 virtual canvas
    with three segments: main speaker (1280x1080) at left, and two participants (640x540 each) positioned
    on the right. Each segment maps to an independent extended layer that can be selectively decoded.



```

<a id="s-annex-g-4"></a>

### Annex G.4 Region-of-interest scalability example with encoder padding

```text
§   G.4.Region-of-interest scalability example with encoder padding
    This example demonstrates a stadium sports broadcast where a high-resolution field-of-play region is
    encoded separately from lower-resolution audience/stadium context. Additionally, this example shows
    how encoder padding and normative cropping work when the encoder needs to operate on dimensions
    that differ from the display resolution for hardware or algorithmic reasons.

    The content uses two extended layers:

      • Extended layer 0: Full stadium view at base quality (1920×1080 display, coded as 1920×1088 with
        padding)
      • Extended layer 1: Field-of-play region at high quality (1280×720)




    AV2 Specification                                                                                                                                                                           Page 1161 of 1169
```

<a id="s-annex-g-4-1"></a>

#### Annex G.4.1 LCR configuration

```text
§   G.4.1.LCR configuration

     // Extended layer 0 (base layer - full stadium with padding and cropping)
     lcr_local_atlas_id_present_flag[0] = 1 // Enable atlas segment assignment
     lcr_local_atlas_id[0] = 0              // References atlas with atlas_segment_id = 0
     lcr_layer_type[0][0][0] = TEXTURE_LAYER (0)
     lcr_view_type[0][0][0] = VIEW_CENTER (1)
     lcr_layer_atlas_segment_id[0][0][0] = 0 // Maps to atlas segment 0 (full 1920×1080 canvas)
     lcr_priority_order[0][0][0] = 0          // Rendered first (background)
     lcr_rendering_method[0][0][0] = 0        // Overwrite

     // Encoder padding and cropping for extended layer 0:
     // - Original video: 1920×1080
     // - Encoder operates on: 1920×1088 (padded to align with 64×64 superblocks)
     // - Cropping window removes padding to produce 1920×1080 output
     lcr_max_pic_width[0][0] = 1920
     lcr_max_pic_height[0][0] = 1088       // Coded height (with padding)
     lcr_cropping_window_present_flag[0][0] = 1
     lcr_cropping_win_left_offset[0][0] = 0
     lcr_cropping_win_right_offset[0][0] = 0
     lcr_cropping_win_top_offset[0][0] = 0
     lcr_cropping_win_bottom_offset[0][0] = 8 // Remove 8 pixels of bottom padding

     // After cropping: 1920×1080 (matches atlas segment 0 dimensions)

     // Extended layer 1 (enhancement - field detail, no padding needed)
     lcr_local_atlas_id_present_flag[1] = 1
     lcr_local_atlas_id[1] = 0
     lcr_layer_type[0][1][0] = TEXTURE_LAYER (0)
     lcr_view_type[0][1][0] = VIEW_CENTER (1)
     lcr_layer_atlas_segment_id[0][1][0] = 1 // Maps to atlas segment 1 (center cell, 1280×720)
     lcr_priority_order[0][1][0] = 1          // Rendered second (overlays base in center region)
     lcr_rendering_method[0][1][0] = 0        // Overwrite (replaces base layer data in center)
     lcr_max_pic_width[0][1] = 1280
     lcr_max_pic_height[0][1] = 720
     lcr_cropping_window_present_flag[0][1] = 0 // No cropping needed


    Note on cropping semantics:

      • Cropping is normative and must be applied to generate the conformant output
      • The cropped dimensions determine what maps to the atlas segment
      • Cropping happens after any upscaling specified by lcr_max_pic_width/height
      • The purpose is encoder implementation convenience (e.g., superblock alignment), not bandwidth
        savings
      • All coded samples (including padding) must be signaled and decoded

```

<a id="s-annex-g-4-2"></a>

#### Annex G.4.2 Atlas configuration

```text
§   G.4.2.Atlas configuration

    The atlas uses mode 0 (enhanced atlas) with a 3-column × 3-row non-uniform grid sized so the center cell
    exactly matches the field-of-play region (1280×720 at position 320,180). Two segments are defined:
    segment 0 spans all 9 grid regions (full 1920×1080 canvas) and segment 1 spans only the center cell.
    Segments 0 and 1 overlap on the center region; the LCR’s lcr_priority_order values (base=0,
    enhancement=1) control rendering order so the field enhancement overwrites the base layer in that
    region.

     // atlas_segment_info_obu() - OBU with obu_xlayer_id = GLOBAL_XLAYER_ID (31)
     atlas_segment_id[31] = 0           // xAId = 0
     ats_atlas_segment_mode_idc[0] = 0 // ENHANCED_ATLAS

     // ats_enhanced_atlas_info(xAId=0) [wrapper defined in companion normative PR] calls ats_region_info then



    AV2 Specification                                                                               Page 1162 of 1169
     ats_region_to_segment_mapping:

     // ats_region_info(xAId=0): 3×3 non-uniform grid
     // Columns: 320 + 1280 + 320 = 1920, Rows: 180 + 720 + 180 = 1080
     ats_num_region_columns_minus_1[0] = 2   // 3 columns
     ats_num_region_rows_minus_1[0] = 2      // 3 rows
     ats_uniform_spacing_flag[0] = 0        // Non-uniform spacing
     ats_column_width_minus_1[0][0] = 319    // Column 0: 320 px (left border)
     ats_column_width_minus_1[0][1] = 1279   // Column 1: 1280 px (field width)
     ats_column_width_minus_1[0][2] = 319    // Column 2: 320 px (right border)
     ats_row_height_minus_1[0][0] = 179      // Row 0: 180 px (top border)
     ats_row_height_minus_1[0][1] = 719      // Row 1: 720 px (field height)
     ats_row_height_minus_1[0][2] = 179      // Row 2: 180 px (bottom border)
     // AtlasWidth = 320+1280+320 = 1920, AtlasHeight = 180+720+180 = 1080

     // ats_region_to_segment_mapping(xAId=0)
     ats_single_region_per_atlas_segment_flag[0] = 0
     ats_num_atlas_segments_minus_1[0] = 1    // 2 segments

     // Segment 0: Full frame base layer (all 9 regions → 1920×1080)
     ats_top_left_region_column[0][0] = 0
     ats_top_left_region_row[0][0] = 0
     ats_bottom_right_region_column_off[0][0] = 2   // Spans to column 2
     ats_bottom_right_region_row_off[0][0] = 2      // Spans to row 2

     // Segment 1: Field-of-play enhancement (center cell only → 1280×720)
     // Canvas top-left: x=320 (col 0 width), y=180 (row 0 height)
     ats_top_left_region_column[0][1] = 1
     ats_top_left_region_row[0][1] = 1
     ats_bottom_right_region_column_off[0][1] = 0
     ats_bottom_right_region_row_off[0][1] = 0


```

<a id="s-annex-g-4-3"></a>

#### Annex G.4.3 Rendering scenarios

```text
§   G.4.3.Rendering scenarios

    Decoding and cropping process for extended layer 0:

     1. Decode extended layer 0 to produce a 1920×1088 frame (coded dimensions)
     2. Apply normative cropping as specified in LCR:

           ◦ Input frame: 1920×1088
           ◦ Cropping window: left=0, right=0, top=0, bottom=8
           ◦ Output frame calculation:

               cropWidth = lcr_max_pic_width - (lcr_cropping_win_left_offset + lcr_cropping_win_right_offset)
                         = 1920 - (0 + 0) = 1920
               cropHeight = lcr_max_pic_height - (lcr_cropping_win_top_offset + lcr_cropping_win_bottom_offset)
                          = 1088 - (0 + 8) = 1080

           ◦ Cropped output: 1920×1080
     3. The cropped frame (1920×1080) is what maps to atlas segment 0

    Full quality rendering (high bandwidth):

     1. Decode extended layer 0 (produces 1920×1088, normatively cropped to 1920×1080)
     2. Decode extended layer 1 (produces 1280×720, no cropping)
     3. Render layer 0 as background (1920×1080)




    AV2 Specification                                                                                 Page 1163 of 1169
 4. Place the field enhancement (segment 1) at position (320, 180) — derived from the atlas grid: x =
    column 0 width = 320 px, y = row 0 height = 180 px. The field enhancement overwrites the base
    layer in this region (lcr_rendering_method = 0, lcr_priority_order = 1)
 5. Result: Full 1920×1080 output with high-quality field region

Why use padding and cropping:

  • Hardware alignment: Many hardware encoders require dimensions aligned to specific boundaries
    (e.g., 64×64 superblocks, 32×32 transform units)
  • Algorithmic efficiency: Some encoding algorithms work more efficiently on certain dimension
    multiples
  • Example: 1920×1080 → pad to 1920×1088 (17 rows of 64-pixel superblocks), then crop back
  • Not for bandwidth savings: All padded samples must still be coded and transmitted

Bandwidth-constrained rendering:

  • Option A: Decode only layer 0 (with cropping applied) → full stadium at 1920×1080 base quality
  • Option B: Decode only layer 1 → high-quality field-of-play (1280×720) without stadium context

Atlas mapping considerations:

  • Atlas segment dimensions reference the cropped output dimensions, not the coded dimensions
  • In this example: atlas segment 0 is 1920×1080 (after cropping), not 1920×1088
  • Cropping must be applied before spatial mapping to the atlas canvas




AV2 Specification                                                                         Page 1164 of 1169
                                                            Region-of-Interest Scalable Encoding - Sports Broadcast

              Full Scene Output (1920x1080)
                                                                                                                                                                                     Atlas Mode:
                                                                                                                                                                                     Mode 0 (Enhanced Atlas)
                                                                                                                                                                                     LCR Config (Layer 0):
                                                                                     Stadium / Audience Context                                                                          type=TEXTURE
                                                                                                                                                                                         segment_id=0
                                                                                   (Extended Layer 0 - Base Quality)
                                                                                                                                                                                         priority=0 (background)
                           Overlay Region                                                                                                                                            LCR Config (Layer 1):
                                                                                                                                                                                         type=TEXTURE
                                                                                                                                                                                         segment_id=1
                                                                                                                                                                                         priority=1 (foreground)

                                                                                                                                                                                     Use Cases:
                                                                                                                                                                                     • Zoom into field
                                                                                                                                                                                     • Adaptive quality
                                                                                                                                                                                     • Bandwidth save
                                                                                                                                                                                     • Viewport-based
                                                                                 Field-of-Play (High Quality)
                                                                                           Extended Layer 1                                                                                1080 px

                                                                                          Enhancement Layer
                                                                                          Position: (320, 180)
                                                                                        Size: 1280 x 720 pixels
                                                                                      Higher bitrate, better quality




                                                                                                1920 pixels
    Rendering Scenarios:
       High Bandwidth: Decode both layers (base + enhancement)       Constrained A: Decode only base (full scene, lower quality)   Constrained B: Decode only enhancement (field only)


    Figure G.3: Region-of-interest scalable encoding for sports broadcast. Extended layer 0 provides full
    stadium view at base quality (1920×1080 after normative cropping from coded 1920×1088 dimensions).
    Extended layer 1 provides high-quality 1280×720 field-of-play that overlays the center region. This
    example demonstrates encoder padding (8 pixels for superblock alignment) with LCR cropping window to
    produce conformant output. Decoders can selectively decode layers based on viewport and bandwidth.



```

<a id="s-annex-g-5"></a>

### Annex G.5 Implementation considerations

```text
§   G.5.Implementation considerations
```

<a id="s-annex-g-5-1"></a>

#### Annex G.5.1 Decoder requirements

```text
§   G.5.1.Decoder requirements

    Decoders implementing LCR and atlas support should:

      1. Parse and validate LCR metadata:

               ◦ Verify layer type and auxiliary type combinations are valid
               ◦ Check view ID consistency across layers belonging to same view
               ◦ Validate atlas segment ID references
      2. Parse and interpret atlas layout:

               ◦ Support all required atlas modes
               ◦ Calculate final canvas dimensions and segment positions
               ◦ Handle segment overlays correctly (later segments may overlay earlier ones)




    AV2 Specification                                                                                                                                                      Page 1165 of 1169
     3. Selective decoding:

           ◦ Use Operating Point Set (OPS) information in combination with LCR to determine which layers
             are required for a given operating point
           ◦ Support independent decoding of extended layers
           ◦ Implement bandwidth-adaptive layer selection based on LCR metadata
     4. Multi-view rendering:

           ◦ Group layers by lcr_view_id for multi-view display
           ◦ Associate auxiliary data (alpha, depth, gain map) with correct texture layers
           ◦ Support stereoscopic display modes when VIEW_LEFT/VIEW_RIGHT layers are present

```

<a id="s-annex-g-5-2"></a>

#### Annex G.5.2 Encoder recommendations

```text
§   G.5.2.Encoder recommendations

    Encoders should:

     1. Choose appropriate layer structure:

           ◦ Use extended layers for independently decodable streams (different views, different regions)
           ◦ Use embedded layers for scalability within a single view (quality/temporal scalability)
           ◦ Balance granularity vs. overhead (more layers = more flexibility but more metadata)
     2. Populate LCR metadata accurately:

           ◦ Set lcr_layer_type and lcr_auxiliary_type to reflect actual content
           ◦ Use consistent lcr_view_id values for layers belonging to same view
           ◦ Associate layers with appropriate atlas segments via lcr_layer_atlas_segment_id
     3. Design atlas layouts efficiently:

           ◦ Choose atlas mode appropriate for use case (mode 0/1 for regular grids, mode 2/3 for flexible
             layouts)
           ◦ Minimize canvas size to reduce padding and memory requirements
           ◦ Consider decoder memory constraints when designing segment layouts
     4. Provide Operating Point Sets:

           ◦ Define OPS entries for common playback scenarios (mono vs. stereo, with/without depth,
             different quality levels)
           ◦ Include profile/tier/level information in OPS for conformance checking
           ◦ Reference atlas segments in OPS where applicable

```

<a id="s-annex-g-5-3"></a>

#### Annex G.5.3 Interoperability

```text
§   G.5.3.Interoperability

    For maximum interoperability:

     1. Legacy decoder fallback:

           ◦ Ensure extended layer 0, embedded layer 0 contains playable base content



    AV2 Specification                                                                            Page 1166 of 1169
           ◦ Decoders that ignore LCR/atlas should still get reasonable output
           ◦ Use sequence header to signal when advanced features are required
     2. Progressive enhancement:

           ◦ Structure layers so additional data enhances rather than replaces base content
           ◦ Design atlas layouts that degrade gracefully if not all segments are decoded
     3. Signaling and discovery:

           ◦ Use content interpretation metadata (CIMD) to signal presence of stereo/depth/HDR
           ◦ Include sufficient LCR information for clients to discover available views and auxiliary data types
           ◦ Document expected rendering behavior in supplementary information

```

<a id="s-index"></a>

## Index

```text
§   Index
```

<a id="s-terms-defined-by-this-specification"></a>

## Terms defined by this specification

```text
§   Terms defined by this specification

        AC coefficient, in § 2              Coded multistream video            frame_header_info, in § 2
                                            sequence, in § 2
        ADST, in § 2                                                           FSC, in § 2
                                            coded video sequences, in § 2
        AOMedia, in § 2                                                        GDF, in § 2
                                            Component, in § 2
        Atlas, in § 2                                                          Global operating point set, in § 2
                                            Compound prediction, in § 2
        Base layer, in § 2                                                     IBP, in § 2
                                            DC coefficient, in § 2
        BAWP, in § 2                                                           Inter coding, in § 2
                                            DCT, in § 2
        Bitstream, in § 2                                                      Inter frame, in § 2
                                            DDT, in § 2
        Bit string, in § 2                                                     Interoperability point, in § A.2
                                            Deblocking filter, in § 2
        blocks, in § 2                                                         Inter prediction, in § 2
                                            Decoded frame, in § 2
        Bridge frame, in § 2                                                   Intra coding, in § 2
                                            Decoder, in § 2
        BRU, in § 2                                                            Intra frame, in § 2
                                            Decoding process, in § 2
        Byte, in § 2                                                           Intra prediction, in § 2
                                            Dequantization, in § 2
        Byte alignment, in § 2                                                 Inverse transform, in § 2
                                            Embedded layer, in § 2
        CCSO, in § 2                                                           IST, in § 2
                                            Encoder, in § 2
        CCTX, in § 2                                                           Key frame, in § 2
                                            Encoding process, in § 2
        CDEF, in § 2                                                           Layer, in § 2
                                            Enhancement layer, in § 2
        CDF, in § 2                                                            LCR, in § 2
                                            EOB, in § 2
        CFL, in § 2                                                            Leading frame, in § 2
                                            extended layers, in § 2
        Chroma, in § 2                                                         Level, in § 2
                                            Frame, in § 2
        CLK, in § 2                                                            LF, in § 2
                                            Frame context, in § 2
        Closed random access, in § 2                                           Local operating point set, in § 2
                                            Frame header info, in § 2
        Coded frame, in § 2




    AV2 Specification                                                                                Page 1167 of 1169
        Long-term reference frame, in      Prediction, in § 2               Sequence, in § 2
        §2
                                           Prediction process, in § 2       Singlestream, in § 2
        LSB, in § 2
                                           Prediction value, in § 2         Sub-bitstream, in § 2
        Luma, in § 2
                                           Profile, in § 2                  Sub-bitstream extraction
        MHCCP, in § 2                                                       process, in § 2
                                           Quantization parameter, in § 2
        Mode info, in § 2                                                   Superblock, in § 2
                                           Quantized coefficient, in § 2
        Mode info block, in § 2                                             Switch Frame, in § 2
                                           Random access switch, in § 2
        Motion vector, in § 2                                               Syntax element, in § 2
                                           RAS, in § 2
        MRL, in § 2                                                         TCQ, in § 2
                                           Raster scan, in § 2
        MSB, in § 2                                                         Temporal delimiter OBU, in § 2
                                           Reconstruction, in § 2
        MSDO, in § 2                                                        Temporal layer, in § 2
                                           Reference, in § 2
        multi-sequence configuration, in                                    temporal units, in § 2
                                           Reference frame, in § 2
        § A.3
                                                                            TG, in § 2
                                           Regular frame, in § 2
        Multistream, in § 2
                                                                            Tier, in § 2
                                           Reserved, in § 2
        OBU, in § 2
                                                                            Tile, in § 2
                                           Residual, in § 2
        OLK, in § 2
                                                                            Tile Group, in § 2
                                           Sample, in § 2
        Open random access, in § 2
                                                                            TIP, in § 2
                                           Sample value, in § 2
        OPS, in § 2
                                                                            Transform block, in § 2
                                           SDP, in § 2
        Parity hiding, in § 2
                                                                            Transform coefficient, in § 2
                                           SEF, in § 2
        Parse, in § 2
                                                                            WAIP, in § 2
                                           Segmentation map, in § 2
        Picture, in § 2
                                                                            WHT, in § 2


```

<a id="s-references"></a>

## References

```text
§   References
```

<a id="s-normative-references"></a>

## Normative References

```text
§   Normative References
    [CTA-861]
       A DTV Profile for Uncompressed High Speed Digital Interfaces (ANSI/CTA-861-J). standard. URL:
       https://www.cta.tech/standards/a-dtv-profile-for-uncompressed-high-speed-digital-interfaces/
    [RFC1321]
       R. Rivest. The MD5 Message-Digest Algorithm. April 1992. Informational. URL: https://www.rfc-
       editor.org/rfc/rfc1321
    [RFC2119]
       S. Bradner. Key words for use in RFCs to Indicate Requirement Levels. March 1997. Best Current
       Practice. URL: https://datatracker.ietf.org/doc/html/rfc2119




    AV2 Specification                                                                            Page 1168 of 1169
```

<a id="s-informative-references"></a>

## Informative References

```text
§   Informative References
    [ITU-R-BT.601]
       Recommendation ITU-R BT.601-7 (03/2011), Studio encoding parameters of digital television for
       standard 4:3 and wide screen 16:9 aspect ratios. 8 March 2011. Recommendation. URL: https://
       www.itu.int/rec/R-REC-BT.601/
    [ITU-R-BT.709]
       Recommendation ITU-R BT.709-6 (06/2015), Parameter values for the HDTV standards for production
       and international programme exchange. 17 June 2015. Recommendation. URL: https://www.itu.int/
       rec/R-REC-BT.709/
    [Rec.2020]
       Recommendation ITU-R BT.2020-2: Parameter values for ultra-high definition television systems for
       production and international programme exchange. October 2015. URL: http://www.itu.int/rec/R-
       REC-BT.2020/en




    AV2 Specification                                                                       Page 1169 of 1169

```
