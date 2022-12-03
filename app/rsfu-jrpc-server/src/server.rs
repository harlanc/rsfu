use jrpc2::error::JsonError;
use rsfu::sfu::peer::JoinConfig;
use rsfu::sfu::peer::PeerLocal;
use rsfu::sfu::sfu::SFU;
use turn::server::request;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

use async_trait::async_trait;
use jrpc2::define::Error as Jrpc2Error;
use jrpc2::define::Request;
use jrpc2::define::Response;
use jrpc2::jsonrpc2::JsonRpc2;
use jrpc2::jsonrpc2::THandler;
use jrpc2::jsonrpc2::TJsonRpc2;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::sync::Arc;

#[derive(Clone, Serialize, Deserialize)]
pub struct Join {
    #[serde(rename = "sid")]
    sid: String,
    #[serde(rename = "uid")]
    uid: String,
    #[serde(rename = "offer")]
    offer: RTCSessionDescription,
    #[serde(skip)]
    config: JoinConfig,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Negotiation {
    #[serde(rename = "desc")]
    desc: RTCSessionDescription,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Trickle {
    #[serde(rename = "target")]
    target: u8,
    #[serde(rename = "candidate")]
    candidate: RTCIceCandidateInit,
}

struct JsonSignal {
    peer_local: PeerLocal,
}

impl JsonSignal {
    fn new(p: PeerLocal) -> Self {
        Self { peer_local: p }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Parameters {
    Join(Join),
    Offer(RTCSessionDescription),
    Trickle(Trickle),
    Negotiation(Negotiation),
}

type ResponseResult = RTCSessionDescription;
type ErrorData = String;
type RequestParams = Parameters;

impl JsonSignal {}

#[async_trait]
impl THandler<RequestParams, ResponseResult, ErrorData> for JsonSignal {
    async fn handle(
        &mut self,
        json_rpc2: Arc<JsonRpc2<RequestParams, ResponseResult, ErrorData>>,
        request: Request<RequestParams>,
    ) {
        let request_id = request.id;
        let response_error = |error_data: &str| {
            let err = Jrpc2Error::new(-1, error_data.to_string(), None);
            let response = Response::new(request_id.clone().unwrap(), None, Some(err));
            if let Err(err) = json_rpc2.response(response) {
                log::error!("response error: {}", err);
            }
        };
        match request.method.as_str() {
            "join" => {
                let mut join_param: Option<Join> = None;
                if let Some(Parameters::Join(join)) = request.params {
                    join_param = Some(join);
                }
                if join_param.is_none() {
                    response_error("join parameter is none");
                    return;
                }

                let rpc2_out_clone = json_rpc2.clone();
                self.peer_local
                    .on_offer(Box::new(move |offer: RTCSessionDescription| {
                        let rpc2_in_clone = rpc2_out_clone.clone();
                        Box::pin(async move {
                            if let Err(err) = rpc2_in_clone.notify(
                                String::from_str("offer").unwrap(),
                                Some(Parameters::Offer(offer)),
                            ) {
                                log::error!("notify err:{}", err);
                            }
                        })
                    }))
                    .await;

                let rpc2_out_clone_2 = json_rpc2.clone();
                self.peer_local
                    .on_ice_candidate(Box::new(
                        move |candidate: RTCIceCandidateInit, target: u8| {
                            let rpc2_in_clone = rpc2_out_clone_2.clone();
                            Box::pin(async move {
                                if let Err(err) = rpc2_in_clone.notify(
                                    String::from_str("trickle").unwrap(),
                                    Some(Parameters::Trickle(Trickle { target, candidate })),
                                ) {
                                    log::error!("notify err:{}", err);
                                }
                            })
                        },
                    ))
                    .await;

                let join = join_param.unwrap();

                if let Err(err) = self.peer_local.join(join.sid, join.uid, join.config).await {
                    response_error("join err");
                    log::error!("join err: {}", err);
                    return;
                }

                match self.peer_local.answer(join.offer).await {
                    Ok(answer) => {
                        if let Err(err) = json_rpc2.response(Response::new(
                            request_id.clone().unwrap(),
                            Some(answer),
                            None,
                        )) {
                            log::error!("response err: {}", err);
                        }
                    }
                    Err(err) => {
                        log::error!("answer error: {}", err);
                        return;
                    }
                }
            }
            "offer" => {
                if let Some(Parameters::Negotiation(negotiation)) = request.params {
                    match self.peer_local.answer(negotiation.desc).await {
                        Ok(answer) => {
                            if let Err(err) = json_rpc2.response(Response::new(
                                request_id.clone().unwrap(),
                                Some(answer),
                                None,
                            )) {
                                log::error!("response err: {}", err);
                            }
                        }
                        Err(err) => {
                            response_error("answer error");
                            return;
                        }
                    }
                } else {
                    response_error("negotiation is none");
                    log::error!("negotiation is none");
                    return;
                }
            }

            "answer" => {
                if let Some(Parameters::Negotiation(negotiation)) = request.params {
                    if let Err(err) = self
                        .peer_local
                        .set_remote_description(negotiation.desc)
                        .await
                    {
                        response_error("set_remote_description error");
                    }
                } else {
                    response_error("negotiation is none");
                    log::error!("negotiation is none");
                    return;
                }
            }

            "trickle" => {
                if let Some(Parameters::Trickle(trickle)) = request.params {
                    if let Err(err) = self
                        .peer_local
                        .trickle(trickle.candidate, trickle.target)
                        .await
                    {
                        response_error("trickle error");
                    }
                } else {
                    response_error("trickle is none");
                    log::error!("trickle is none");
                    return;
                }
            }

            _ => {
                log::info!("unknow method");
            }
        }
    }
}
