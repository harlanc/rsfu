use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum Error {
    #[error("ext info empty")]
    ErrExtInfoEmpty,
}
impl Error {
    pub fn equal(&self, err: &anyhow::Error) -> bool {
        err.downcast_ref::<Self>().map_or(false, |e| e == self)
    }
}
