use super::buffer::Buffer;
use super::buffer::BufferPacketType;
use super::buffer_io::BufferIO;
use super::rtcpreader::RTCPReader;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Once;
use tokio::sync::Mutex;
#[derive(Default)]
pub struct Factory {
    video_pool_size: usize,
    audio_pool_size: usize,
    pub rtp_buffers: HashMap<u32, Arc<Mutex<Buffer>>>,
    pub rtcp_readers: HashMap<u32, Arc<Mutex<RTCPReader>>>,
}

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

    // async fn get_or_new<'a>(
    //     &'a mut self,
    //     packet_type: BufferPacketType,
    //     ssrc: u32,
    // ) -> Box<&'a dyn BufferIO> {
    //     let mut factory = self.factory.lock().await;

    //     match packet_type {
    //         BufferPacketType::RTPBufferPacket => {
    //             if let Some(reader) = factory.rtp_buffers.get(&ssrc) {
    //                 return Box::new(reader);
    //             }
    //             let reader = Buffer::new(ssrc);
    //             factory.rtp_buffers.insert(ssrc, reader);
    //             let ref_reader = factory.rtp_buffers.get(&ssrc).unwrap();
    //             return Box::new(ref_reader);
    //         }
    //         BufferPacketType::RTCPBufferPacket => {
    //             if let Some(readerr) = factory.rtcp_readers.get(&ssrc) {
    //                 return Box::new(readerr);
    //             }
    //             let reader = RTCPReader::new(ssrc);
    //             factory.rtcp_readers.insert(ssrc, reader);
    //             Box::new(factory.rtcp_readers.get(&ssrc).unwrap())
    //         }
    //     }
    // }

    pub async fn get_or_new_rtcp_buffer(&mut self, ssrc: u32) -> Arc<Mutex<RTCPReader>> {
        let factory = &mut self.factory.lock().await;

        if let Some(reader) = factory.rtcp_readers.get_mut(&ssrc) {
            return reader.clone();
        }
        let reader = Arc::new(Mutex::new(RTCPReader::new(ssrc)));

        factory.rtcp_readers.insert(ssrc, reader.clone());

        reader
    }

    pub async fn get_or_new_buffer(
        &mut self,
        // packet_type: BufferPacketType,
        ssrc: u32,
    ) -> Arc<Mutex<Buffer>> {
        let factory = &mut self.factory.lock().await;

        if let Some(reader) = factory.rtp_buffers.get_mut(&ssrc) {
            return reader.clone();
        }
        let reader = Arc::new(Mutex::new(Buffer::new(ssrc)));

        factory.rtp_buffers.insert(ssrc, reader.clone());

        reader
    }
}
