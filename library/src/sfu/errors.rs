use thiserror::Error;

//use webrtc::error::Error as WebRTCErrorError;

use webrtc::Error as WebRTCError;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Error, Debug, PartialEq)]
pub enum Error {
    // ErrTransportExists join is called after a peerconnection is established
    #[error("rtc transport already exists for this connection")]
    ErrTransportExists,
    // ErrNoTransportEstablished cannot signal before join
    #[error("no rtc transport exists for this Peer")]
    ErrNoTransportEstablished,
    // ErrOfferIgnored if offer received in unstable state
    #[error("offered ignored")]
    ErrOfferIgnored,
    // ErrTurnNoneAuthKey
    #[error("cannot get auth key from user map")]
    ErrTurnNoneAuthKey,
    #[error("webrtc error")]
    ErrWebRTC(WebRTCError),
    // #[error("webrtc error error")]
    // ErrWebRTCError(WebRTCErrorError),
}
impl Error {
    pub fn equal(&self, err: &anyhow::Error) -> bool {
        err.downcast_ref::<Self>().map_or(false, |e| e == self)
    }
}

impl From<WebRTCError> for Error {
    fn from(error: WebRTCError) -> Self {
        Error::ErrWebRTC(error)
    }
}

// impl From<WebRTCErrorError> for Error {
//     fn from(error: WebRTCErrorError) -> Self {
//         Error::ErrWebRTCError(error)
//     }
// }
