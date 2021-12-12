use anyhow::Result;
pub trait BufferIO {
    fn read(&mut self, buff: &mut [u8]) -> Result<usize>;
    fn write(&mut self, pkt: &[u8]) -> Result<u32>;
    fn close(&mut self) -> Result<()>;
}
