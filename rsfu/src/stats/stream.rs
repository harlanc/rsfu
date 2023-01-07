use anyhow::Result;
use prometheus::{core::Collector, Histogram, HistogramOpts, HistogramVec};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

use prometheus::{Counter, CounterVec, Encoder, Gauge, GaugeVec, Opts, Registry, TextEncoder};

use crate::buffer::buffer::{AtomicBuffer, Stats};

struct PrometheusHandler {
    drift: Histogram,
    expected_count: Counter,
    received_count: Counter,
    packet_count: Counter,
    total_bytes: Counter,
    sessions: Gauge,
    peers: Gauge,
    audio_tracks: Gauge,
    video_tracks: Gauge,
}

impl PrometheusHandler {
    fn new() -> Self {
        let arift_buckets = vec![5.0, 10.0, 20.0, 40.0, 80.0, 160.0, f64::MAX];
        let drift = Histogram::with_opts(
            HistogramOpts::new("drift_millis", "drift_millis")
                .subsystem("rtp")
                .buckets(arift_buckets),
        )
        .unwrap();

        let expected_count =
            Counter::with_opts(Opts::new("expected", "expected").subsystem("rtp")).unwrap();
        let received_count =
            Counter::with_opts(Opts::new("received", "received").subsystem("rtp")).unwrap();
        let packet_count =
            Counter::with_opts(Opts::new("packets", "packets").subsystem("rtp")).unwrap();
        let total_bytes = Counter::with_opts(Opts::new("bytes", "bytes").subsystem("rtp")).unwrap();

        let sessions =
            Gauge::with_opts(Opts::new("sessions", "Current number of sessions").subsystem("sfu"))
                .unwrap();
        let peers = Gauge::with_opts(
            Opts::new("peers", "Current number of peers connected").subsystem("sfu"),
        )
        .unwrap();

        let audio_tracks = Gauge::with_opts(
            Opts::new("audio_tracks", "Current number of audio tracks").subsystem("sfu"),
        )
        .unwrap();
        let video_tracks = Gauge::with_opts(
            Opts::new("video_tracks", "Current number of video tracks").subsystem("sfu"),
        )
        .unwrap();

        Self {
            drift,
            expected_count,
            received_count,
            packet_count,
            total_bytes,
            sessions,
            peers,
            audio_tracks,
            video_tracks,
        }
    }

    fn register(&self) -> Result<()> {
        let r = Registry::new();

        r.register(Box::new(self.drift.clone()))?;
        r.register(Box::new(self.expected_count.clone()))?;
        r.register(Box::new(self.received_count.clone()))?;
        r.register(Box::new(self.packet_count.clone()))?;
        r.register(Box::new(self.total_bytes.clone()))?;
        r.register(Box::new(self.sessions.clone()))?;
        r.register(Box::new(self.peers.clone()))?;
        r.register(Box::new(self.audio_tracks.clone()))?;
        r.register(Box::new(self.video_tracks.clone()))?;

        Ok(())
    }
}

#[derive(Default)]
pub struct StreamStats {
    has_stas: bool,
    last_stats: Stats,
    diff_stats: Stats,
}

pub struct Stream {
    buffer: Arc<AtomicBuffer>,

    cname: Arc<Mutex<String>>,
    drift_in_millis: AtomicU64,
    stats: Arc<Mutex<StreamStats>>,

    prometheus_handler: PrometheusHandler,
}

impl Stream {
    pub fn new(buffer: Arc<AtomicBuffer>) -> Self {
        let prometheus_handler = PrometheusHandler::new();

        Self {
            buffer: buffer,
            cname: Arc::new(Mutex::new(String::default())),
            drift_in_millis: AtomicU64::default(),
            stats: Arc::new(Mutex::new(StreamStats::default())),
            prometheus_handler,
        }
    }

    async fn get_cname(&mut self) -> String {
        let cname = self.cname.lock().await;
        cname.clone()
    }

    pub async fn set_cname(&mut self, val: String) {
        let mut cname = self.cname.lock().await;
        *cname = val;
    }

    fn set_drift_in_millis(&mut self, val: u64) {
        self.drift_in_millis.store(val, Ordering::Relaxed);
    }

    fn get_drift_in_millis(&self) -> u64 {
        self.drift_in_millis.load(Ordering::Relaxed)
    }

    async fn update_stats(&mut self, stats: Stats) -> (bool, Stats) {
        let mut cur_stats = self.stats.lock().await;

        let mut has_status = false;

        if cur_stats.has_stas {
            cur_stats.diff_stats.last_expected =
                stats.last_expected - cur_stats.last_stats.last_expected;

            cur_stats.diff_stats.last_expected =
                stats.last_received - cur_stats.last_stats.last_received;

            cur_stats.diff_stats.packet_count =
                stats.packet_count - cur_stats.last_stats.packet_count;

            cur_stats.diff_stats.total_byte = stats.total_byte - cur_stats.last_stats.total_byte;

            has_status = true
        }

        cur_stats.last_stats = stats;
        cur_stats.has_stas = true;

        (has_status, cur_stats.diff_stats.clone())
    }

    async fn calc_stats(&mut self) {
        let buffer_stats = self.buffer.get_status().await;
        let drift_in_millis = self.get_drift_in_millis();

        let (has_stats, diff_stats) = self.update_stats(buffer_stats).await;

        self.prometheus_handler
            .drift
            .observe(drift_in_millis as f64);

        if has_stats {
            //self.prometheus_handler.expected_count.
        }

        // self.prometheus_handler.expected_count
    }
}
