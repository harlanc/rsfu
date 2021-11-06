use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering::SeqCst};
use std::sync::Arc;
use tokio::sync::Mutex;

pub type OnReadyFn =
    Box<dyn (FnMut() -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>) + Send + Sync>;
pub struct Pen {
    pub on_ready_handler: Arc<Mutex<Option<OnReadyFn>>>,
}

impl Pen {
    fn new() -> Self {
        Self {
            on_ready_handler: Arc::new(Mutex::new(None)),
        }
    }
    pub async fn on_ready(&self, f: OnReadyFn) {
        let mut handler = self.on_ready_handler.lock().await;
        *handler = Some(f);
    }
}

pub struct Person {
    pen: Pen,
    state: Arc<AtomicU32>,
}

impl Person {
    fn new() -> Self {
        Self {
            pen: Pen::new(),
            state: Arc::new(0.into()),
        }
    }

    async fn draw(&mut self) {}

    // async fn init(&mut self) {
    //     let state = Arc::clone(&self.state);
    //     self.pen
    //         .on_ready(Box::new(move || {
    //             state.store(2, SeqCst);
    //             Box::pin(async {
    //                 self.draw().await;
    //             })
    //         }))
    //         .await;
    // }
}
