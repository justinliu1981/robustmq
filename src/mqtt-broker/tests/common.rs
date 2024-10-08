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

use paho_mqtt::{
    Client, ConnectOptions, ConnectOptionsBuilder, CreateOptions, CreateOptionsBuilder,
    DisconnectOptionsBuilder, Properties, PropertyCode, ReasonCode,
};
use std::{process, time::Duration};

#[allow(dead_code)]
pub fn broker_addr() -> String {
    return "tcp://127.0.0.1:1883".to_string();
}

#[allow(dead_code)]
pub fn username() -> String {
    return "admin".to_string();
}

#[allow(dead_code)]
pub fn password() -> String {
    return "pwd123".to_string();
}

#[allow(dead_code)]
pub fn err_password() -> String {
    return "pwd1235".to_string();
}

#[allow(dead_code)]
pub fn build_v5_pros() -> Properties {
    let mut props = Properties::new();
    props
        .push_u32(PropertyCode::SessionExpiryInterval, 3)
        .unwrap();
    props.push_u16(PropertyCode::ReceiveMaximum, 128).unwrap();
    props
        .push_u32(PropertyCode::MaximumPacketSize, 2048)
        .unwrap();
    props
        .push_u16(PropertyCode::TopicAliasMaximum, 128)
        .unwrap();
    props
        .push_val(PropertyCode::RequestResponseInformation, 1)
        .unwrap();
    props
        .push_val(PropertyCode::RequestProblemInformation, 1)
        .unwrap();
    props
        .push_string_pair(PropertyCode::UserProperty, "lobo1", "1")
        .unwrap();
    props
        .push_string_pair(PropertyCode::UserProperty, "lobo2", "2")
        .unwrap();
    return props;
}

#[allow(dead_code)]
pub fn build_v5_conn_pros(props: Properties, err_pwd: bool) -> ConnectOptions {
    let pwd = if err_pwd { err_password() } else { password() };
    let conn_opts = ConnectOptionsBuilder::new_v5()
        .keep_alive_interval(Duration::from_secs(20))
        .clean_start(true)
        .connect_timeout(Duration::from_secs(5))
        .properties(props.clone())
        .user_name(username())
        .password(pwd)
        .finalize();
    return conn_opts;
}

#[allow(dead_code)]
pub fn build_v3_conn_pros(mqtt_version: u32, err_pwd: bool) -> ConnectOptions {
    let pwd = if err_pwd { err_password() } else { password() };
    let conn_opts = ConnectOptionsBuilder::with_mqtt_version(mqtt_version)
        .keep_alive_interval(Duration::from_secs(20))
        .clean_session(true)
        .connect_timeout(Duration::from_secs(5))
        .user_name(username())
        .password(pwd)
        .finalize();
    return conn_opts;
}

#[allow(dead_code)]
pub fn build_create_pros(client_id: &String, addr: &String) -> CreateOptions {
    let create_opts = if client_id.is_empty() {
        CreateOptionsBuilder::new()
            .server_uri(addr.clone())
            .finalize()
    } else {
        CreateOptionsBuilder::new()
            .server_uri(addr.clone())
            .client_id(client_id.clone())
            .finalize()
    };
    return create_opts;
}

#[allow(dead_code)]
pub fn distinct_conn(cli: Client) {
    let disconnect_opts = DisconnectOptionsBuilder::new()
        .reason_code(ReasonCode::DisconnectWithWillMessage)
        .finalize();
    cli.disconnect(disconnect_opts).unwrap();
}

#[allow(dead_code)]
pub fn connect_server34(mqtt_version: u32, client_id: &String, addr: &String) -> Client {
    let create_opts = build_create_pros(client_id, addr);
    let cli = Client::new(create_opts).unwrap_or_else(|err| {
        println!("Error creating the client: {:?}", err);
        process::exit(1);
    });

    let conn_opts = build_v3_conn_pros(mqtt_version, false);

    match cli.connect(conn_opts) {
        Ok(_) => {}
        Err(e) => {
            println!("Unable to connect:\n\t{:?},{}", e, addr);
            assert!(false)
        }
    }
    return cli;
}

#[allow(dead_code)]
pub fn connect_server5(client_id: &String, addr: &String) -> Client {
    let mqtt_version = 5;
    let props = build_v5_pros();

    let create_opts = build_create_pros(client_id, addr);
    let cli = Client::new(create_opts).unwrap_or_else(|err| {
        println!("Error creating the client: {:?}", err);
        process::exit(1);
    });

    let conn_opts = build_v5_conn_pros(props.clone(), false);
    match cli.connect(conn_opts) {
        Ok(response) => {
            let resp = response.connect_response().unwrap();
            assert_eq!(format!("tcp://{}", resp.server_uri), broker_addr());
            assert_eq!(mqtt_version, resp.mqtt_version);
            assert!(resp.session_present);
            assert_eq!(response.reason_code(), ReasonCode::Success);
        }
        Err(e) => {
            println!("Unable to connect:\n\t{:?}", e);
            process::exit(1);
        }
    }
    return cli;
}
