use super::{
    manager::SubscribeManager,
    subscribe::{max_qos, share_sub_rewrite_publish_flag},
};
use crate::{
    core::metadata_cache::MetadataCacheManager,
    metadata::message::Message,
    server::{tcp::packet::ResponsePackage, MQTTProtocol},
    storage::message::MessageStorage,
};
use bytes::Bytes;
use common_base::{
    config::broker_mqtt::broker_mqtt_conf,
    log::{error, info},
};
use dashmap::DashMap;
use protocol::mqtt::{MQTTPacket, Publish, PublishProperties};
use std::{sync::Arc, time::Duration};
use storage_adapter::{record::Record, storage::StorageAdapter};
use tokio::{
    sync::{
        broadcast,
        mpsc::{self, Receiver, Sender},
    },
    time::sleep,
};

const SHARED_SUBSCRIPTION_STRATEGY_ROUND_ROBIN: &str = "round_robin";
const SHARED_SUBSCRIPTION_STRATEGY_RANDOM: &str = "random";
const SHARED_SUBSCRIPTION_STRATEGY_STICKY: &str = "sticky";
const SHARED_SUBSCRIPTION_STRATEGY_HASH: &str = "hash";
const SHARED_SUBSCRIPTION_STRATEGY_LOCAL: &str = "local";

#[derive(Clone)]
pub struct SubscribeShareLeader<S> {
    // (topic_id, Sender<bool>)
    pub leader_pull_data_thread: DashMap<String, Sender<bool>>,

    // (topic_id, Sender<bool>)
    pub leader_push_data_thread: DashMap<String, Sender<bool>>,

    pub subscribe_manager: Arc<SubscribeManager>,
    message_storage: Arc<S>,
    response_queue_sx4: broadcast::Sender<ResponsePackage>,
    response_queue_sx5: broadcast::Sender<ResponsePackage>,
    metadata_cache: Arc<MetadataCacheManager>,
}

impl<S> SubscribeShareLeader<S>
where
    S: StorageAdapter + Sync + Send + 'static + Clone,
{
    pub fn new(
        subscribe_manager: Arc<SubscribeManager>,
        message_storage: Arc<S>,
        response_queue_sx4: broadcast::Sender<ResponsePackage>,
        response_queue_sx5: broadcast::Sender<ResponsePackage>,
        metadata_cache: Arc<MetadataCacheManager>,
    ) -> Self {
        return SubscribeShareLeader {
            leader_pull_data_thread: DashMap::with_capacity(128),
            leader_push_data_thread: DashMap::with_capacity(128),
            subscribe_manager,
            message_storage,
            response_queue_sx4,
            response_queue_sx5,
            metadata_cache,
        };
    }

    pub async fn start(&self) {
        let conf = broker_mqtt_conf();
        loop {
            for (topic_id, sub_list) in self.subscribe_manager.share_leader_subscribe.clone() {
                if sub_list.len() == 0 {
                    // stop pull data thread
                    if let Some(sx) = self.leader_pull_data_thread.get(&topic_id) {
                        match sx.send(true).await {
                            Ok(_) => {
                                self.leader_pull_data_thread.remove(&topic_id);
                            }
                            Err(e) => error(e.to_string()),
                        }
                    }

                    // stop push data thread
                    if let Some(sx) = self.leader_push_data_thread.get(&topic_id) {
                        match sx.send(true).await {
                            Ok(_) => {
                                self.leader_push_data_thread.remove(&topic_id);
                            }
                            Err(e) => error(e.to_string()),
                        }
                    }

                    self.subscribe_manager
                        .share_leader_subscribe
                        .remove(&topic_id);
                    continue;
                }

                let (sx, rx) = mpsc::channel(10000);

                let subscribe_manager = self.subscribe_manager.clone();
                // start pull data thread
                if !self.leader_pull_data_thread.contains_key(&topic_id) {
                    self.start_topic_pull_data_thread(topic_id.clone(), sx)
                        .await;
                }

                // start push data thread
                if !self.leader_push_data_thread.contains_key(&topic_id) {
                    // round_robin
                    if conf.subscribe.shared_subscription_strategy
                        == SHARED_SUBSCRIPTION_STRATEGY_ROUND_ROBIN.to_string()
                    {
                        self.start_push_by_round_robin(topic_id.clone(), rx, subscribe_manager);
                    }

                    // random
                    if conf.subscribe.shared_subscription_strategy
                        == SHARED_SUBSCRIPTION_STRATEGY_RANDOM.to_string()
                    {
                        self.start_push_by_random();
                    }

                    // sticky
                    if conf.subscribe.shared_subscription_strategy
                        == SHARED_SUBSCRIPTION_STRATEGY_STICKY.to_string()
                    {
                        self.start_push_by_sticky();
                    }

                    // hash
                    if conf.subscribe.shared_subscription_strategy
                        == SHARED_SUBSCRIPTION_STRATEGY_HASH.to_string()
                    {
                        self.start_push_by_hash();
                    }

                    // local
                    if conf.subscribe.shared_subscription_strategy
                        == SHARED_SUBSCRIPTION_STRATEGY_LOCAL.to_string()
                    {
                        self.start_push_by_local();
                    }
                }
            }

            sleep(Duration::from_secs(1)).await;
        }
    }

    pub async fn start_topic_pull_data_thread(&self, topic_id: String, channel_sx: Sender<Record>) {
        let (sx, mut rx) = mpsc::channel(1);
        self.leader_pull_data_thread.insert(topic_id.clone(), sx);
        let message_storage = self.message_storage.clone();
        tokio::spawn(async move {
            info(format!(
                "Share push thread for Topic [{}] was started successfully",
                topic_id
            ));
            let message_storage = MessageStorage::new(message_storage);
            let group_id = format!("system_sub_{}", topic_id);
            let record_num = 100;
            let max_wait_ms = 500;
            loop {
                match rx.try_recv() {
                    Ok(flag) => {
                        if flag {
                            info(format!(
                                "Exclusive Push thread for Topic [{}] was stopped successfully",
                                topic_id
                            ));
                            break;
                        }
                    }
                    Err(_) => {}
                }

                match message_storage
                    .read_topic_message(topic_id.clone(), group_id.clone(), record_num as u128)
                    .await
                {
                    Ok(results) => {
                        if results.len() == 0 {
                            sleep(Duration::from_millis(max_wait_ms)).await;
                            return;
                        }

                        // commit offset
                        if let Some(last_res) = results.last() {
                            match message_storage
                                .commit_group_offset(
                                    topic_id.clone(),
                                    group_id.clone(),
                                    last_res.offset,
                                )
                                .await
                            {
                                Ok(_) => {}
                                Err(e) => {
                                    error(e.to_string());
                                    return;
                                }
                            }
                        }

                        // Push data to subscribers
                        for record in results.clone() {
                            match channel_sx.send(record).await {
                                Ok(_) => {}
                                Err(e) => error(e.to_string()),
                            }
                        }
                    }
                    Err(e) => {
                        error(e.to_string());
                        sleep(Duration::from_millis(max_wait_ms)).await;
                    }
                }
            }
        });
    }

    pub fn start_push_by_round_robin(
        &self,
        topic_id: String,
        mut channel_rx: Receiver<Record>,
        subscribe_manager: Arc<SubscribeManager>,
    ) {
        let (sx, mut rx) = mpsc::channel(1);
        self.leader_push_data_thread.insert(topic_id.clone(), sx);
        let response_queue_sx4 = self.response_queue_sx4.clone();
        let response_queue_sx5 = self.response_queue_sx5.clone();
        let metadata_cache = self.metadata_cache.clone();

        tokio::spawn(async move {
            info(format!(
                "Share push thread for Topic [{}] was started successfully",
                topic_id
            ));

            loop {
                match rx.try_recv() {
                    Ok(flag) => {
                        if flag {
                            info(format!(
                                "Exclusive Push thread for Topic [{}] was stopped successfully",
                                topic_id
                            ));
                            break;
                        }
                    }
                    Err(_) => {}
                }

                if let Some(sub_list) = subscribe_manager.share_leader_subscribe.get(&topic_id) {
                    for (_, subscribe) in sub_list.clone() {
                        let connect_id = if let Some(sess) =
                            metadata_cache.session_info.get(&subscribe.client_id)
                        {
                            if let Some(conn_id) = sess.connection_id {
                                conn_id
                            } else {
                                continue;
                            }
                        } else {
                            continue;
                        };

                        let topic_name: String =
                            if let Some(topic_name) = metadata_cache.topic_id_name.get(&topic_id) {
                                topic_name.clone()
                            } else {
                                continue;
                            };
                        if let Some(record) = channel_rx.recv().await {
                            let msg: Message = match Message::decode_record(record) {
                                Ok(msg) => msg,
                                Err(e) => {
                                    error(e.to_string());
                                    return;
                                }
                            };
                            let mut sub_id = Vec::new();
                            if let Some(id) = subscribe.subscription_identifier {
                                sub_id.push(id);
                            }

                            let publish = Publish {
                                dup: false,
                                qos: max_qos(msg.qos, subscribe.qos),
                                pkid: subscribe.packet_identifier,
                                retain: false,
                                topic: Bytes::from(topic_name),
                                payload: msg.payload,
                            };

                            // If it is a shared subscription, it will be identified with the push message
                            let mut user_properteis = Vec::new();
                            user_properteis.push(share_sub_rewrite_publish_flag());

                            let properties = PublishProperties {
                                payload_format_indicator: None,
                                message_expiry_interval: None,
                                topic_alias: None,
                                response_topic: None,
                                correlation_data: None,
                                user_properties: user_properteis,
                                subscription_identifiers: sub_id.clone(),
                                content_type: None,
                            };
                            let resp: ResponsePackage = ResponsePackage {
                                connection_id: connect_id,
                                packet: MQTTPacket::Publish(publish, Some(properties)),
                            };
                            publish_to_client(
                                subscribe.protocol,
                                resp,
                                response_queue_sx4.clone(),
                                response_queue_sx5.clone(),
                            )
                            .await;
                        }
                    }
                }
            }
        });
    }

    fn start_push_by_random(&self) {}

    fn start_push_by_hash(&self) {}

    fn start_push_by_sticky(&self) {}

    fn start_push_by_local(&self) {}
}

pub async fn publish_to_client(
    protocol: MQTTProtocol,
    resp: ResponsePackage,
    response_queue_sx4: broadcast::Sender<ResponsePackage>,
    response_queue_sx5: broadcast::Sender<ResponsePackage>,
) {
    if protocol == MQTTProtocol::MQTT4 {
        match response_queue_sx4.send(resp) {
            Ok(_) => {}
            Err(e) => error(format!("{}", e.to_string())),
        }
    } else if protocol == MQTTProtocol::MQTT5 {
        match response_queue_sx5.send(resp) {
            Ok(_) => {}
            Err(e) => error(format!("{}", e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::subscribe::subscribe::{decode_share_info, is_share_sub};

    #[tokio::test]
    async fn is_share_sub_test() {
        let sub1 = "$share/consumer1/sport/tennis/+".to_string();
        let sub2 = "$share/consumer2/sport/tennis/+".to_string();
        let sub3 = "$share/consumer1/sport/#".to_string();
        let sub4 = "$share/comsumer1/finance/#".to_string();

        assert!(is_share_sub(sub1));
        assert!(is_share_sub(sub2));
        assert!(is_share_sub(sub3));
        assert!(is_share_sub(sub4));

        let sub5 = "/comsumer1/$share/finance/#".to_string();
        let sub6 = "/comsumer1/$share/finance/$share".to_string();

        assert!(!is_share_sub(sub5));
        assert!(!is_share_sub(sub6));
    }

    #[tokio::test]
    async fn decode_share_info_test() {
        let sub1 = "$share/consumer1/sport/tennis/+".to_string();
        let sub2 = "$share/consumer2/sport/tennis/+".to_string();
        let sub3 = "$share/consumer1/sport/#".to_string();
        let sub4 = "$share/comsumer1/finance/#".to_string();

        let (group_name, topic_name) = decode_share_info(sub1);
        assert_eq!(group_name, "consumer1".to_string());
        assert_eq!(topic_name, "/sport/tennis/+".to_string());

        let (group_name, topic_name) = decode_share_info(sub2);
        assert_eq!(group_name, "consumer2".to_string());
        assert_eq!(topic_name, "/sport/tennis/+".to_string());

        let (group_name, topic_name) = decode_share_info(sub3);
        assert_eq!(group_name, "consumer1".to_string());
        assert_eq!(topic_name, "/sport/#".to_string());

        let (group_name, topic_name) = decode_share_info(sub4);
        assert_eq!(group_name, "comsumer1".to_string());
        assert_eq!(topic_name, "/finance/#".to_string());
    }
}