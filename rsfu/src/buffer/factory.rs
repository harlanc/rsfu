use super::buffer::AtomicBuffer;
use super::rtcpreader::RTCPReader;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
#[derive(Default)]
pub struct Factory {
    #[allow(dead_code)]
    video_pool_size: usize,
    #[allow(dead_code)]
    audio_pool_size: usize,
    pub rtp_buffers: HashMap<u32, Arc<AtomicBuffer>>,
    pub rtcp_readers: HashMap<u32, Arc<Mutex<RTCPReader>>>,
}
#[derive(Default)]
pub struct AtomicFactory {
    factory: Arc<Mutex<Factory>>,
}

impl AtomicFactory {
    pub fn new(video_pool_size: usize, audio_pool_size: usize) -> Self {
        Self {
            factory: Arc::new(Mutex::new(Factory {
                video_pool_size,
                audio_pool_size,
                ..Default::default()
            })),
        }
    }

    pub async fn get_or_new_rtcp_buffer(&self, ssrc: u32) -> Arc<Mutex<RTCPReader>> {
        let factory = &mut self.factory.lock().await;

        if let Some(reader) = factory.rtcp_readers.get_mut(&ssrc) {
            return reader.clone();
        }

        let reader = Arc::new(Mutex::new(RTCPReader::new(ssrc)));
        factory.rtcp_readers.insert(ssrc, reader.clone());

        reader
    }

    pub async fn get_or_new_buffer(&self, ssrc: u32) -> Arc<AtomicBuffer> {
        let factory = &mut self.factory.lock().await;

        if let Some(reader) = factory.rtp_buffers.get_mut(&ssrc) {
            return reader.clone();
        }

        let reader = Arc::new(AtomicBuffer::new(ssrc));
        factory.rtp_buffers.insert(ssrc, reader.clone());

        reader
    }
}
