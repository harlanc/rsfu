use serde_json::value::Index;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Default, Clone)]
pub struct AudioStream {
    id: String,
    sum: i32,
    total: i32,
}
#[derive(Default, Clone)]
pub struct AudioObserver {
    streams: Arc<Mutex<Vec<AudioStream>>>,
    expected: i32,
    threshold: u8,
    previous: Vec<String>,
}

impl AudioObserver {
    fn new(threshold_parameter: u8, interval_parameter: i32, filter_parameter: i32) -> Self {
        let mut threshold: u8 = threshold_parameter;
        if threshold > 127 {
            threshold = 127;
        }
        let mut filter: i32 = filter_parameter;
        if filter < 0 {
            filter = 0;
        }
        if filter > 100 {
            filter = 100;
        }
        Self {
            threshold,
            expected: interval_parameter * filter / 2000,
            ..Default::default()
        }
    }

    pub async fn add_stream(&mut self, stream_id: String) {
        self.streams.lock().await.push(AudioStream {
            id: stream_id,
            ..Default::default()
        })
    }

    async fn remove_stream(&mut self, stream_id: String) {
        let mut streams = self.streams.lock().await;
        let mut idx = 0 as usize;
        while idx < streams.len() {
            if streams[idx].id == stream_id {
                streams.remove(idx);
                continue;
            }
            idx = idx + 1;
        }
    }

    async fn observe(&mut self, stream_id: String, d_bov: u8) {
        let mut streams = self.streams.lock().await;

        for stream in streams.iter_mut() {
            if stream.id == stream_id {
                if d_bov <= self.threshold {
                    stream.sum += d_bov as i32;
                    stream.total += 1;
                }
                return;
            }
        }
    }

    pub async fn calc(&mut self) -> Option<Vec<String>> {
        let mut streams = self.streams.lock().await;

        streams.sort_by(|a, b| {
            if b.total != a.total {
                return b.total.cmp(&a.total);
            }
            return b.sum.cmp(&a.sum);
        });

        let mut stream_ids = Vec::new();

        for stream in streams.iter_mut() {
            if stream.total >= self.expected {
                stream_ids.push(stream.id.clone());
            }

            stream.total = 0;
            stream.sum = 0;
        }

        if self.previous.len() == stream_ids.len() {
            for idx in 0..self.previous.len() {
                if self.previous[idx] != stream_ids[idx] {
                    self.previous = stream_ids.clone();

                    return Some(stream_ids);
                }
            }
            return None;
        }
        self.previous = stream_ids.clone();

        return Some(stream_ids);
    }
}
