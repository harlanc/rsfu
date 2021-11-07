use thiserror::Error;

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
}
impl Error {
    pub fn equal(&self, err: &anyhow::Error) -> bool {
        err.downcast_ref::<Self>().map_or(false, |e| e == self)
    }
}
