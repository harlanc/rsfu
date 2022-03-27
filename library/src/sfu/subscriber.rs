use std::collections::HashMap;

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, AtomicU32, Ordering};
use webrtc::api;
use webrtc::api::media_engine::MediaEngine;
use webrtc::data_channel::RTCDataChannel;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::ice_transport::ice_connection_state::RTCIceConnectionState;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::rtcp::source_description::SourceDescription;

use std::future::Future;
use std::io::Write;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};

use super::down_track::DownTrack;
use super::media_engine;
use super::sfu::WebRTCTransportConfig;

use anyhow::Result;

pub const API_CHANNEL_LABEL: &'static str = "rsfu";

pub type NegotiateFn =
    Box<dyn (FnMut() -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>) + Send + Sync>;

pub struct Subscriber {
    id: String,
    pc: Arc<RTCPeerConnection>,
    me: MediaEngine,

    tracks: HashMap<String, Vec<DownTrack>>,
    channels: HashMap<String, Vec<RTCDataChannel>>,
    candidates: Vec<RTCIceCandidateInit>,
    negotiate: Arc<Mutex<Option<NegotiateFn>>>,
    no_auto_subscribe: bool,
}

impl Subscriber {
    async fn new(id: String, cfg: WebRTCTransportConfig) -> Result<Subscriber> {
        let me = media_engine::get_subscriber_media_engine()?;

        let api = api::APIBuilder::new()
            .with_media_engine(me)
            .with_setting_engine(cfg.setting)
            .build();

        let pc = api.new_peer_connection(cfg.configuration).await?;

        let subscriber = Subscriber {
            id,
            pc: Arc::new(pc),
            me: media_engine::get_subscriber_media_engine()?,
            tracks: HashMap::new(),
            channels: HashMap::new(),
            candidates: Vec::new(),
            negotiate: Arc::new(Mutex::new(None)),
            no_auto_subscribe: false,
        };

        subscriber.on_ice_connection_state_change().await;

        Ok(subscriber)
    }

    async fn on_ice_connection_state_change(&self) {
        let pc_out = Arc::clone(&self.pc);
        self.pc
            .on_ice_connection_state_change(Box::new(move |ice_state: RTCIceConnectionState| {
                let pc_in = Arc::clone(&pc_out);

                Box::pin(async move {
                    match ice_state {
                        RTCIceConnectionState::Failed | RTCIceConnectionState::Closed => {
                            pc_in.close().await;
                        }
                        _ => {}
                    }
                })
            }))
            .await;
    }

    async fn down_track_reports(&self) {
        loop {
            sleep(Duration::from_secs(5)).await;

            if self.pc.connection_state() == RTCPeerConnectionState::Closed {
                return;
            }

            let mut rtcp_packets: Vec<Box<(dyn rtcp::packet::Packet + Send + Sync + 'static)>> =
                vec![];

            let mut sds = Vec::new();

            for dts in &self.tracks {
                for dt in dts.1 {
                    if dt.bound.load(Ordering::Relaxed) {
                        continue;
                    }

                    if let Some(sr) = dt.create_sender_report().await {
                        rtcp_packets.push(Box::new(sr));
                    }

                    if let Some(sd) = dt.create_source_description_chunks().await {
                        sds.append(&mut sd.clone());
                    }
                }
            }

            let mut i = 0;
            let mut j = 0;

            while i < sds.len() {
                i = (j + 1) * 15;

                if i > sds.len() {
                    i = sds.len();
                }

                let nsd = &sds[j * 15..i];

                rtcp_packets.push(Box::new(SourceDescription {
                    chunks: nsd.to_vec(),
                }));

                j += 1;

                if let Err(err) = self.pc.write_rtcp(&rtcp_packets[..]).await {}

                rtcp_packets.clear();
            }
        }
    }

    async fn send_stream_down_track_reports(&self, stream_id: String) {
        let mut sds = Vec::new();
        let mut rtcp_packets: Vec<Box<(dyn rtcp::packet::Packet + Send + Sync + 'static)>> = vec![];

        if let Some(dts) = self.tracks.get(&stream_id) {
            for dt in dts {
                if !dt.bound.load(Ordering::Relaxed) {
                    continue;
                }
                if let Some(dcs) = dt.create_source_description_chunks().await {
                    sds.append(&mut dcs.clone());
                }
            }
        }

        if sds.len() == 0 {
            return;
        }

        rtcp_packets.push(Box::new(SourceDescription { chunks: sds }));

        let pc_out = self.pc.clone();

        tokio::spawn(async move {
            let mut i = 0;
            loop {
                if let Err(err) = pc_out.write_rtcp(&rtcp_packets[..]).await {}

                if i > 5 {
                    return;
                }
                i += 1;

                sleep(Duration::from_millis(20)).await;
            }
        });
    }

    async fn close(&mut self) -> Result<(), webrtc::Error> {
        self.pc.close().await
    }

    //async fn add_data_channel(peer: Pee)
}
