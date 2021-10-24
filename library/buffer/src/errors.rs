use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum Error {
    #[error("packet not found in cache")]
    ErrPacketNotFound,
    #[error("buffer too small")]
    ErrBufferTooSmall,
    #[error("received packet too old")]
    ErrPacketTooOld,
    #[error("packet already received")]
    ErrRTXPacket,

    #[error("packet is not large enough")]
    ErrShortPacket,
    #[error("invalid nil packet")]
    ErrNilPacket,
}
impl Error {
    pub fn equal(&self, err: &anyhow::Error) -> bool {
        err.downcast_ref::<Self>().map_or(false, |e| e == self)
    }
}
