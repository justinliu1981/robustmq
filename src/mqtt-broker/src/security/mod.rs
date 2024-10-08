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

use crate::{
    handler::{cache::CacheManager, connection::Connection},
    subscribe::sub_common::get_sub_topic_id_list,
};
use acl::is_allow_acl;
use axum::async_trait;
use clients::poll::ClientPool;
use common_base::{
    config::{broker_mqtt::broker_mqtt_conf, common::Auth},
    error::{common::CommonError, mqtt_broker::MQTTBrokerError},
};
use dashmap::DashMap;
use login::{plaintext::Plaintext, Authentication};
use metadata_struct::{
    acl::{
        mqtt_acl::{MQTTAcl, MQTTAclAction},
        mqtt_blacklist::MQTTAclBlackList,
    },
    mqtt::user::MQTTUser,
};
use mysql::MySQLAuthStorageAdapter;
use placement::PlacementAuthStorageAdapter;
use protocol::mqtt::common::{ConnectProperties, Login, QoS, Subscribe};
use std::{net::SocketAddr, sync::Arc};
use storage_adapter::{storage_is_mysql, storage_is_placement};

pub mod acl;
pub mod login;
pub mod mysql;
pub mod placement;
pub mod redis;

#[async_trait]
pub trait AuthStorageAdapter {
    async fn read_all_user(&self) -> Result<DashMap<String, MQTTUser>, CommonError>;

    async fn read_all_acl(&self) -> Result<Vec<MQTTAcl>, CommonError>;

    async fn read_all_blacklist(&self) -> Result<Vec<MQTTAclBlackList>, CommonError>;

    async fn get_user(&self, username: String) -> Result<Option<MQTTUser>, CommonError>;
}

pub struct AuthDriver {
    cache_manager: Arc<CacheManager>,
    client_poll: Arc<ClientPool>,
    driver: Arc<dyn AuthStorageAdapter + Send + 'static + Sync>,
}

impl AuthDriver {
    pub fn new(cache_manager: Arc<CacheManager>, client_poll: Arc<ClientPool>) -> AuthDriver {
        let conf = broker_mqtt_conf();
        let driver = match build_driver(client_poll.clone(), conf.auth.clone()) {
            Ok(driver) => driver,
            Err(e) => {
                panic!("{}", e.to_string());
            }
        };
        return AuthDriver {
            cache_manager,
            driver: driver,
            client_poll,
        };
    }

    pub fn update_driver(&mut self, auth: Auth) -> Result<(), CommonError> {
        let driver = match build_driver(self.client_poll.clone(), auth) {
            Ok(driver) => driver,
            Err(e) => {
                return Err(e);
            }
        };
        self.driver = driver;
        return Ok(());
    }

    pub async fn read_all_user(&self) -> Result<DashMap<String, MQTTUser>, CommonError> {
        return self.driver.read_all_user().await;
    }

    pub async fn read_all_acl(&self) -> Result<Vec<MQTTAcl>, CommonError> {
        return self.driver.read_all_acl().await;
    }

    pub async fn read_all_blacklist(&self) -> Result<Vec<MQTTAclBlackList>, CommonError> {
        return self.driver.read_all_blacklist().await;
    }

    pub async fn check_login_auth(
        &self,
        login: &Option<Login>,
        _: &Option<ConnectProperties>,
        _: &SocketAddr,
    ) -> Result<bool, CommonError> {
        let cluster = self.cache_manager.get_cluster_info();

        if cluster.security.secret_free_login {
            return Ok(true);
        }

        if let Some(info) = login {
            return self
                .plaintext_check_login(&info.username, &info.password)
                .await;
        }

        return Ok(false);
    }

    pub async fn allow_publish(
        &self,
        connection: &Connection,
        topic_name: &String,
        retain: bool,
        qos: QoS,
    ) -> bool {
        return is_allow_acl(
            &self.cache_manager,
            connection,
            topic_name,
            MQTTAclAction::Publish,
            retain,
            qos,
        );
    }

    pub async fn allow_subscribe(&self, connection: &Connection, subscribe: &Subscribe) -> bool {
        for filter in subscribe.filters.clone() {
            let topic_list = get_sub_topic_id_list(self.cache_manager.clone(), filter.path).await;
            for topic in topic_list {
                if !is_allow_acl(
                    &self.cache_manager,
                    connection,
                    &topic,
                    MQTTAclAction::Publish,
                    false,
                    filter.qos,
                ) {
                    return false;
                }
            }
        }
        return true;
    }

    async fn plaintext_check_login(
        &self,
        username: &String,
        password: &String,
    ) -> Result<bool, CommonError> {
        let plaintext = Plaintext::new(
            username.clone(),
            password.clone(),
            self.cache_manager.clone(),
        );
        match plaintext.apply().await {
            Ok(flag) => {
                if flag {
                    return Ok(true);
                }
            }
            Err(e) => {
                // If the user does not exist, try to get the user information from the storage layer
                if e.to_string() == MQTTBrokerError::UserDoesNotExist.to_string() {
                    return self.try_get_check_user_by_driver(username).await;
                }
                return Err(e.into());
            }
        }

        return Ok(false);
    }

    async fn try_get_check_user_by_driver(&self, username: &String) -> Result<bool, CommonError> {
        match self.driver.get_user(username.clone()).await {
            Ok(Some(user)) => {
                self.cache_manager.add_user(user.clone());
                let plaintext = Plaintext::new(
                    user.username.clone(),
                    user.password.clone(),
                    self.cache_manager.clone(),
                );
                match plaintext.apply().await {
                    Ok(flag) => {
                        if flag {
                            return Ok(true);
                        }
                    }
                    Err(e) => {
                        return Err(e.into());
                    }
                }
            }
            Ok(None) => {
                return Ok(false);
            }
            Err(e) => {
                return Err(e);
            }
        }
        return Ok(false);
    }
}

pub fn build_driver(
    client_poll: Arc<ClientPool>,
    auth: Auth,
) -> Result<Arc<dyn AuthStorageAdapter + Send + 'static + Sync>, CommonError> {
    if storage_is_placement(&auth.storage_type) {
        let driver = PlacementAuthStorageAdapter::new(client_poll);
        return Ok(Arc::new(driver));
    }

    if storage_is_mysql(&auth.storage_type) {
        let driver = MySQLAuthStorageAdapter::new(auth.mysql_addr.clone());
        return Ok(Arc::new(driver));
    }

    return Err(CommonError::UnavailableStorageType);
}

pub fn authentication_acl() -> bool {
    return false;
}
