use std::collections::HashMap;

use sdp::SessionDescription;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, AtomicU32, Ordering};
use webrtc::api;
use webrtc::api::media_engine::MediaEngine;
use webrtc::data_channel::data_channel_init::RTCDataChannelInit;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::data_channel::RTCDataChannel;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::ice_transport::ice_connection_state::RTCIceConnectionState;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;

use webrtc::ice_transport::ice_gatherer::OnLocalCandidateHdlrFn;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::rtcp::source_description::SourceDescription;

use super::peer::Peer;
use std::future::Future;
use std::io::Write;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};

use std::sync::Mutex as SyncMutex;

use super::down_track::DownTrack;
use super::media_engine;
use super::sfu::WebRTCTransportConfig;
use std::rc::Rc;

use super::data_channel::Middlewares;
use super::data_channel::{DataChannel, ProcessArgs, ProcessFunc};
// use super::errors::SfuErrorValue;
// use super::errors::{Result, SfuError};

use anyhow::Result;

pub const API_CHANNEL_LABEL: &'static str = "rsfu";

pub type OnNegotiateFn =
    Box<dyn (FnMut() -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>) + Send + Sync>;

pub struct Subscriber {
    id: String,
    pc: Arc<RTCPeerConnection>,
    me: MediaEngine,

    tracks: HashMap<String, Vec<Arc<DownTrack>>>,
    channels: HashMap<String, Arc<RTCDataChannel>>,
    candidates: Vec<RTCIceCandidateInit>,
    on_negotiate_handler: Arc<Mutex<Option<OnNegotiateFn>>>,
    pub no_auto_subscribe: bool,
}

impl Subscriber {
    pub async fn new(id: String, cfg: Arc<WebRTCTransportConfig>) -> Result<Subscriber> {
        let me = media_engine::get_subscriber_media_engine()?;

        // let transport_cfg = cfg.lock().await;

        let api = api::APIBuilder::new()
            .with_media_engine(me)
            .with_setting_engine(cfg.setting.clone())
            .build();

        let pc = api
            .new_peer_connection(RTCConfiguration {
                ice_servers: cfg.configuration.ice_servers.clone(),
                sdp_semantics: cfg.configuration.sdp_semantics.clone(),
                ..Default::default()
            })
            .await?;

        let subscriber = Subscriber {
            id,
            pc: Arc::new(pc),
            me: media_engine::get_subscriber_media_engine()?,
            tracks: HashMap::new(),
            channels: HashMap::new(),
            candidates: Vec::new(),
            on_negotiate_handler: Arc::new(Mutex::new(None)),
            no_auto_subscribe: false,
        };

        subscriber.on_ice_connection_state_change().await;

        Ok(subscriber)
    }

    pub async fn add_data_channel(
        &mut self,
        peer: Arc<dyn Peer + Send + Sync>,
        dc: Arc<Mutex<DataChannel>>,
    ) -> Result<()> {
        let ndc = self
            .pc
            .create_data_channel(
                &dc.lock().await.label[..],
                Some(RTCDataChannelInit::default()),
            )
            .await?;

        let dc_out = dc.clone();
        let mws = Middlewares::new(dc.lock().await.middlewares.clone());

        let p = mws.process(Arc::new(SyncMutex::new(ProcessFunc::new(Box::new(
            move |args: ProcessArgs| {
                let dc_in = Arc::clone(&dc_out);
                Box::pin(async move {
                    let f = dc_in.lock().await;
                    if let Some(on_message) = f.on_message {
                        on_message(args);
                    }
                })
            },
        )))));

        let ndc_out = ndc.clone();
        let peer_out = peer.clone();

        ndc.on_message(Box::new(move |msg: DataChannelMessage| {
            let p_in = Arc::clone(&p);
            let ndc_in = ndc_out.clone();
            let peer_in = peer_out.clone();
            Box::pin(async move {
                let mut f = p_in.lock().unwrap();

                f.process(ProcessArgs {
                    peer: peer_in,
                    message: msg,
                    data_channel: ndc_in,
                })
            })
        }))
        .await;

        self.channels
            .insert(dc.lock().await.label.clone(), ndc.clone());

        Ok(())
    }

    pub fn data_channel(&self, label: String) -> Option<Arc<RTCDataChannel>> {
        if let Some(rtc_data_channel) = self.channels.get(&label) {
            return Some(rtc_data_channel.clone());
        }
        None
    }

    pub async fn on_negotiate(&mut self, f: OnNegotiateFn) {
        let mut handler = self.on_negotiate_handler.lock().await;
        *handler = Some(f);
    }

    pub async fn create_offer(&self) -> Result<RTCSessionDescription> {
        let offer = self.pc.create_offer(None).await?;
        self.pc.set_local_description(offer.clone()).await?;

        Ok(offer)
    }

    pub async fn on_ice_candidate(&self, f: OnLocalCandidateHdlrFn) {
        self.pc.on_ice_candidate(f).await
    }

    async fn add_ice_candidate(&mut self, candidate: RTCIceCandidateInit) -> Result<()> {
        if let Some(descripton) = self.pc.remote_description().await {
            self.pc.add_ice_candidate(candidate).await?;
            return Ok(());
        }

        self.candidates.push(candidate);
        Ok(())
    }

    fn add_down_track(&mut self, stream_id: String, down_track: Arc<DownTrack>) {
        if let Some(dt) = self.tracks.get_mut(&stream_id) {
            dt.push(down_track)
        } else {
            self.tracks.insert(stream_id, Vec::new());
        }
    }

    fn remove_down_track(&mut self, stream_id: String, down_track: DownTrack) {
        if let Some(dts) = self.tracks.get_mut(&stream_id) {
            let mut idx: i16 = -1;

            for (i, val) in dts.iter_mut().enumerate() {
                if val.id == down_track.id {
                    idx = i as i16;
                }
            }

            if idx >= 0 {
                dts.remove(idx as usize);
            }
        }
    }

    async fn add_data_channel_by_label(&mut self, label: String) -> Result<Arc<RTCDataChannel>> {
        if let Some(channel) = self.channels.get(&label) {
            return Ok(channel.clone());
        }

        let channel = self
            .pc
            .create_data_channel(&label, Some(RTCDataChannelInit::default()))
            .await?;
        self.channels.insert(label, channel.clone());
        Ok(channel)
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

    async fn close(&mut self) -> Result<()> {
        self.pc.close().await.map_err(anyhow::Error::msg)?;
        Ok(())
    }

    pub async fn set_remote_description(&mut self, desc: RTCSessionDescription) -> Result<()> {
        self.pc.set_remote_description(desc).await?;

        for candidate in &self.candidates {
            self.pc.add_ice_candidate(candidate.clone()).await?;
        }

        self.candidates.clear();

        Ok(())
    }

    pub fn register_data_channel(&mut self, label: String, dc: Arc<RTCDataChannel>) {
        self.channels.insert(label, dc);
    }

    pub fn get_data_channel(&self, label: String) -> Option<Arc<RTCDataChannel>> {
        self.data_channel(label)
    }

    fn downtracks(&mut self) -> Vec<Arc<DownTrack>> {
        let mut downtracks: Vec<Arc<DownTrack>> = Vec::new();
        for (_, v) in &mut self.tracks {
            downtracks.append(v);
        }

        downtracks
    }

    fn get_downtracks(&self, stream_id: String) -> Option<Vec<Arc<DownTrack>>> {
        if let Some(val) = self.tracks.get(&stream_id) {
            Some(val.clone())
        } else {
            None
        }
    }

    async fn negotiate(&self) {
        let mut handler = self.on_negotiate_handler.lock().await;
        if let Some(f) = &mut *handler {
            f().await;
        }
    }
}
