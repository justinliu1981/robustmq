use common::config::meta::MetaConfig;
use raft::prelude::ConfState;
use raft::prelude::Entry;
use raft::prelude::Snapshot;
use raft::Error;
use raft::RaftState;
use raft::Result as RaftResult;
use raft::Storage as RaftStorage;
use raft::StorageError;
use raft_proto::eraftpb::HardState;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::RwLockReadGuard;
use std::sync::RwLockWriteGuard;

use super::data::convert_conf_state_from_rds_cs;
use super::data::convert_entry_from_rds_save_entry;
use super::data::convert_hard_state_from_rds_hs;
use super::raft_core::RaftRocksDBStorageCore;

#[derive(Clone)]
pub struct RaftRocksDBStorage {
    core: Arc<RwLock<RaftRocksDBStorageCore>>,
}

impl RaftRocksDBStorage {
    pub fn new(config: &MetaConfig) -> Self {
        let core = RaftRocksDBStorageCore::new(config);
        return RaftRocksDBStorage {
            core: Arc::new(RwLock::new(core)),
        };
    }

    pub fn new_with_conf_state<T>(config: &MetaConfig, conf_state: T) -> RaftRocksDBStorage
    where
        ConfState: From<T>,
    {
        let store = RaftRocksDBStorage::new(config);
        store.initialize_with_conf_state(conf_state);
        return store;
    }

    pub fn initialize_with_conf_state<T>(&self, conf_state: T)
    where
        ConfState: From<T>,
    {
        assert!(!self.initial_state().unwrap().initialized());
        let _ = self
            .write_lock()
            .save_conf_state(ConfState::from(conf_state));
    }

    pub fn read_lock(&self) -> RwLockReadGuard<'_, RaftRocksDBStorageCore> {
        self.core.read().unwrap()
    }

    pub fn write_lock(&self) -> RwLockWriteGuard<'_, RaftRocksDBStorageCore> {
        self.core.write().unwrap()
    }
}

impl RaftRocksDBStorage {
    pub fn apply_snapshot(&mut self, snapshot: Snapshot) -> RaftResult<()> {
        let mut store = self.core.write().unwrap();
        let _ = store.apply_snapshot(snapshot);
        Ok(())
    }

    pub fn append(&mut self, entrys: &Vec<Entry>) -> RaftResult<()> {
        let mut store = self.core.write().unwrap();
        let _ = store.append(entrys);
        return Ok(());
    }

    pub fn set_hard_state(&mut self, hs: &HardState) -> RaftResult<()> {
        let store = self.core.write().unwrap();
        let _ = store.save_hard_state(hs);
        return Ok(());
    }

    pub fn set_hard_state_comit(&mut self, hs: u64) -> RaftResult<()> {
        let store = self.core.write().unwrap();
        let _ = store.set_hard_state_commit(hs);
        return Ok(());
    }
}

impl RaftStorage for RaftRocksDBStorage {
    /// `initial_state` is called when Raft is initialized. This interface will return a `RaftState`
    /// which contains `HardState` and `ConfState`.
    ///
    /// `RaftState` could be initialized or not. If it's initialized it means the `Storage` is
    /// created with a configuration, and its last index and term should be greater than 0.
    fn initial_state(&self) -> RaftResult<RaftState> {
        let core = self.read_lock();
        let shs = core.hard_state();
        let scs = core.conf_state();
        Ok(RaftState {
            hard_state: convert_hard_state_from_rds_hs(shs),
            conf_state: convert_conf_state_from_rds_cs(scs),
        })
    }

    /// Returns a slice of log entries in the range `[low, high)`.
    /// max_size limits the total size of the log entries returned if not `None`, however
    /// the slice of entries returned will always have length at least 1 if entries are
    /// found in the range.
    ///
    /// Entries are supported to be fetched asynchronously depending on the context. Async is optional.
    /// Storage should check context.can_async() first and decide whether to fetch entries asynchronously
    /// based on its own implementation. If the entries are fetched asynchronously, storage should return
    /// LogTemporarilyUnavailable, and application needs to call `on_entries_fetched(context)` to trigger
    /// re-fetch of the entries after the storage finishes fetching the entries.
    ///
    /// # Panics
    ///
    /// Panics if `high` is higher than `Storage::last_index(&self) + 1`.
    fn entries(
        &self,
        low: u64,
        high: u64,
        max_size: impl Into<Option<u64>>,
        context: raft::GetEntriesContext,
    ) -> RaftResult<Vec<Entry>> {
        let max_size = max_size.into();
        let mut core = self.read_lock();
        if low < core.first_index() {
            return Err(Error::Store(StorageError::Compacted));
        }

        if high > core.last_index() + 1 {
            panic!(
                "index out of bound (last: {}, high: {})",
                core.last_index() + 1,
                high
            )
        }
        let mut entry_list: Vec<Entry> = Vec::new();
        for idx in low..=high {
            let sret = core.get_entry_by_idx(idx);
            entry_list.push(convert_entry_from_rds_save_entry(sret).unwrap());
        }
        return Ok(entry_list);
    }

    /// Returns the term of entry idx, which must be in the range
    /// [first_index()-1, last_index()]. The term of the entry before
    /// first_index is retained for matching purpose even though the
    /// rest of that entry may not be available.
    fn term(&self, idx: u64) -> RaftResult<u64> {
        let core = self.read_lock();

        if idx == core.snapshot_metadata.index {
            return Ok(core.snapshot_metadata.index);
        }

        if idx < core.first_index(){
            return Err(Error::Store(StorageError::Compacted));
        }

        if idx > core.last_index(){
            return Err(Error::Store(StorageError::Unavailable));
        }

        let value = core.get_entry_by_idx(idx);
        return Ok(value.term);
    }

    /// Returns the index of the first log entry that is possible available via entries, which will
    /// always equal to `truncated index` plus 1.
    ///
    /// New created (but not initialized) `Storage` can be considered as truncated at 0 so that 1
    /// will be returned in this case.
    fn first_index(&self) -> RaftResult<u64> {
        Ok(self.read_lock().first_index())
    }

    /// The index of the last entry replicated in the `Storage`.
    fn last_index(&self) -> RaftResult<u64> {
        Ok(self.read_lock().last_index())
    }

    /// Returns the most recent snapshot.
    ///
    /// If snapshot is temporarily unavailable, it should return SnapshotTemporarilyUnavailable,
    /// so raft state machine could know that Storage needs some time to prepare
    /// snapshot and call snapshot later.
    /// A snapshot's index must not less than the `request_index`.
    /// `to` indicates which peer is requesting the snapshot.
    fn snapshot(&self, request_index: u64, to: u64) -> RaftResult<Snapshot> {
        let mut core = self.write_lock();
        if core.trigger_snap_unavailable {
            core.trigger_snap_unavailable = false;
            return Err(Error::Store(StorageError::SnapshotTemporarilyUnavailable));
        } else {
            let mut snap = core.snapshot();
            if snap.get_metadata().index < request_index {
                snap.mut_metadata().index = request_index;
            }
            Ok(snap)
        }
    }
}
mod tests {
    use common::config::meta::MetaConfig;
    use super::RaftRocksDBStorage;
    #[test]
    fn running() {
        let mut config = MetaConfig::default();
        config.data_path ="/tmp/data_1".to_string();
        let mut s = RaftRocksDBStorage::new(&config);
    }
}