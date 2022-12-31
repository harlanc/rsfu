use super::errors::Result;
use async_trait::async_trait;
#[async_trait]
pub trait BufferIO {
    fn read(&mut self, buff: &mut [u8]) -> Result<usize>;
    async fn write(&mut self, pkt: &[u8]) -> Result<u32>;
    fn close(&mut self) -> Result<()>;
}
