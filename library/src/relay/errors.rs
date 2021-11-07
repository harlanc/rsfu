use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum Error {
    #[error("relay Peer is not ready")]
    ErrRelayPeerNotReady,
    #[error("relay Peer signal already called")]
    ErrRelayPeerSignalDone,
    #[error("relay Peer data channel is not ready")]
    ErrRelaySignalDCNotReady,

    #[error("relay Peer send data failed")]
    ErrRelaySendDataFailed,

    #[error("relay request timeout")]
    ErrRelayRequestTimeout,

    #[error("relay request empty response")]
    ErrRelayRequestEmptyRespose,
}
impl Error {
    pub fn equal(&self, err: &anyhow::Error) -> bool {
        err.downcast_ref::<Self>().map_or(false, |e| e == self)
    }
}
