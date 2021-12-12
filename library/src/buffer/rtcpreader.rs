use super::buffer_io::BufferIO;
use crate::buffer::errors::*;
use anyhow::Result;
use atomic::Atomic;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

pub type OnPacketFn =
    Box<dyn (FnMut(&[u8]) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>) + Send + Sync>;
pub type OnCloseFn =
    Box<dyn (FnMut() -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>) + Send + Sync>;

pub struct RTCPReader {
    ssrc: u32,
    closed: AtomicBool,
    on_packet: Arc<Mutex<Option<OnPacketFn>>>,
    on_close: Arc<Mutex<Option<OnCloseFn>>>,
}
//https://stackoverflow.com/questions/37370120/right-way-to-have-function-pointers-in-struct
fn default() {}

impl RTCPReader {
    pub fn new(ssrc: u32) -> Self {
        Self {
            ssrc,
            closed: AtomicBool::new(false),
            on_packet: Arc::default(),
            on_close: Arc::default(),
        }
    }

    async fn set_on_close(&mut self, f: OnCloseFn) {
        let mut on_close = self.on_close.lock().await;
        *on_close = Some(f);
    }

    async fn set_on_packet(&mut self, f: OnPacketFn) {
        let mut on_packet = self.on_packet.lock().await;
        *on_packet = Some(f);
    }

    fn read(&mut self, buff: &mut [u8]) -> Result<usize> {
        Ok(0)
    }
    async fn write(&mut self, p: &[u8]) -> Result<u32> {
        if self.closed.load(Ordering::Relaxed) {
            return Err(Error::ErrIOEof.into());
        }

        // let mut handler = on_track_handler.lock().await;
        // if let Some(f) = &mut *handler {
        //     f(t, r).await;
        // } else {
        //     log::warn!("on_track unset, unable to handle incoming media streams");
        // }

        let mut handler = self.on_packet.lock().await;
        if let Some(f) = &mut *handler {
            f(p);
        }

        // let f = self.on_packet.load(Ordering::Relaxed);
        // f();

        Ok(9)
    }
    async fn close(&mut self) -> Result<()> {
        self.closed.store(true, Ordering::Relaxed);

        let mut handler = self.on_close.lock().await;
        if let Some(f) = &mut *handler {
            f();
        }

        Ok(())
    }
}

//impl BufferIO for RTCPReader {}
