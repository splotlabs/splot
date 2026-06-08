# AV2 v1.0.0 — Annex E: Decoder model

<!-- Verbatim mirror of the AOM AV2 v1.0.0 specification (© Alliance for Open Media). The PDF is normative; this is a faithful `pdftotext -layout` copy. See [./README.md](./README.md) and [./index.md](./index.md). Do not hand-edit: regenerate via scripts/spec/regenerate-av2-spec.sh. -->

<a id="s-annex-e"></a>

## Annex E: Decoder model

```text
§   Annex E: Decoder model
§   E.1.General
    The decoder model is used to verify that a bitstream, sub-bitstream or an operating point can be decoded
    within the constraints imposed by one of the coding levels defined in Annex A.4 Levels. The decoder
    model is also used to verify conformance for a decoder that claims conformance to a certain coding level.

    A set of decoder model parameters may be optionally specified for extended layers or for zero or more
    operating points. If the new Sequence Header OBU does not signal decoder model parameters for an
    extended layer, the previous set of decoder model parameters does not persist. If the new Operating
    Point Set OBU does not signal decoder model parameters for a given operating point, the previous set of
    decoder model parameters does not persist.

    The decoder model constraints are checked for each extended layer independently. When a bitstream
    includes multiple operating points, the decoder model constraints are verified for each operating point
    and extended layer independently against its own decoder model parameters (BitRate, BufferSize,
    DecoderBufferDelay, EncoderBufferDelay) as signaled in the seq_decoder_model_info() or
    ops_decoder_model_info() and updated, if necessary, according to section Annex A.4 Levels. If the
    decoder model is verified for a certain operating point or a certain extended layer, the corresponding
    profile, level and tier are used to set the decoding model parameters.


      NOTE: The variables MaxDisplayRate, MaxDecodeRate, and BitRate depend on the value of
      variable MultiStreamDecoderMode, which is set in § 7.4.1 General and used to adjust level variables
      in Annex A.4 Levels.


    The decoder model describes the smoothing buffer, decoding process, operation of the frame buffers and
    the frame output process.

    The decoder model can be applied to an extended layer. The decoder model parameters for an extended
    layer take into account all embedded layers within that extended layer that are necessary for decoding
    the extended layer.

    The decoder model can be applied to an operating point. An operating point can specify the decoder
    model that allows establishing conformance to the level signaled for this operating point.

    The decoder model defines two modes of operation. A conformant bitstream shall satisfy constraints
    imposed by one of these two modes of the decoder model depending on which mode is applicable.

    Annex E.2 Operating point selection describes how the operating point is selected for the decoder model.

    Annex E.3 Decoder model definitions defines additional concepts used by the decoder model.

    Annex E.4 Operating modes defines the operating modes.

    Annex E.5 Frame timing definitions specifies how the frame timings can be computed in the different
    operating modes.

    Annex E.6 Decoder model specifies the decoder model process.




    AV2 Specification                                                                          Page 1122 of 1169
    Annex E.7 Bitstream conformance specifies the conformance requirements.

§   E.2.Operating point selection
    The decoder model process is performed for an extended layer or for a certain operating point. The
    decoder model is applied to each extended layer independently. If an operating point includes more than
    one extended layer, the decoder model is checked for each extended layer independently. When an
    extended layer conformance is checked by the decoder model, the OBUs related to this extended layer
    are taken into account by the decoder model, whereas the OBUs not related to this extended layer are
    not taken into account by the decoder model.

    The operating point is selected by choosing an operating points set ops_id and an operating point op
    within the operating point set. When the operating point op conformance is checked by the decoder
    model for a certain extended layer with id xId, the OBUs related to the operating point set ops, the
    operating point op, and this extended layer xId are taken into account by the decoder model, whereas the
    OBUs not related to the operating point op in the operating point set ops and the extended layer xId are
    not taken into account by the decoder model. When the decoder model is applied to the entire extended
    layer xId, the entire extended layer is treated as an operating point, whereas the decoder model
    parameters may be conveyed in the sequence header associated with this extended layer, in an operating
    point OBU or delivered by external means.

    The decoder model parameters are defined as follows.

    When the decoder model is applied to the whole extended layer xId, the parameters DecoderBufferDelay,
    EncoderBufferDelay, and LowDelayMode are defined as follows:

      • DecoderBufferDelay is assigned the value of decoder_buffer_delay.
      • EncoderBufferDelay is assigned the value of encoder_buffer_delay.
      • LowDelayMode is assigned the value of low_delay_mode_flag.

    Otherwise, when the parameters for the operating point op in the operating point set ops and xlayer xId
    are present, and the operating point op is selected, parameters DecoderBufferDelay, EncoderBufferDelay,
    and LowDelayMode are defined as follows:

      • DecoderBufferDelay is assigned the value of ops_decoder_buffer_delay[ xId ][ ops ][ op ].
      • EncoderBufferDelay is assigned the value of ops_encoder_buffer_delay[ xId ][ ops ][ op ].
      • LowDelayMode is assigned the value of ops_low_delay_mode_flag[ xId ][ ops ][ op ].

§   E.3.Decoder model definitions
    The decoder model uses the following elements to verify bitstream conformance that are not part of the
    decoding process specified in § 7 Decoding process.


      NOTE: The elements defined in this section do not have to be present in a conformant decoder
      implementation. These elements may be considered examples of elements of a conformant decoder,
      although the actual decoder implementation may differ. The elements are defined for the extended
      layer, which is used by the selected operating point.




    AV2 Specification                                                                          Page 1123 of 1169
BufferPool is a storage area for a set of frame buffers. Buffer pool area allocated for storing separate
frames is defined as BufferPool[ i ], where i takes values from 0 to NumRefFrames + 1. When a frame
buffer is used for storing a decoded frame, it is indicated by a VBI slot that points to this frame buffer.

VBI (virtual buffer index) is an array of indices of the frame areas in the BufferPool. VBI elements which
do not point to any slot in the BufferPool are set to -1. VBI array size is equal to NumRefFrames, with the
indices taking values from 0 to NumRefFrames - 1.

Cfbi (current frame buffer index) is the variable that contains the index to the area in the BufferPool that
contains the current frame.

DecoderRefCount[ i ] is a variable associated with a frame buffer i. DecoderRefCount[ i ] is initialized
to 0, and incremented by 1 each time the decoder adds the buffer i to a VBI index slot. It is decremented
by 1 each time the decoder removes the buffer from a VBI index slot i. The decoder may update multiple
VBI index slots with the same frame buffer, as specified by refresh_frame_flags, so the counter may be
incremented several times. When the counter is 0 the pixel data becomes permanently invalid and shall
not be used by the decode process.

PlayerRefCount[ i ] is a variable associated with a frame buffer i. PlayerRefCount[ i ] is initialized to 0,
incremented by 1 each time the decoder determines that the frame is a presentation frame. It is reset to
0 after the last time the frame is presented.

PresentationTimes[ i ] is an array corresponding to the BufferPool [ i ] that holds the last presentation
time for the decoded frame that is kept in the BufferPool [ i ].




AV2 Specification                                                                              Page 1124 of 1169
Figure E.1: Example of how the coded frame buffer fullness varies as data arrives from the stream, and is
subsequently removed for decoding. Relevant timing points and values are indicated.



Coded frames arrive at the decoder smoothing buffer of the size BufferSize at a rate defined by BitRate.
The following variables are used in this section and below:

BitRate is set to a value equal to MaxBitrate * BitrateProfileFactor specified for the level signaled for the
operating point or an extended layer that is being decoded.

BufferSize is set to a value equal to MaxBufferSize * BitrateProfileFactor value specified for the level
signaled for the operating point that is being decoded.

Decodable Frame Group i (DFG i) consists of all OBUs, including headers, between the end of the last
OBU associated with the previous frame with ShowExistingFrame flag equal to 0 (frame k), and the end
of the last OBU associated with the current frame with ShowExistingFrame flag equal to 0 (frame p). This
comprises the OBUs that make up frame p, plus any additional OBUs present in the bitstream that belong
to frame p (such as the metadata OBU), and OBUs that belong to frames with ShowExistingFrame flag
equal to 1 which are located between frame k and frame p. The decoder model assumes that the
decoding time for processing a frame with ShowExistingFrame flag equal to 1, a header, or a metadata
OBU is 0, hence the smoothing buffer operates in the units of DFG. The decoder model used to verify the
constraints for an extended layer xId only takes into account OBUs related to the extended layer xId. The
decoder model used to verify the constraints for an operating point op in the operating point set ops and
the extended layer xId only takes into account OBUs related to the operating point op in the operating
point set ops and the extended layer xId. The OBUs not related to the operating point op in the operating



AV2 Specification                                                                             Page 1125 of 1169
    point set ops and the extended layer xId should be omitted by the decoder model and not increase the
    value of the DFG index i.

    CodedBits[ i ] is the amount of data, in bits, that belongs to DFG i. Note that the index i of the DFG only
    increases with frames with ShowExistingFrame flag equal to 0, i.e., frames that need to be decoded by
    the decoding process.

    FirstBitArrival[ i ] is the time when the first bit of the i-th DFG starts entering the decoder smoothing
    buffer. For the first coded DFG in the sequence, DFG 0 (or after updating decoder model parameters at a
    random access point), FirstBitArrival[ 0 ] = 0.

    LastBitArrival[ i ] is the time when the last bit of DFG i finishes entering the smoothing buffer.

    Each output frame j has a scheduled presentation time, PresentationTime[ j ], defined to be a multiple
    of the display clock tick DispCT. The index j counts all output frames related to the operating point and/or
    extended layer in output order, including immediate output frames, frames with ShowExistingFrame
    equal to 1, and implicit output frames. These output frames may belong to one or more embedded layers.

    DispCT represents the expected time interval between displaying two consecutive frames, or a common
    divisor of the expected times between displaying two consecutive frames if the encoded bitstream has a
    variable display frame rate.

§   E.4.Operating modes
§   E.4.1.Resource availability mode

    In this mode the model simulates the operation of the decoder under the assumption that the complete
    coded frame is available in the smoothing buffer when decoding of that frame begins. In addition, it is
    assumed that the decoder will begin to decode a frame immediately after it finishes decoding the
    previous frame or when a frame buffer becomes available, whichever is later. This model uses the
    generated time moments when the decoding of a frame begins as times when the data is removed from
    the smoothing buffer to check the conformance of a bitstream to the bitrate specified for a level signaled
    for the operating point or an extended layer of a bitstream.

    To verify that a bitstream can be decoded by a decoder under the constraints of a particular level it is
    assumed that the decoder performs the decoding operations at maximum speed (the minimum time
    interval) specified for that level in Annex A.4 Levels.

    To use Resource Availability mode, the following parameters should be set in the encoded video
    bitstream:

      • ci_timing_info_present_flag equal to 1
      • ops_decoder_model_info_for_this_op_present_flag[ xId ][ ops ][ op ] equal to 0 and
        seq_decoder_model_info_present_flag equal to 0
      • equal_picture_interval equal to 1,

    where xId is the extended layer id for which conformance needs to be established, ops is the operating
    point set id and op is the selected operating point, and parameter seq_decoder_model_info_present_flag
    is signaled in the sequence header that is associated with the extended layer xId for which conformance
    is checked, and equal_picture_interval is signaled in the Content Interpretation OBU.




    AV2 Specification                                                                            Page 1126 of 1169
    If the parameters listed above are not specified by the bitstream, the parameters necessary to input into
    this model can be signaled by the application or some other means. If the parameters necessary to run
    this model are not signaled, it is not possible to check the conformance of the stream or an operating
    point to the claimed level.

    In this mode of operation, the decoder model parameters below take the following (default) values:

      • EncoderBufferDelay = 20 000
      • DecoderBufferDelay = 70 000
      • LowDelayMode = 0

    The decoder writes the decoded frame into one of the available frame buffers. Decoding must be delayed
    until a frame buffer becomes available.

§   E.4.2.Decoding schedule mode

    This mode imposes additional constraints relating to the operation of the smoothing buffer and the timing
    points, specified for each frame, defining exactly when the decoder should start decoding a frame and
    when that frame should be presented.

    To use Decoding Schedule Mode, the following parameters should be signaled by the encoded video
    bitstream:

      • ci_timing_info_present_flag equal to 1 in the content interpretation OBU associated with this
        extended layer
      • decoder_model_info_present_flag equal to 1
      • seq_decoder_model_info_present_flag equal to 1 or
        ops_decoder_model_info_for_this_op_present_flag[ xId ][ ops ][ op ],

    where xId is the extended layer, for which conformance needs to be established, ops is the selected
    operating point set and op is the selected operating point, and parameter
    seq_decoder_model_info_present_flag is signaled in the sequence header that is associated with the
    extended layer xId for which conformance needs to be established.

    When these flags are signaled, the bitstream should provide the associated information specified in
    seq_decoder_model_info( ) or ops_decoder_model_info( ), depending on if the parameters are signaled for
    the extended layer or an operating point.

    In addition, for each frame and each operating point op, the following parameters must be specified:

      • BufferRemovalTime
      • frame_presentation_time for each frame or equal_picture_interval set to 1 in the Content
        Interpretation OBU

    BufferRemovalTime is defined equal to

      • br_time when the decoder model is applied to the extended layer xId
      • or br_time_op[ ops ][ op ] when the decoding model is applied to an operating point set ops and
        operating point op.




    AV2 Specification                                                                          Page 1127 of 1169
      NOTE: The two cases above are mutually exclusive. When br_ops_dependent_flag is equal to 0 in
      the buffer_removal_timing_obu( ), only br_time is present and the decoder model is applied to the
      extended layer as a whole. When br_ops_dependent_flag is equal to 1, only br_time_op is present and
      the decoder model is applied per operating point within the specified operating point set.


    If the parameters listed above are not specified by the bitstream, the parameters necessary to input into
    this model can be signaled by the application or some other means. If the parameters necessary to input
    into this model are not signaled, it is not possible to check conformance of the stream to the claimed level
    with this model.

§   E.4.3.Establishing bitstream conformance

    When the parameters necessary for the decoding schedule mode are specified by the bitstream, extended
    layer or an operating point or signaled to the decoder by the application or some other means, the
    decoder schedule mode shall be used for establishing the bitstream conformance.

    When the parameters necessary for the decoding schedule mode are not available and the parameters
    necessary for the resource availability mode are specified by the bitstream, extended layer or an
    operating point or signaled to the decoder by the application or some other means, the resource
    availability mode shall be used for establishing the bitstream conformance.

§   E.4.4.When timing information is not present in the bitstream

    When the parameters necessary as the input to at least one of the operating modes specified in Annex E.4
    Operating modes, i.e., resource availability mode or decoding schedule mode, are not present in the
    bitstream, it is impossible to verify whether the bitstream satisfies the level constraints according to
    either of the decoder models. In order to enable verification of the bitstream conformance, the equivalent
    information necessary to verify the conformance can be provided by external means. Otherwise,
    conformance cannot be established.

§   E.5.Frame timing definitions
§   E.5.1.Start of DFG bits arrival

    The bits arrive in the smoothing buffer at a constant bitrate BitRate or the bitrate equal to 0. Hence, the
    average bitrate can be lower than the bitrate BitRate specified in the level definition, which, in this case,
    represents a peak bitrate. The first bit of DFG i is expected to arrive by the latest time that would
    guarantee timely reception of the entire DFG by the time when the decodable frame in the DFG i is due
    to be decoded:

     FirstBitArrival[ i ]   = max ( LastBitArrival[ i - 1 ], LatestArrivalTime[ i ] ),


    where LatestArrivalTime[ i ] is the latest time when the first bit of DFG i must arrive in the smoothing
    buffer to ensure that the complete DFG is available at the scheduled removal time, ScheduledRemoval




    AV2 Specification                                                                              Page 1128 of 1169
    [ i ], in units of seconds, unless the new set of decoding model parameters is received. In its turn, the
    latest time the DFG data should start being received is determined as follows:

     LatestArrivalTime[ i ]   = ScheduledRemoval[ i ] -
                                ( EncoderBufferDelay + DecoderBufferDelay ) ÷
                                90 000


§   E.5.2.End of DFG bits arrival

    For the bits that belong to the DFG i, the time of arrival of the last bit of the DFG i is determined as
    follows:

     LastBitArrival[ i ] = FirstBitArrival[ i ] + CodedBits[ i ] ÷ BitRate


§   E.5.3.Scheduled removal times

    The decoder starts to decode a frame exactly at the moment when the data corresponding to the DFG of
    that frame is removed from the smoothing buffer. Each DFG has a scheduled removal time and an actual
    removal time. Under certain circumstances these times may be different.

    The ScheduledRemoval[ i ] time is determined differently in the resource availability and the decoding
    schedule mode.

    When the decoder model operates in the decoding schedule mode

     ScheduledRemoval[ i ] = ScheduledRemovalTiming[ i ]


    When the decoder model operates in the resource availability mode

     ScheduledRemoval[ i ] = ScheduledRemovalResource[ i ]


    Derivation of ScheduledRemovalTiming[ i ] in the decoding schedule mode is described in Annex E.5.4
    Removal times in decoding schedule mode, and derivation of ScheduledRemovalResource[ i ] in the
    resource availability mode is described in Annex E.5.5 Removal times in resource availability mode.

§   E.5.4.Removal times in decoding schedule mode

    DFG i is scheduled for removal from the smoothing buffer at time ScheduledRemovalTiming [ i ] which is
    defined as an offset, BufferRemovalTime[ i ], signaled for the frame of the DFG with ShowExistingFrame
    equal to 0, relative to the moment of time when the first DFG is removed from the smoothing buffer,
    DecoderBufferDelay:

     ScheduledRemovalTiming[ 0 ] = DecoderBufferDelay ÷ 90 000

     ScheduledRemovalTiming[ i ] = ScheduledRemovalTiming[ PrevRap ] +
                                   BufferRemovalTime[ i ] * DecCT


    When i is not equal to 0 and frame i is associated with a random access point, PrevRap is the index
    associated with the previous random access point. Otherwise, if frame i is not associated with the random
    access point, PrevRap corresponds to the index associated with the most recent random access point.



    AV2 Specification                                                                              Page 1129 of 1169
    DFG i is removed from the smoothing buffer at time Removal[ i ].

    There are two modes of operation of a decoder which determine whether the actual DFG removal time
    Removal[ i ] may be different from the scheduled DFG removal timing ScheduledRemovalTiming [ i ]. As
    mentioned earlier, the decoder starts decoding a frame when the data that belongs to its DFG is removed
    from the smoothing buffer.

    In this mode, frame decoding start times / DFG removal times are determined by the BufferRemovalTime
    [ i ] for the chosen operating point, op or extended layer.

    If LowDelayMode is equal to 0, the decoder operates in Strict Arrival Mode, and DFG is removed from
    the smoothing buffer at the scheduled time, that is:

     Removal[ i ] = ScheduledRemovalTiming[ i ]


    Otherwise, LowDelayMode is equal to 1 and the decoder operates in Low-Delay Mode, where the DFG
    data may not be available in the smoothing buffer at the scheduled removal time, i.e.,
    ScheduledRemovalTiming[ i ] < LastBitArrival[ i ]. In that case, the removal of the DFG is deferred until
    the first decode clock tick after the complete DFG is present in the smoothing buffer, that is:

     Removal[ i ] = ceil ( LastBitArrival[ i ] ÷ DecCT ) * DecCT


    If the entire DFG is available in the smoothing buffer at the scheduled removal time, i.e.,
    ScheduledRemovalTiming[ i ] >= LastBitArrival[ i ], then it is removed at the scheduled time, that is:

     Removal[ i ]       =   ScheduledRemovalTiming[ i ]


§   E.5.5.Removal times in resource availability mode

    In the resource availability mode, BufferRemovalTime[ i ] are not signaled for the chosen operating point.
    In this mode, timing of the decoder model is driven by the availability of the resources in the decoder, in
    particular, by times when the decoding of the previous frame with ShowExistingFrame flag equal to 0 has
    been completed and a free frame buffer is available.

    In particular, ScheduledRemovalResource [ i ] times are generated as the earliest time that a non-
    assigned frame buffer becomes available for decoding of the frame i. In this mode, the decoder starts to
    decode a frame as fast as it can after completing decoding of the previous frame and a free frame buffer
    is available. A frame buffer is defined as being available if it is no longer being used and its content can
    be overwritten.

    Removal times in the resource availability mode are produced by Annex E.6.2 Decoder model functions.

    The following function, time_next_buffer_is_free, is used by the decode process to determine the
    Removal[ i ] time for the next DFG and generate the value of ScheduledRemovalResource[ i ].

     time_next_buffer_is_free ( i, time ) {
         if ( i == 0 ) {
             time = DecoderBufferDelay ÷ 90000
         }
         foundBuffer = 0
         for ( k = 0; k < NumRefFrames + 2; k++ ) {
             if ( DecoderRefCount[ k ] == 0 ) {



    AV2 Specification                                                                             Page 1130 of 1169
                    if ( PlayerRefCount[ k ] == 0 ) {
                        ScheduledRemovalResource[ i ] = time
                        return time
                    }
                    if ( !foundBuffer || PresentationTimes[ k ] < bufFreeTime ) {
                        bufFreeTime = PresentationTimes[ k ]
                        foundBuffer = 1
                    }
              }
          }
          ScheduledRemovalResource[ i ] = bufFreeTime
          return bufFreeTime
     }


§   E.5.6.Frame decode timing

    The time required to decode a frame (i.e., to process the decodable frame’s DFG), TimeToDecode [ i ], is
    calculated based on the frame type, a maximum number of luma pixels for the frame, and the throughput
    of the decoder as specified in the definition of the level assigned to the operating point or extended layer
    that the frame belongs to.

    The time that it takes the decoder to decode a frame according to the decoder model is estimated by
    using the function time_to_decode_frame( ) as follows.

     time_to_decode_frame( ) {
         if ( ShowExistingFrame == 1 ) {
             lumaSamples = 0
         } else if ( FrameIsIntra ) {
             if ( allow_global_intrabc == 1 && InloopFilteringEnabled == 1 )
                  lumaSamples = 2 * FrameWidth * FrameHeight
             else
                  lumaSamples = FrameWidth * FrameHeight
         } else {
             lumaSamples = ( max_frame_width_minus_1 + 1 ) *
                            ( max_frame_height_minus_1 + 1 )
         }
         return lumaSamples ÷ MaxDecodeRate
     }


§   E.5.7.Frame presentation timing

    When the decoder model is applied to the whole extended layer, InitialDisplayDelay is set to
    seq_initial_display_delay_minus_1 + 1.

    When the decoder model is applied to a chosen operating point, InitialDisplayDelay is set equal to
    ops_initial_display_delay_minus_1[ xId ][ ops ][ op ] + 1 if the ops_initial_display_delay_present_flag[ xId ]
    [ ops ][ op ] is equal to 1 for the current operating point and to seq_initial_display_delay_minus_1 + 1
    when ops_initial_display_delay_present_flag[ xId ][ ops ][ op ] is equal to 0 or is not specified for the
    current operating point.

    Initial presentation delay is determined as follows:

     InitialPresentationDelay =     Removal [ InitialDisplayDelay - 1 ] +
                                    TimeToDecode [ InitialDisplayDelay - 1 ]




    AV2 Specification                                                                              Page 1131 of 1169
    When equal_picture_interval is equal to 0, the decoder operates in variable frame rate mode, the frame
    presentation time is defined as follows:

     PresentationTime[ 0 ] = InitialPresentationDelay

     PresentationTime[ j ] = PresentationTime[ PrevPresent ] +
                             frame_presentation_time[ j ] * DispCT


    When j is not equal to 0 and frame j is associated with a leading frame or a random access point,
    PrevPresent corresponds to the index associated with the previous random access point. Otherwise,
    PrevPresent corresponds to the index associated with the last random access point.

    When equal_picture_interval is equal to 1, the decoder operates in the constant frame rate mode, and the
    frame presentation time is defined as follows:

     PresentationTime[ 0 ] = InitialPresentationDelay


    If frame j and frame j - 1 belong to the same temporal unit

     PresentationTime[ j ] = PresentationTime[ j - 1 ]


    Otherwise, if frame j and frame j - 1 belong to different temporal units

     PresentationTime[ j ] = PresentationTime[ j - 1 ] +
                             ( num_ticks_per_picture_minus_1 + 1 ) * DispCT ;


    where PresentationTime[ j - 1 ] refers to the previous frame in the output order, and j counts all output
    frames.

    The presentation interval, i.e., the time interval between the display of consecutive frames j and j + 1 in
    presentation order and when frames j and j + 1 belong to different temporal units is defined as follows:

     PresentationInterval[ j ] = PresentationTime[ j + 1 ] - PresentationTime[ j ]


§   E.6.Decoder model
§   E.6.1.Decoder model structure

    The decoder model simulates the values of selected timing points as successive frames are decoded. This
    includes the time that the decoder has to wait for a free frame buffer, the time required to decode the
    frame and various basic checks to make sure that buffer slots are occupied when they are supposed to
    be. Non-conformance is signaled by a call to the function bitstream_non_conformant; the various error
    codes are tabulated in Annex E.6.3 Decoder model error codes.

    To align the decoder model with the general decoding process and output frame management, the
    decoder model in AV2 is defined as running in parallel to the decoding process and relies on the decoding
    process functions for the reference frames management and frame output.




    AV2 Specification                                                                             Page 1132 of 1169
    In particular, the decoder model defines functions that are invoked at specified points of the
    corresponding functions and processes of § 7 Decoding process. This allows the decoder model to rely on
    variables and processes defined in the § 7 Decoding process and other parts of the specification.

    The proposed approach is used for convenience of the decoder model description and to avoid duplication
    of definitions of certain functions and processes. Other implementations of the decoder model may use a
    standalone approach that derives values of variables used by the decoder model without the use of the
    complete decode process.

§   E.6.2.Decoder model functions

    This section defines the buffer management functions invoked by the decoder model process.

    The free_buffer function clears the variables for a particular index in the BufferPool.

     free_buffer( idx ) {
         DecoderRefCount[ idx ] = 0
         PlayerRefCount[ idx ] = 0
         PresentationTimes[ idx ] =     -1
     }


    The initialize_buffer_pool function resets the BufferPool and the VBI.

     initialize_buffer_pool( ) {
         for ( i = 0; i < NumRefFrames + 2; i++ )
             free_buffer( i )
         for ( i = 0; i < NumRefFrames; i++ )
             VBI[ i ] = -1
     }


    The initialize_decoder_model function initializes the BufferPool related arrays and sets the decoder model
    variables to initial values. This function is called before the start of decoding an extended layer or an
    operating point. This function is also called during random access before the start of the decoding
    process.

     initialize_decoder_model( ) {
         initialize_buffer_pool( )
         Time = 0
         FrameNum = -1
         DfgNum = -1
         ShownFrameNum = -1
         Cfbi = -1
         InitialPresentationDelay = 0
     }


    The get_free_buffer function searches for an un-assigned frame in the BufferPool. The decoder needs an
    un-assigned frame buffer from the BufferPool for each frame that it decodes.


     get_free_buffer( ) {
         for ( i = 0; i < NumRefFrames + 2; i++ ) {
             if ( DecoderRefCount[ i ] == 0 &&
                  PlayerRefCount[ i ] == 0 )
                 return i




    AV2 Specification                                                                          Page 1133 of 1169
      }
      return -1
 }


In the decoding schedule mode, the decoder only starts to decode a frame at the time designated by a
removal time associated with that frame, and expects a free frame buffer to be immediately available.

In the resource availability mode, the decoder may start to decode the next frame as soon as a free
reference buffer is available. If a free frame buffer is not available immediately, the PresentationTimes[ i ]
may be used to compute the time when such a buffer will become available.

The function start_decode_at_removal_time returns buffers to the BufferPool when they are no longer
required for decode or display.

 start_decode_at_removal_time( removal ) {
     for ( i = 0; i < NumRefFrames + 2; i++ ) {
         if ( PlayerRefCount[ i ] > 0) {
             if ( PresentationTimes[ i ] <= removal ) {
                 PlayerRefCount[ i ] = 0
                 if ( DecoderRefCount[ i ] == 0 )
                     free_buffer( i )
             }
         }
     }
     return removal
 }


Function start_frame_decode is invoked at the start of the § 7.2 Decode frame wrapup process function in
the decoding process. Function start_frame_decode does not change the flow or the results of the § 7.2
Decode frame wrapup process. It uses the variables available to the decoding process at the start of the
§ 7.2 Decode frame wrapup process. In start_frame_decode, UsingResourceAvailabilityMode is a variable
that is set to 1 when using resource availability mode, or 0 when using decoding schedule mode.

 start_frame_decode( ){
     FrameNum++
     if ( !ShowExistingFrame ) {
         DfgNum++
         if ( UsingResourceAvailabilityMode )
             Removal [ DfgNum ] = time_next_buffer_is_free( DfgNum, Time )
         Time = start_decode_at_removal_time( Removal[ DfgNum ] )
         Cfbi = get_free_buffer( )
         if ( Cfbi == -1 )
             bitstream_non_conformant( DECODE_FRAME_BUF_UNAVAILABLE )
         Time += time_to_decode_frame( )
     }
 }


Once decoded, frames may update one or more of the VBI index slots, as defined by refresh_frame_flags.
Each time a VBI index slot is updated, the decoder reference count is incremented by 1 for the
corresponding frame buffer. If the VBI index slot being updated is currently occupied, the decoder
reference count for the frame buffer being displaced must be decremented by 1.

The update_ref_buffers function is called at the end of § 7.23 Reference frame update process. The
function update_ref_buffers function updates the VBI and reference counts when the reference frames
are updated, according to the results of § 7.23 Reference frame update process. This function mirrors the
results of the frame update process in § 7.23 Reference frame update process with respect to the decoder



AV2 Specification                                                                             Page 1134 of 1169
model variables. This function uses refresh_frame_flags of the current frame and the RefValid array
updated by § 7.23 Reference frame update process.

 update_ref_buffers ( ) {
     for ( i = 0; i < NumRefFrames; i++ ) {
         if ( (refresh_frame_flags >> i) & 1 ) {
             if ( VBI[ i ] != -1 )
                 DecoderRefCount[ VBI[ i ] ] --
             if( RefValid[ i ] ){
                 VBI[ i ] = Cfbi
                 DecoderRefCount[ Cfbi ] ++
             } else
                 VBI[ i ] = -1
         }
     }
 }


The decoder needs to know the number of decoded frames in the BufferPool in order to determine the
presentation delay for the first frame. A buffer is un-assigned if both DecoderRefCount[ i ] is equal to 0,
and PlayerRefCount[ i ] is equal to 0.

The function frames_in_buffer_pool returns the number of assigned frames in the BufferPool.

 frames_in_buffer_pool( ) {
     framesInPool = 0
     for ( i = 0; i < NumRefFrames + 2; i++ )
         if ( DecoderRefCount[ i ] != 0 || PlayerRefCount[ i ] != 0 )
             framesInPool++
     return framesInPool
 }


The function set_initial_presentation_delay is invoked during § 7.2 Decode frame wrapup process
immediately after the function § 7.23 Reference frame update process is invoked and has returned. The
function set_initial_presentation_delay initializes the InitialPresentationDelay.

 set_initial_presentation_delay( ){
     if ( !ShowExistingFrame ) {
         if ( InitialPresentationDelay == 0 &&
                 ( frames_in_buffer_pool( ) >=
                 InitialDisplayDelay ) )
             InitialPresentationDelay = Time
     }
 }


Function check_output_frame is invoked at the end of § 7.21.1 Output process. The function checks the
availability of the frames to be output, increases the output frame number and checks if the frames can
be output at their presentation time.

 check_output_frame( ){
     if ( frameToShowMapIdx == -1 ) {
         bufIdx = Cfbi
     } else {
         if ( !RefValid[ frameToShowMapIdx ] || VBI[ frameToShowMapIdx ] == -1 )
              bitstream_non_conformant( DECODE_EXISTING_FRAME_BUF_EMPTY )
         bufIdx = VBI[ frameToShowMapIdx ]
     }
     ShownFrameNum++
     PresentationTimes[ bufIdx ] = PresentationTime[ ShownFrameNum ]
     PlayerRefCount[ bufIdx ]++



AV2 Specification                                                                             Page 1135 of 1169
          if ( InitialPresentationDelay != 0 ) {
              if ( Time > PresentationTime[ ShownFrameNum ] )
                  bitstream_non_conformant( DISPLAY_FRAME_LATE )
          }
     }



      NOTE: PresentationTime[ ShownFrameNum ] includes the InitialPresentationDelay in its
      calculation. However, InitialPresentationDelay may be unknown until the number of frames in the
      buffer pool reaches InitialDisplayDelay. Depending on the implementation, PresentationTime of
      output frames may need to be updated when the InitialPresentationDelay is known.

§   E.6.3.Decoder model error codes

    The various non-conformant error codes are as specified in Table E.1:

                          Table E.1: Error codes produced by bitstream_non_conformant().

                    Error Codes                                                 Description

     DECODE_FRAME_BUF_UNAVAILABLE          All the frame buffers were in use.

     DECODE_EXISTING_FRAME_BUF_EMPTY       The buffer of the frame designated for display was empty.

     DISPLAY_FRAME_LATE                    The frame was decoded too late for timely display, i.e., by the PresentationTime[ i ]
                                           time associated with the frame.


§   E.7.Bitstream conformance
§   E.7.1.General

    A conformant coded bitstream shall satisfy the following set of constraints.

    For the decoder model, a DFG shall be available in the smoothing buffer at the scheduled removal time,
    i.e., ScheduledRemoval[ i ] >= LastBitArrival[ i ].

    It is a requirement of the bitstream conformance that after each random access point, the
    PresentationTime[ j ], where j corresponds to the frame output order (counting all output frames,
    including implicit output frames) is non-decreasing until the next random access point or the end of the
    coded video sequence, i.e., PresentationTime[ j + 1] >= PresentationTime[ j ].

    When BufferRemovalTime[ i ] is not specified in the bitstream, a bitstream is conformant if the decoder
    model in resource availability mode can decode frames successfully before they are scheduled for
    presentation.

    If BufferRemovalTime[ i ] is signaled, it shall have a value greater than or equal to the equivalent value
    that would have been assigned if the decoder model was decoding frames in the resource availability
    mode.

    It is a requirement of a bitstream conformance that a conformant bitstream is decodable according to the
    decoder model if the decoding starts from any of its random access points. This means that for a
    conformant bitstream, a bitstream produced from the conformant bitstream by removing the part of the
    bitstream preceding a random access point associated with an OBU_CLOSED_LOOP_KEY shall also be a
    conformant bitstream according to the decoder model.




    AV2 Specification                                                                                           Page 1136 of 1169
    For a conformant bitstream, a bitstream produced from the conformant bitstream by: 1) removing the
    part of the bitstream preceding a random access point associated with an OBU_OPEN_LOOP_KEY 2)
    removing the part of the bitstream corresponding to the leading frames following the
    OBU_OPEN_LOOP_KEY shall also be a conformant bitstream according to the decoder model.

    For a random access point associated with an OBU_RAS_FRAME, the bitstream shall also be a
    conformant bitstream according to the decoder model, provided that the long-term key frames are
    available at the specified frame buffer slots.

    Conformance requirements based on a decoder model are not applicable to a bitstream with
    seq_level_idx equal to 31.

    In addition to these, a conformant bitstream shall satisfy the constraints specified in the following
    sections.

§   E.7.2.Decoder buffer delay consistency across random access points (applies to decoding
    schedule mode)

    For frame i, where i > 0, TimeDelta[ i ] is defined as follows:

     TimeDelta[ i ] = ( ScheduledRemoval[ i ] - LastBitArrival[ i - 1 ] ) * 90 000


    For the video sequence that includes one or more random access points, for each random access point,
    where the DecoderBufferDelay is signaled, the following expression shall hold.

     DecoderBufferDelay <= ceil( TimeDelta[ i ] )


§   E.7.3.Smoothing buffer overflow

    Smoothing buffer overflow is defined as the state where the total number of bits in the smoothing buffer
    exceeds the size of the smoothing buffer BufferSize. The smoothing buffer shall never overflow.

§   E.7.4.Smoothing buffer underflow

    Smoothing buffer underflow is defined as the state where a complete DFG is not present in the smoothing
    buffer at the scheduled removal time, ScheduledRemoval [ i ]:

     ScheduledRemoval[ i ] < LastBitArrival[ i ]


    When the LowDelayMode is equal to 0, the smoothing buffer shall never underflow.

§   E.7.5.Minimum decode time (applies to decoding schedule mode)

    There must be enough time between a DFG being removed from the smoothing buffer, Removal[ i ], and
    the scheduled removal of the next DFG, ScheduledRemoval[ i + 1 ]:

     ScheduledRemoval[ i + 1 ] - Removal[ i ] >= Max( TimeToDecode[ i ],
                                                     1 ÷ MaxNumFrameHeadersPerSec ),


    where MaxNumFrameHeadersPerSec is defined in the level constraints.



    AV2 Specification                                                                             Page 1137 of 1169
§   E.7.6.Minimum presentation interval

    Variable numOutputFramesInTU [ j ] is equal to the number of output frames with the
    PresentationTime[ j ], in the temporal unit associated with the presentation time PresentationTime[ j ],
    that belong to the selected operating point op in the operating point set ops and / or extended layer xId,
    which may include frames that belong to different embedded layers.

    The difference between presentation times for consecutive shown frames or groups of shown frames that
    belong to different temporal units, shall satisfy the following constraint:

     MinFrameTime = MaxDecodeRate ÷ ( MaxNumFrameHeadersPerSec * MaxDisplayRate )

     PresentationInterval[ j ] >= Max( ( max_frame_width_minus_1 + 1 ) * ( max_frame_height_minus_1 + 1)
      * numOutputFramesInTU[ j ] ÷ MaxDisplayRate, MinFrameTime )


    Where MaxNumFrameHeadersPerSec is defined in the level constraints.

§   E.7.7.Decode deadline

    It is a requirement of the bitstream conformance that each frame shall be fully decoded at, or before, the
    time that it is scheduled for presentation:

     Removal[ i ] + TimeToDecode[ i ] <= PresentationTime[ i ]


§   E.7.8.Level imposed constraints

    When operating in the decoding schedule mode, DecoderBufferDelay shall not be equal to 0 and shall not
    exceed 90000 * ( BufferSize ÷ BitRate).


      NOTE: It is common to choose ( ( EncoderBufferDelay + DecoderBufferDelay ) ÷ 90000 ) * BitRate
      equal to a constant within a coded video sequence, and for this constant to be equal to BufferSize, but
      these are not strict requirements for bitstream conformance.

§   E.7.9.Decode Process constraints

    It is a requirement of bitstream conformance that the decoder model process can be invoked with the
    bitstream data for any signaled operating point or an extended layer without triggering a call to the
    bitstream_non_conformant function.

                                                                                       ↑ Back to Table of Contents




    AV2 Specification                                                                             Page 1138 of 1169
```
