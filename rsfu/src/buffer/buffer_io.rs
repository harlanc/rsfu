use super::errors::Result;
use async_trait::async_trait;
#[async_trait]
pub trait BufferIO {
    async fn read(&mut self, buff: &mut [u8]) -> Result<usize>;
    async fn write(& self, pkt: &[u8]) -> Result<u32>;
    async fn close(& self) -> Result<()>;
}
