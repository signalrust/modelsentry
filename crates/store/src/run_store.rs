//! Typed storage for [`ProbeRun`] records.

use std::sync::Arc;

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

use modelsentry_common::{
    error::{ModelSentryError, Result},
    models::ProbeRun,
    types::{ProbeId, RunId},
};

const TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("runs");

/// Typed CRUD for [`ProbeRun`] records.
pub struct RunStore {
    db: Arc<Database>,
}

impl RunStore {
    pub(crate) fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Persist a probe run.
    ///
    /// # Errors
    ///
    /// Returns [`ModelSentryError::Db`] or [`ModelSentryError::Serialization`].
    pub fn insert(&self, run: &ProbeRun) -> Result<()> {
        let bytes = serde_json::to_vec(run)?;
        let write_txn = self
            .db
            .begin_write()
            .map_err(|e| ModelSentryError::Db(e.to_string()))?;
        {
            let mut table = write_txn
                .open_table(TABLE)
                .map_err(|e| ModelSentryError::Db(e.to_string()))?;
            let id = run.id.to_string();
            table.insert(id.as_str(), bytes.as_slice())?;
        }
        write_txn
            .commit()
            .map_err(|e| ModelSentryError::Db(e.to_string()))?;
        Ok(())
    }

    /// Retrieve a run by id.
    ///
    /// # Errors
    ///
    /// Returns [`ModelSentryError::Db`] on transaction errors.
    pub fn get(&self, id: &RunId) -> Result<Option<ProbeRun>> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|e| ModelSentryError::Db(e.to_string()))?;
        let table: redb::ReadOnlyTable<&str, &[u8]> = read_txn
            .open_table(TABLE)
            .map_err(|e| ModelSentryError::Db(e.to_string()))?;
        let id_str = id.to_string();
        match table.get(id_str.as_str())? {
            Some(guard) => {
                let run: ProbeRun = serde_json::from_slice(guard.value())?;
                Ok(Some(run))
            }
            None => Ok(None),
        }
    }

    /// Return the `limit` most-recent runs for `probe_id`, newest first.
    ///
    /// # Errors
    ///
    /// Returns [`ModelSentryError::Db`] on transaction errors.
    pub fn list_for_probe(&self, probe_id: &ProbeId, limit: usize) -> Result<Vec<ProbeRun>> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|e| ModelSentryError::Db(e.to_string()))?;
        let table: redb::ReadOnlyTable<&str, &[u8]> = read_txn
            .open_table(TABLE)
            .map_err(|e| ModelSentryError::Db(e.to_string()))?;
        let mut runs = Vec::new();
        for entry in table.iter()? {
            let (_, v) = entry?;
            let run: ProbeRun = serde_json::from_slice(v.value())?;
            if &run.probe_id == probe_id {
                runs.push(run);
            }
        }
        runs.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        Ok(runs.into_iter().take(limit).collect())
    }

    /// Delete all runs associated with `probe_id`.
    ///
    /// Returns the number of records deleted.
    ///
    /// # Errors
    ///
    /// Returns [`ModelSentryError::Db`] on transaction errors.
    pub fn delete_for_probe(&self, probe_id: &ProbeId) -> Result<usize> {
        // Collect IDs in a read transaction first to avoid holding a write lock
        // while iterating.
        let ids_to_delete: Vec<String> = {
            let read_txn = self
                .db
                .begin_read()
                .map_err(|e| ModelSentryError::Db(e.to_string()))?;
            let table: redb::ReadOnlyTable<&str, &[u8]> = read_txn
                .open_table(TABLE)
                .map_err(|e| ModelSentryError::Db(e.to_string()))?;
            let mut ids = Vec::new();
            for entry in table.iter()? {
                let (k, v) = entry?;
                let run: ProbeRun = serde_json::from_slice(v.value())?;
                if &run.probe_id == probe_id {
                    ids.push(k.value().to_owned());
                }
            }
            ids
        };

        let count = ids_to_delete.len();
        if count == 0 {
            return Ok(0);
        }

        let write_txn = self
            .db
            .begin_write()
            .map_err(|e| ModelSentryError::Db(e.to_string()))?;
        {
            let mut table = write_txn
                .open_table(TABLE)
                .map_err(|e| ModelSentryError::Db(e.to_string()))?;
            for id in &ids_to_delete {
                table.remove(id.as_str())?;
            }
        }
        write_txn
            .commit()
            .map_err(|e| ModelSentryError::Db(e.to_string()))?;
        Ok(count)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};
    use tempfile::TempDir;

    use modelsentry_common::{
        models::{ProbeRun, RunStatus},
        types::{ProbeId, RunId},
    };

    use crate::AppStore;

    fn open_test_db() -> (TempDir, AppStore) {
        let dir = TempDir::new().unwrap();
        let store = AppStore::open(&dir.path().join("test.db")).unwrap();
        (dir, store)
    }

    fn make_run(probe_id: &ProbeId, offset_secs: i64) -> ProbeRun {
        let t = Utc::now() + Duration::seconds(offset_secs);
        ProbeRun {
            id: RunId::new(),
            probe_id: probe_id.clone(),
            started_at: t,
            finished_at: t + Duration::seconds(1),
            embeddings: vec![],
            completions: vec!["ok".into()],
            drift_report: None,
            status: RunStatus::Success,
        }
    }

    #[test]
    fn list_for_probe_respects_limit() {
        let (_dir, store) = open_test_db();
        let probe_id = ProbeId::new();
        for i in 0..5 {
            store.runs().insert(&make_run(&probe_id, i)).unwrap();
        }
        let runs = store.runs().list_for_probe(&probe_id, 3).unwrap();
        assert_eq!(runs.len(), 3);
    }

    #[test]
    fn list_for_probe_ordered_newest_first() {
        let (_dir, store) = open_test_db();
        let probe_id = ProbeId::new();
        let old = make_run(&probe_id, 0);
        let new = make_run(&probe_id, 100);
        store.runs().insert(&old).unwrap();
        store.runs().insert(&new).unwrap();
        let runs = store.runs().list_for_probe(&probe_id, 10).unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].id, new.id);
        assert_eq!(runs[1].id, old.id);
    }

    #[test]
    fn get_run_by_id() {
        let (_dir, store) = open_test_db();
        let probe_id = ProbeId::new();
        let run = make_run(&probe_id, 0);
        store.runs().insert(&run).unwrap();
        let got = store.runs().get(&run.id).unwrap().unwrap();
        assert_eq!(got.id, run.id);
    }
}
