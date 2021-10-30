use crate::buffer::errors::*;
use anyhow::Result;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

pub struct RTCPReader {
    ssrc: u32,
    closed: AtomicBool,

    on_packet: AtomicPtr<fn()>,

    on_close: fn(),
}
//https://stackoverflow.com/questions/37370120/right-way-to-have-function-pointers-in-struct
fn default() {}

impl RTCPReader {
    fn new(ssrc: u32) -> Self {
        Self {
            ssrc,
            closed: AtomicBool::new(false),
            on_packet: AtomicPtr::default(),
            on_close: default,
        }
    }

    fn write(&mut self, p: &[u8]) -> Result<u32> {
        if self.closed.load(Ordering::Relaxed) {
            return Err(Error::ErrIOEof.into());
        }

        // if let f = self.on_packet.load(Ordering::Relaxed) {
        //     f();
        // }

        Ok(9)
    }

    fn set_on_close(&mut self, f: fn()) {
        self.on_close = f;
    }
    fn close(&mut self) -> Result<()> {
        self.closed.store(true, Ordering::Relaxed);
        (self.on_close)();

        Ok(())
    }
}
