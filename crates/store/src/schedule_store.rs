//! Persistent per-probe scheduler state.
//!
//! Stores each probe's **next scheduled run time** so the daemon resumes a
//! probe's cadence across restarts instead of re-phasing every probe to "one
//! interval from process start". On restart an overdue probe runs once
//! immediately (catch-up), then resumes its normal schedule.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use redb::{Database, ReadableDatabase, TableDefinition};

use modelsentry_common::error::Result;
use modelsentry_common::types::ProbeId;

const TABLE: TableDefinition<&str, &[u8]> =
    TableDefinition::new(modelsentry_common::constants::table::SCHEDULE_STATE);

/// Typed access to per-probe scheduler state (the next-run timestamp).
pub struct ScheduleStore {
    db: Arc<Database>,
}

impl ScheduleStore {
    pub(crate) fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// The persisted next-run time for `probe_id`, or `None` if the probe has no
    /// stored schedule state yet (e.g. a brand-new probe).
    ///
    /// # Errors
    ///
    /// Returns a database error or a deserialization error.
    pub fn get_next_run(&self, probe_id: &ProbeId) -> Result<Option<DateTime<Utc>>> {
        let read_txn = self.db.begin_read()?;
        let table: redb::ReadOnlyTable<&str, &[u8]> = read_txn.open_table(TABLE)?;
        let key = probe_id.to_string();
        match table.get(key.as_str())? {
            Some(guard) => Ok(Some(serde_json::from_slice(guard.value())?)),
            None => Ok(None),
        }
    }

    /// Persist the next-run time for `probe_id` (overwriting any prior value).
    ///
    /// # Errors
    ///
    /// Returns a database error or a serialization error.
    pub fn set_next_run(&self, probe_id: &ProbeId, next_run: DateTime<Utc>) -> Result<()> {
        let bytes = serde_json::to_vec(&next_run)?;
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(TABLE)?;
            let key = probe_id.to_string();
            table.insert(key.as_str(), bytes.as_slice())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    /// Remove the stored schedule state for `probe_id`. Returns `true` if a
    /// record was present. Used when a probe is deleted or its schedule edited.
    ///
    /// # Errors
    ///
    /// Returns a database error on transaction/commit errors.
    pub fn delete(&self, probe_id: &ProbeId) -> Result<bool> {
        let write_txn = self.db.begin_write()?;
        let existed = {
            let mut table = write_txn.open_table(TABLE)?;
            let key = probe_id.to_string();
            table.remove(key.as_str())?.is_some()
        };
        write_txn.commit()?;
        Ok(existed)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};
    use tempfile::TempDir;

    use modelsentry_common::types::ProbeId;

    use crate::AppStore;

    fn open_test_db() -> (TempDir, AppStore) {
        let dir = TempDir::new().unwrap();
        let store = AppStore::open(&dir.path().join("test.db")).unwrap();
        (dir, store)
    }

    #[test]
    fn next_run_round_trips_and_overwrites() {
        let (_dir, store) = open_test_db();
        let probe_id = ProbeId::new();

        assert!(store.schedule().get_next_run(&probe_id).unwrap().is_none());

        let first = Utc::now() + Duration::minutes(5);
        store.schedule().set_next_run(&probe_id, first).unwrap();
        assert_eq!(
            store.schedule().get_next_run(&probe_id).unwrap(),
            Some(first)
        );

        let second = first + Duration::minutes(60);
        store.schedule().set_next_run(&probe_id, second).unwrap();
        assert_eq!(
            store.schedule().get_next_run(&probe_id).unwrap(),
            Some(second)
        );
    }

    #[test]
    fn delete_removes_state() {
        let (_dir, store) = open_test_db();
        let probe_id = ProbeId::new();
        store
            .schedule()
            .set_next_run(&probe_id, Utc::now())
            .unwrap();
        assert!(store.schedule().delete(&probe_id).unwrap());
        assert!(!store.schedule().delete(&probe_id).unwrap());
        assert!(store.schedule().get_next_run(&probe_id).unwrap().is_none());
    }
}
