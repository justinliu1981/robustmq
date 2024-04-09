// Copyright 2023 RobustMQ Team
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use clients::ClientPool;
use cluster::{register_broker_node, report_heartbeat, unregister_broker_node};
use common_base::{
    config::broker_mqtt::{broker_mqtt_conf, BrokerMQTTConfig},
    log::info_meta,
    runtime::create_runtime,
};
use flume::{Receiver, Sender};
use heartbeat::{
    heartbeat_manager::HeartbeatManager, keep_alive::KeepAlive, HEART_CONNECT_SHARD_HASH_NUM,
};
use metadata::cache::MetadataCache;
use server::{
    grpc::server::GrpcServer,
    http::server::{start_http_server, HttpServerState},
    start_mqtt_server,
    tcp::packet::{RequestPackage, ResponsePackage},
};
use std::sync::Arc;
use storage_adapter::adapter::placement::PlacementStorageAdapter;
use subscribe::subscribe_manager::SubScribeManager;
use tokio::{
    runtime::Runtime,
    signal,
    sync::{broadcast, Mutex, RwLock},
};

mod cluster;
mod heartbeat;
mod metadata;
mod metrics;
mod packet;
mod security;
mod server;
mod storage;
mod subscribe;

pub struct MqttBroker<'a> {
    conf: &'a BrokerMQTTConfig,
    metadata_cache: Arc<RwLock<MetadataCache>>,
    heartbeat_manager: Arc<RwLock<HeartbeatManager>>,
    subscribe_manager: Arc<RwLock<SubScribeManager>>,
    runtime: Runtime,
    request_queue_sx4: Sender<RequestPackage>,
    request_queue_rx4: Receiver<RequestPackage>,
    request_queue_sx5: Sender<RequestPackage>,
    request_queue_rx5: Receiver<RequestPackage>,
    response_queue_sx4: Sender<ResponsePackage>,
    response_queue_rx4: Receiver<ResponsePackage>,
    response_queue_sx5: Sender<ResponsePackage>,
    response_queue_rx5: Receiver<ResponsePackage>,
    client_poll: Arc<Mutex<ClientPool>>,
    storage_adapter: Arc<PlacementStorageAdapter>,
}

impl<'a> MqttBroker<'a> {
    pub fn new() -> Self {
        let conf = broker_mqtt_conf();
        let runtime = create_runtime("storage-engine-server-runtime", conf.runtime.worker_threads);

        let (request_queue_sx4, request_queue_rx4) = flume::bounded::<RequestPackage>(1000);
        let (request_queue_sx5, request_queue_rx5) = flume::bounded::<RequestPackage>(1000);
        let (response_queue_sx4, response_queue_rx4) = flume::bounded::<ResponsePackage>(1000);
        let (response_queue_sx5, response_queue_rx5) = flume::bounded::<ResponsePackage>(1000);

        let subscribe_manager = Arc::new(RwLock::new(SubScribeManager::new()));
        let heartbeat_manager = Arc::new(RwLock::new(HeartbeatManager::new(
            HEART_CONNECT_SHARD_HASH_NUM,
        )));

        let client_poll: Arc<Mutex<ClientPool>> = Arc::new(Mutex::new(ClientPool::new(1)));
        let storage_adapter = Arc::new(PlacementStorageAdapter::new(
            client_poll.clone(),
            conf.placement_center.clone(),
        ));
        let metadata_cache = Arc::new(RwLock::new(MetadataCache::new(storage_adapter.clone())));

        return MqttBroker {
            conf,
            runtime,
            metadata_cache,
            heartbeat_manager,
            subscribe_manager,
            request_queue_sx4,
            request_queue_rx4,
            request_queue_sx5,
            request_queue_rx5,
            response_queue_sx4,
            response_queue_rx4,
            response_queue_sx5,
            response_queue_rx5,
            client_poll,
            storage_adapter,
        };
    }

    pub fn start(&self, stop_send: broadcast::Sender<bool>) {
        self.register_node();
        self.start_grpc_server();
        self.start_mqtt_server();
        self.start_prometheus_export();
        self.start_keep_alive_thread();
        self.start_cluster_heartbeat_report();
        self.start_cluster_heartbeat_report();
        self.awaiting_stop(stop_send);
    }

    fn start_mqtt_server(&self) {
        let cache = self.metadata_cache.clone();
        let heartbeat_manager = self.heartbeat_manager.clone();
        let subscribe_manager = self.subscribe_manager.clone();
        let storage_adapter = self.storage_adapter.clone();

        let request_queue_sx4 = self.request_queue_sx4.clone();
        let request_queue_rx4 = self.request_queue_rx4.clone();
        let request_queue_sx5 = self.request_queue_sx5.clone();
        let request_queue_rx5 = self.request_queue_rx5.clone();

        let response_queue_sx4 = self.response_queue_sx4.clone();
        let response_queue_rx4 = self.response_queue_rx4.clone();
        let response_queue_sx5 = self.response_queue_sx5.clone();
        let response_queue_rx5 = self.response_queue_rx5.clone();
        self.runtime.spawn(async move {
            start_mqtt_server(
                cache,
                heartbeat_manager,
                subscribe_manager,
                storage_adapter,
                request_queue_sx4,
                request_queue_rx4,
                request_queue_sx5,
                request_queue_rx5,
                response_queue_sx4,
                response_queue_rx4,
                response_queue_sx5,
                response_queue_rx5,
            )
            .await
        });
    }

    fn start_grpc_server(&self) {
        let server = GrpcServer::new(
            self.conf.grpc_port.clone(),
            self.metadata_cache.clone(),
            self.storage_adapter.clone(),
        );
        self.runtime.spawn(async move {
            server.start().await;
        });
    }

    fn start_prometheus_export(&self) {
        let http_state = HttpServerState::new(
            self.metadata_cache.clone(),
            self.heartbeat_manager.clone(),
            self.subscribe_manager.clone(),
            self.response_queue_sx4.clone(),
            self.response_queue_sx5.clone(),
        );
        self.runtime
            .spawn(async move { start_http_server(http_state).await });
    }

    fn start_cluster_heartbeat_report(&self) {
        let client_poll = self.client_poll.clone();
        self.runtime
            .spawn(async move { report_heartbeat(client_poll).await });
    }

    fn start_keep_alive_thread(&self) {
        let keep_alive = KeepAlive::new(
            HEART_CONNECT_SHARD_HASH_NUM,
            self.heartbeat_manager.clone(),
            self.request_queue_sx4.clone(),
            self.request_queue_sx5.clone(),
        );
        self.runtime.spawn(async move {
            keep_alive.start_heartbeat_check().await;
        });
    }

    pub fn awaiting_stop(&self, stop_send: broadcast::Sender<bool>) {
        // Wait for the stop signal
        self.runtime.block_on(async move {
            loop {
                signal::ctrl_c().await.expect("failed to listen for event");
                match stop_send.send(true) {
                    Ok(_) => {
                        info_meta("When ctrl + c is received, the service starts to stop");
                        self.stop_server().await;
                        break;
                    }
                    Err(_) => {}
                }
            }
        });

        // todo tokio runtime shutdown
    }

    fn register_node(&self) {
        self.runtime.block_on(async move {
            register_broker_node(self.client_poll.clone()).await;
        });
    }

    async fn stop_server(&self) {
        // unregister node
        unregister_broker_node(self.client_poll.clone()).await;
    }
}
