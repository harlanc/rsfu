use webrtc::peer_connection::RTCPeerConnection;

struct Publisher {
    id: String,

    pc: RTCPeerConnection,
}

pub(super) struct PublisherTrack {}
