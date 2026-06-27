//! Typed storage for [`BaselineSnapshot`] records.

use std::sync::Arc;

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

use modelsentry_common::{
    error::Result,
    models::BaselineSnapshot,
    types::{BaselineId, ProbeId},
};

const TABLE: TableDefinition<&str, &[u8]> =
    TableDefinition::new(modelsentry_common::constants::table::BASELINES);

/// Typed CRUD for [`BaselineSnapshot`] records.
pub struct BaselineStore {
    db: Arc<Database>,
}

impl BaselineStore {
    pub(crate) fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Persist a baseline snapshot.
    ///
    /// # Errors
    ///
    /// Returns a database error or a serialization error.
    pub fn insert(&self, baseline: &BaselineSnapshot) -> Result<()> {
        let bytes = serde_json::to_vec(baseline)?;
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(TABLE)?;
            let id = baseline.id.to_string();
            table.insert(id.as_str(), bytes.as_slice())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    /// Return the most recently captured baseline for `probe_id`, if any.
    ///
    /// # Errors
    ///
    /// Returns a database error on transaction errors.
    pub fn get_latest_for_probe(&self, probe_id: &ProbeId) -> Result<Option<BaselineSnapshot>> {
        let all = self.list_for_probe(probe_id)?;
        Ok(all.into_iter().max_by_key(|b| b.captured_at))
    }

    /// Return all baselines for a probe, in unspecified order.
    ///
    /// # Errors
    ///
    /// Returns a database error on transaction errors.
    pub fn list_for_probe(&self, probe_id: &ProbeId) -> Result<Vec<BaselineSnapshot>> {
        let read_txn = self.db.begin_read()?;
        let table: redb::ReadOnlyTable<&str, &[u8]> = read_txn.open_table(TABLE)?;
        let mut baselines = Vec::new();
        for entry in table.iter()? {
            let (_, v) = entry?;
            let baseline: BaselineSnapshot = serde_json::from_slice(v.value())?;
            if &baseline.probe_id == probe_id {
                baselines.push(baseline);
            }
        }
        Ok(baselines)
    }

    /// Delete a baseline by id. Returns `false` if not found.
    ///
    /// # Errors
    ///
    /// Returns a database error on transaction/commit errors.
    pub fn delete(&self, id: &BaselineId) -> Result<bool> {
        let write_txn = self.db.begin_write()?;
        let existed = {
            let mut table = write_txn.open_table(TABLE)?;
            let id_str = id.to_string();
            table.remove(id_str.as_str())?.is_some()
        };
        write_txn.commit()?;
        Ok(existed)
    }

    /// Delete all baselines associated with `probe_id`.
    ///
    /// Returns the number of records deleted.
    ///
    /// # Errors
    ///
    /// Returns a database error on transaction errors.
    pub fn delete_for_probe(&self, probe_id: &ProbeId) -> Result<usize> {
        let ids_to_delete: Vec<String> = {
            let read_txn = self.db.begin_read()?;
            let table: redb::ReadOnlyTable<&str, &[u8]> = read_txn.open_table(TABLE)?;
            let mut ids = Vec::new();
            for entry in table.iter()? {
                let (k, v) = entry?;
                let b: BaselineSnapshot = serde_json::from_slice(v.value())?;
                if &b.probe_id == probe_id {
                    ids.push(k.value().to_owned());
                }
            }
            ids
        };

        let count = ids_to_delete.len();
        if count == 0 {
            return Ok(0);
        }

        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(TABLE)?;
            for id in &ids_to_delete {
                table.remove(id.as_str())?;
            }
        }
        write_txn.commit()?;
        Ok(count)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};
    use tempfile::TempDir;

    use modelsentry_common::{
        models::BaselineSnapshot,
        types::{BaselineId, ProbeId, RunId},
    };

    use crate::AppStore;

    fn open_test_db() -> (TempDir, AppStore) {
        let dir = TempDir::new().unwrap();
        let store = AppStore::open(&dir.path().join("test.db")).unwrap();
        (dir, store)
    }

    fn make_baseline(probe_id: &ProbeId, offset_secs: i64) -> BaselineSnapshot {
        BaselineSnapshot {
            id: BaselineId::new(),
            probe_id: probe_id.clone(),
            captured_at: Utc::now() + Duration::seconds(offset_secs),
            schema_version: modelsentry_common::models::BASELINE_SCHEMA_VERSION,
            embedding_model: "test".into(),
            prompt_clouds: vec![vec![vec![1.0, 2.0], vec![1.1, 1.9]]],
            n_runs: 1,
            run_id: RunId::new(),
        }
    }

    #[test]
    fn get_latest_returns_most_recent_baseline() {
        let (_dir, store) = open_test_db();
        let probe_id = ProbeId::new();
        let older = make_baseline(&probe_id, 0);
        let newer = make_baseline(&probe_id, 10);
        store.baselines().insert(&older).unwrap();
        store.baselines().insert(&newer).unwrap();
        let latest = store
            .baselines()
            .get_latest_for_probe(&probe_id)
            .unwrap()
            .unwrap();
        assert_eq!(latest.id, newer.id);
    }

    #[test]
    fn multiple_baselines_for_same_probe_stored_correctly() {
        let (_dir, store) = open_test_db();
        let probe_id = ProbeId::new();
        let b1 = make_baseline(&probe_id, 0);
        let b2 = make_baseline(&probe_id, 5);
        let b3 = make_baseline(&probe_id, 10);
        store.baselines().insert(&b1).unwrap();
        store.baselines().insert(&b2).unwrap();
        store.baselines().insert(&b3).unwrap();
        let all = store.baselines().list_for_probe(&probe_id).unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn list_for_probe_filters_by_probe_id() {
        let (_dir, store) = open_test_db();
        let probe_a = ProbeId::new();
        let probe_b = ProbeId::new();
        store
            .baselines()
            .insert(&make_baseline(&probe_a, 0))
            .unwrap();
        store
            .baselines()
            .insert(&make_baseline(&probe_b, 0))
            .unwrap();
        let all_a = store.baselines().list_for_probe(&probe_a).unwrap();
        assert_eq!(all_a.len(), 1);
    }
}
