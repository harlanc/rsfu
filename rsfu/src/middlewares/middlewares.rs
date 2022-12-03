use crate::sfu::data_channel::MessageProcessor;
use crate::sfu::data_channel::ProcessArgs;
use crate::sfu::data_channel::ProcessFunc;
use crate::sfu::helpers::set_vp8_temporal_layer;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::Mutex;
use webrtc::rtp_transceiver::rtp_codec::RTPCodecType;
use webrtc::track::track_local::TrackLocal;

const HIGH_VALUE: &'static str = "high";
const MEDIA_VALUE: &'static str = "medium";
const LOW_VALUE: &'static str = "low";
const MUTED_VALUE: &'static str = "none";
const ACTIVE_LAYER_METHOD: &'static str = "activeLayer";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetRemoteMedia {
    #[serde(rename = "streamId")]
    pub stream_id: String,
    #[serde(rename = "video")]
    pub video: String,
    #[serde(rename = "frameRate")]
    pub frame_rate: String,
    #[serde(rename = "audio")]
    pub audio: bool,
    #[serde(rename = "layers", skip_serializing_if = "Option::is_none")]
    pub layers: Option<Vec<String>>,
}

fn subscriber_api(
    next: Arc<Mutex<dyn MessageProcessor + Send>>,
) -> Arc<Mutex<dyn MessageProcessor + Send>> {
    let f = ProcessFunc::new(Box::new(move |args: ProcessArgs| {
        let next_in = next.clone();
        let args_clone = args.clone();

        Box::pin(async move {
            // let data = String::from_utf8(args.message.data.to_vec()).unwrap();
            // let set_remote_media = serde_json::from_str::<SetRemoteMedia>(&data).unwrap();

            // if !set_remote_media.layers.is_none() && set_remote_media.layers.unwrap().len() > 0 {
            // } else {
            //     for dt in args.down_tracks {
            //         let mut dt_val = dt.lock().await;
            //         match dt_val.kind() {
            //             RTPCodecType::Audio => dt_val.mute(!set_remote_media.audio),
            //             RTPCodecType::Video => {
            //                 match set_remote_media.video.as_str() {
            //                     HIGH_VALUE => {
            //                         dt_val.mute(false);
            //                         dt_val.switch_spatial_layer(2, true).await;
            //                     }
            //                     MEDIA_VALUE => {
            //                         dt_val.mute(false);
            //                         dt_val.switch_spatial_layer(1, true).await;
            //                     }
            //                     LOW_VALUE => {
            //                         dt_val.mute(false);
            //                         dt_val.switch_spatial_layer(0, true).await;
            //                     }
            //                     MUTED_VALUE => {
            //                         dt_val.mute(true);
            //                     }
            //                     _ => {}
            //                 }

            //                 match set_remote_media.frame_rate.as_str() {
            //                     HIGH_VALUE => {
            //                         dt_val.switch_temporal_layer(2, true);
            //                     }
            //                     MEDIA_VALUE => {
            //                         dt_val.switch_temporal_layer(1, true);
            //                     }
            //                     LOW_VALUE => {
            //                         dt_val.switch_temporal_layer(0, true);
            //                     }
            //                     _ => {}
            //                 }
            //             }

            //             RTPCodecType::Unspecified => {}
            //         }
            //     }
            // }
            // let mut n = next_in.lock().unwrap();
            // n.process(args_clone);
        })
    }));

    Arc::new(Mutex::new(f))
}
