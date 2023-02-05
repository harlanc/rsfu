use super::down_track::DownTrack;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use tokio::sync::Mutex as TokioMutex;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::data_channel::RTCDataChannel;

pub type MessageProcessorFunc = Box<
    dyn (FnMut(ProcessArgs) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>) + Send + Sync,
>;
pub type MessageProcessorChainFunc =
    fn(Arc<Mutex<dyn MessageProcessor + Send>>) -> Arc<Mutex<dyn MessageProcessor + Send>>;

#[derive(Default, Clone)]
pub struct ProcessArgs {
    pub down_tracks: Vec<Arc<TokioMutex<DownTrack>>>,
    pub message: DataChannelMessage,
    pub data_channel: Arc<RTCDataChannel>,
}

pub trait MessageProcessor {
    fn process(&mut self, args: ProcessArgs);
}

#[derive(Default)]
pub struct DataChannel {
    pub label: String,
    pub middlewares: Arc<Mutex<Vec<MessageProcessorChainFunc>>>,
    pub on_message: Option<fn(args: ProcessArgs)>,
}

pub struct Middlewares {
    middlewares: Arc<Mutex<Vec<MessageProcessorChainFunc>>>,
}
pub struct ProcessFunc {
    f: Arc<Mutex<Option<MessageProcessorFunc>>>,
}

pub struct ChainHandler {
    #[allow(dead_code)]
    middlewares: Arc<Middlewares>,
    #[allow(dead_code)]
    last: Arc<Mutex<dyn MessageProcessor + Send>>,
    current: Arc<Mutex<dyn MessageProcessor + Send>>,
}

impl DataChannel {
    pub fn new(label: String) -> Self {
        Self {
            label,
            ..Default::default()
        }
    }
    #[allow(dead_code)]
    fn use_middleware(&mut self, f: MessageProcessorChainFunc) {
        self.middlewares.lock().unwrap().push(f);
    }
    #[allow(dead_code)]
    fn on_message(&mut self, f: fn(args: ProcessArgs)) {
        self.on_message = Some(f);

        // let mut on_close = self.on_close_handler.lock().await;
        // *on_close = Some(f);
    }
}

// async fn on_publisher_track(&mut self, f: OnPublisherTrack) {
//     let mut handler = self.on_publisher_track.lock().await;
//     *handler = Some(f);
// }

impl ProcessFunc {
    pub fn new(f: MessageProcessorFunc) -> Self {
        Self {
            f: Arc::new(Mutex::new(Some(f))),
        }
    }
}

impl MessageProcessor for ProcessFunc {
    fn process(&mut self, args: ProcessArgs) {
        let mut handler = self.f.lock().unwrap();
        if let Some(f) = &mut *handler {
            f(args);
        }
    }
}

impl Middlewares {
    pub fn new(m: Arc<Mutex<Vec<MessageProcessorChainFunc>>>) -> Arc<Self> {
        Arc::new(Middlewares { middlewares: m })
    }

    pub fn process(
        self: &Arc<Self>,
        h: Arc<Mutex<dyn MessageProcessor + Send>>,
    ) -> Arc<Mutex<dyn MessageProcessor + Send>> {
        Arc::new(Mutex::new(ChainHandler {
            middlewares: self.clone(),
            last: h.clone(),
            current: chain(self.middlewares.clone(), h),
        }))
    }
}

impl MessageProcessor for ChainHandler {
    fn process(&mut self, args: ProcessArgs) {
        let mut c = self.current.lock().unwrap();
        c.process(args)
    }
}

pub fn chain(
    mws: Arc<Mutex<Vec<MessageProcessorChainFunc>>>,
    last: Arc<Mutex<dyn MessageProcessor + Send>>,
) -> Arc<Mutex<dyn MessageProcessor + Send>> {
    let mws_value = mws.lock().unwrap();
    if mws_value.len() == 0 {
        return last;
    }

    let mut h = mws_value[mws_value.len() - 1](last);
    for i in (0..mws_value.len() - 2).rev() {
        h = mws_value[i](h);
    }
    h
}
