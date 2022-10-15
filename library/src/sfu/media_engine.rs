use super::errors::Result;
use sdp::extmap;
use webrtc::api::media_engine::MediaEngine;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecParameters;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpHeaderExtensionCapability;
use webrtc::rtp_transceiver::rtp_codec::RTPCodecType;
use webrtc::rtp_transceiver::RTCPFeedback;

const MIME_TYPE_H264: &'static str = "video/h264";
const MIME_TYPE_OPUS: &'static str = "audio/opus";
const MIME_TYPE_VP8: &'static str = "video/vp8";
const MIME_TYPE_VP9: &'static str = "video/vp9";

const FRAME_MARKING: &'static str = "urn:ietf:params:rtp-hdrext:framemarking";

pub(super) async fn get_publisher_media_engine() -> Result<MediaEngine> {
    let mut me = MediaEngine::default();
    me.register_codec(
        RTCRtpCodecParameters {
            capability: RTCRtpCodecCapability {
                mime_type: String::from(MIME_TYPE_OPUS),
                clock_rate: 48000,
                channels: 2,
                sdp_fmtp_line: String::from("minptime=10;useinbandfec=1"),
                rtcp_feedback: Vec::new(),
            },
            payload_type: 111,
            ..Default::default()
        },
        RTPCodecType::Audio,
    )?;

    let feedbacks = vec![
        RTCPFeedback {
            typ: String::from("goog-remb"),
            parameter: String::from(""),
        },
        RTCPFeedback {
            typ: String::from("ccm"),
            parameter: String::from("fir"),
        },
        RTCPFeedback {
            typ: String::from("nack"),
            parameter: String::from(""),
        },
        RTCPFeedback {
            typ: String::from("nack"),
            parameter: String::from("pli"),
        },
    ];

    let codc_parameters = vec![
        RTCRtpCodecParameters {
            capability: RTCRtpCodecCapability {
                mime_type: String::from(MIME_TYPE_VP8),
                clock_rate: 90000,
                rtcp_feedback: feedbacks.clone(),
                ..Default::default()
            },
            payload_type: 96,
            ..Default::default()
        },
        RTCRtpCodecParameters {
            capability: RTCRtpCodecCapability {
                mime_type: String::from(MIME_TYPE_VP9),
                clock_rate: 90000,
                sdp_fmtp_line: String::from("profile-id=0"),
                rtcp_feedback: feedbacks.clone(),
                ..Default::default()
            },
            payload_type: 98,
            ..Default::default()
        },
        RTCRtpCodecParameters {
            capability: RTCRtpCodecCapability {
                mime_type: String::from(MIME_TYPE_VP9),
                clock_rate: 90000,
                sdp_fmtp_line: String::from("profile-id=1"),
                rtcp_feedback: feedbacks.clone(),
                ..Default::default()
            },
            payload_type: 100,
            ..Default::default()
        },
        RTCRtpCodecParameters {
            capability: RTCRtpCodecCapability {
                mime_type: String::from(MIME_TYPE_H264),
                clock_rate: 90000,
                sdp_fmtp_line: String::from(
                    "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42001f",
                ),
                rtcp_feedback: feedbacks.clone(),
                ..Default::default()
            },
            payload_type: 102,
            ..Default::default()
        },
        RTCRtpCodecParameters {
            capability: RTCRtpCodecCapability {
                mime_type: String::from(MIME_TYPE_H264),
                clock_rate: 90000,
                sdp_fmtp_line: String::from(
                    "level-asymmetry-allowed=1;packetization-mode=0;profile-level-id=42001f",
                ),
                rtcp_feedback: feedbacks.clone(),
                ..Default::default()
            },
            payload_type: 127,
            ..Default::default()
        },
        RTCRtpCodecParameters {
            capability: RTCRtpCodecCapability {
                mime_type: String::from(MIME_TYPE_H264),
                clock_rate: 90000,
                sdp_fmtp_line: String::from(
                    "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f",
                ),
                rtcp_feedback: feedbacks.clone(),
                ..Default::default()
            },
            payload_type: 125,
            ..Default::default()
        },
        RTCRtpCodecParameters {
            capability: RTCRtpCodecCapability {
                mime_type: String::from(MIME_TYPE_H264),
                clock_rate: 90000,
                sdp_fmtp_line: String::from(
                    "level-asymmetry-allowed=1;packetization-mode=0;profile-level-id=42e01f",
                ),
                rtcp_feedback: feedbacks.clone(),
                ..Default::default()
            },
            payload_type: 108,
            ..Default::default()
        },
        RTCRtpCodecParameters {
            capability: RTCRtpCodecCapability {
                mime_type: String::from(MIME_TYPE_VP8),
                clock_rate: 90000,
                sdp_fmtp_line: String::from(
                    "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=640032",
                ),
                rtcp_feedback: feedbacks.clone(),
                ..Default::default()
            },
            payload_type: 123,
            ..Default::default()
        },
    ];

    for codec in codc_parameters {
        me.register_codec(codec, RTPCodecType::Video)?;
    }

    let extensions_video = vec![
        extmap::SDES_MID_URI,
        extmap::SDES_RTP_STREAM_ID_URI,
        extmap::TRANSPORT_CC_URI,
        FRAME_MARKING,
    ];

    for extention in extensions_video {
        me.register_header_extension(
            RTCRtpHeaderExtensionCapability {
                uri: String::from(extention),
            },
            RTPCodecType::Video,
            Vec::new(),
        )?;
    }

    let extensions_audio = vec![
        extmap::SDES_MID_URI,
        extmap::SDES_RTP_STREAM_ID_URI,
        extmap::AUDIO_LEVEL_URI,
    ];

    for extention in extensions_audio {
        me.register_header_extension(
            RTCRtpHeaderExtensionCapability {
                uri: String::from(extention),
            },
            RTPCodecType::Audio,
            Vec::new(),
        )?;
    }
    Ok(me)
}

pub(super) fn get_subscriber_media_engine() -> Result<MediaEngine> {
    Ok(MediaEngine::default())
}
