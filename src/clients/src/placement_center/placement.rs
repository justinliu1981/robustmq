/*
 * Copyright (c) 2023 RobustMQ Team
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
use crate::{retry_sleep_time, retry_times, ClientPool};
use common_base::{errors::RobustMQError, log::error_meta};
use protocol::placement_center::generate::{
    common::CommonReply,
    placement::{
        HeartbeatRequest, RegisterNodeRequest, SendRaftConfChangeReply, SendRaftConfChangeRequest,
        SendRaftMessageReply, SendRaftMessageRequest, UnRegisterNodeRequest,
    },
};
use std::{sync::Arc, time::Duration};
use tokio::{sync::Mutex, time::sleep};

use super::manager::placement_client;

pub async fn register_node(
    client_poll: Arc<Mutex<ClientPool>>,
    addr: String,
    request: RegisterNodeRequest,
) -> Result<CommonReply, RobustMQError> {
    match placement_client(client_poll, addr.clone()).await {
        Ok(mut client) => {
            let mut times = 0;
            loop {
                match client
                    .register_node(tonic::Request::new(request.clone()))
                    .await
                {
                    Ok(reply) => return Ok(reply.into_inner()),
                    Err(status) => {
                        error_meta(&format!(
                            "{},target ip:{},call function:{}",
                            status.to_string(),
                            addr,
                            "register_node"
                        ));
                        if times > retry_times() {
                            return Err(RobustMQError::MetaGrpcStatus(status));
                        }
                        times = times + 1;
                        sleep(Duration::from_secs(retry_sleep_time(times))).await;
                    }
                };
            }
        }
        Err(e) => {
            return Err(e);
        }
    }
}

pub async fn unregister_node(
    client_poll: Arc<Mutex<ClientPool>>,
    addr: String,
    request: UnRegisterNodeRequest,
) -> Result<CommonReply, RobustMQError> {
    match placement_client(client_poll, addr.clone()).await {
        Ok(mut client) => {
            let mut times = 0;
            loop {
                match client
                    .un_register_node(tonic::Request::new(request.clone()))
                    .await
                {
                    Ok(reply) => return Ok(reply.into_inner()),
                    Err(status) => {
                        error_meta(&format!(
                            "{},target ip:{},call function:{}",
                            status.to_string(),
                            addr,
                            "unregister_node"
                        ));
                        if times > retry_times() {
                            return Err(RobustMQError::MetaGrpcStatus(status));
                        }
                        times = times + 1;
                        sleep(Duration::from_secs(retry_sleep_time(times))).await;
                    }
                };
            }
        }
        Err(e) => {
            return Err(e);
        }
    }
}

pub async fn heartbeat(
    client_poll: Arc<Mutex<ClientPool>>,
    addr: String,
    request: HeartbeatRequest,
) -> Result<CommonReply, RobustMQError> {
    match placement_client(client_poll, addr.clone()).await {
        Ok(mut client) => {
            let mut times = 0;
            loop {
                match client.heartbeat(tonic::Request::new(request.clone())).await {
                    Ok(reply) => return Ok(reply.into_inner()),
                    Err(status) => {
                        error_meta(&format!(
                            "{},target ip:{},call function:{}",
                            status.to_string(),
                            addr,
                            "heartbeat"
                        ));
                        if times > retry_times() {
                            return Err(RobustMQError::MetaGrpcStatus(status));
                        }
                        times = times + 1;
                        sleep(Duration::from_secs(retry_sleep_time(times))).await;
                    }
                };
            }
        }
        Err(e) => {
            return Err(e);
        }
    }
}

pub async fn send_raft_message(
    client_poll: Arc<Mutex<ClientPool>>,
    addr: String,
    message: Vec<u8>,
) -> Result<SendRaftMessageReply, RobustMQError> {
    match placement_client(client_poll, addr.clone()).await {
        Ok(mut client) => {
            let request = SendRaftMessageRequest { message };
            let mut times = 0;
            loop {
                match client
                    .send_raft_message(tonic::Request::new(request.clone()))
                    .await
                {
                    Ok(reply) => return Ok(reply.into_inner()),
                    Err(status) => {
                        error_meta(&format!(
                            "{},target ip:{},call function:{}",
                            status.to_string(),
                            addr,
                            "send_raft_message"
                        ));
                        if times > retry_times() {
                            return Err(RobustMQError::MetaGrpcStatus(status));
                        }
                        times = times + 1;
                        sleep(Duration::from_secs(retry_sleep_time(times))).await;
                    }
                };
            }
        }
        Err(e) => {
            return Err(e);
        }
    }
}

pub async fn send_raft_conf_change(
    client_poll: Arc<Mutex<ClientPool>>,
    addr: String,
    message: Vec<u8>,
) -> Result<SendRaftConfChangeReply, RobustMQError> {
    match placement_client(client_poll, addr.clone()).await {
        Ok(mut client) => {
            let request = SendRaftConfChangeRequest { message };
            let mut times = 0;
            loop {
                match client
                    .send_raft_conf_change(tonic::Request::new(request.clone()))
                    .await
                {
                    Ok(reply) => return Ok(reply.into_inner()),
                    Err(status) => {
                        error_meta(&format!(
                            "{},target ip:{},call function:{}",
                            status.to_string(),
                            addr,
                            "send_raft_conf_change"
                        ));
                        if times > retry_times() {
                            return Err(RobustMQError::MetaGrpcStatus(status));
                        }
                        times = times + 1;
                        sleep(Duration::from_secs(retry_sleep_time(times))).await;
                    }
                };
            }
        }
        Err(e) => {
            return Err(e);
        }
    }
}
