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

use axum::async_trait;
use common_base::error::common::CommonError;
use metadata_struct::adapter::record::Record;

#[derive(Default)]
pub struct ShardConfig {}

#[async_trait]
pub trait StorageAdapter {
    async fn create_shard(
        &self,
        shard_name: String,
        shard_config: ShardConfig,
    ) -> Result<(), CommonError>;

    async fn delete_shard(&self, shard_name: String) -> Result<(), CommonError>;

    // kv storage model: Set data
    async fn set(&self, key: String, value: Record) -> Result<(), CommonError>;

    // kv storage model: Get data
    async fn get(&self, key: String) -> Result<Option<Record>, CommonError>;

    // kv storage model: Delete data
    async fn delete(&self, key: String) -> Result<(), CommonError>;

    // kv storage model: Determines whether the key exists
    async fn exists(&self, key: String) -> Result<bool, CommonError>;

    // Streaming storage model: Append data in a Shard dimension, returning a unique self-incrementing ID for the Shard dimension
    async fn stream_write(
        &self,
        shard_name: String,
        data: Vec<Record>,
    ) -> Result<Vec<usize>, CommonError>;

    // Streaming storage model: Read the next batch of data in the dimension of the Shard + subscription name tuple
    async fn stream_read(
        &self,
        shard_name: String,
        group_id: String,
        record_num: Option<u128>,
        record_size: Option<usize>,
    ) -> Result<Option<Vec<Record>>, CommonError>;

    // Streaming storage model: Read the next batch of data in the dimension of the Shard + subscription name tuple
    async fn stream_commit_offset(
        &self,
        shard_name: String,
        group_id: String,
        offset: u128,
    ) -> Result<bool, CommonError>;

    // Streaming storage model: A piece of data is uniquely read based on the shard name and a unique auto-incrementing ID.
    async fn stream_read_by_offset(
        &self,
        shard_name: String,
        record_id: usize,
    ) -> Result<Option<Record>, CommonError>;

    // Streaming storage model: A batch of data is read based on the shard name and time range.
    async fn stream_read_by_timestamp(
        &self,
        shard_name: String,
        start_timestamp: u128,
        end_timestamp: u128,
        record_num: Option<usize>,
        record_size: Option<usize>,
    ) -> Result<Option<Vec<Record>>, CommonError>;

    // Streaming storage model: A batch of data is read based on the shard name and the last time it expires
    async fn stream_read_by_key(
        &self,
        shard_name: String,
        key: String,
    ) -> Result<Option<Record>, CommonError>;
}
