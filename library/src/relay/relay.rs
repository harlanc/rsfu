use webrtc::media::dtls_transport::dtls_parameters::DTLSParameters;
use webrtc::media::ice_transport::ice_parameters::RTCIceParameters;
use webrtc::media::rtp::RTCRtpCodingParameters;
use webrtc::peer::ice::ice_candidate::RTCIceCandidate;

const SIGNALER_LABEL: &'static str = "ion_sfu_relay_signaler";
const SIGNALER_REQUEST_EVENT: &'static str = "ion_relay_request";

pub struct signal {
    encodings: RTCRtpCodingParameters,
    ice_candidates: Vec<RTCIceCandidate>,
    ice_parameters: RTCIceParameters,
    dtls_parameters: DTLSParameters,
}
