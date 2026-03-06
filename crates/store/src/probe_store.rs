//! Typed storage for [`Probe`] records.

use std::sync::Arc;

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

use modelsentry_common::{
    error::{ModelSentryError, Result},
    models::Probe,
    types::ProbeId,
};

const TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("probes");

/// Typed CRUD for [`Probe`] records.
pub struct ProbeStore {
    db: Arc<Database>,
}

impl ProbeStore {
    pub(crate) fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Persist `probe`, overwriting any existing record with the same id.
    ///
    /// # Errors
    ///
    /// Returns [`ModelSentryError::Db`] on transaction/commit errors or
    /// [`ModelSentryError::Serialization`] if the model cannot be serialized.
    pub fn insert(&self, probe: &Probe) -> Result<()> {
        let bytes = serde_json::to_vec(probe)?;
        let write_txn = self
            .db
            .begin_write()
            .map_err(|e| ModelSentryError::Db(e.to_string()))?;
        {
            let mut table = write_txn
                .open_table(TABLE)
                .map_err(|e| ModelSentryError::Db(e.to_string()))?;
            let id = probe.id.to_string();
            table.insert(id.as_str(), bytes.as_slice())?;
        }
        write_txn
            .commit()
            .map_err(|e| ModelSentryError::Db(e.to_string()))?;
        Ok(())
    }

    /// Retrieve a probe by id. Returns `None` if not found.
    ///
    /// # Errors
    ///
    /// Returns [`ModelSentryError::Db`] on transaction errors or
    /// [`ModelSentryError::Serialization`] on deserialization failure.
    pub fn get(&self, id: &ProbeId) -> Result<Option<Probe>> {
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
                let probe: Probe = serde_json::from_slice(guard.value())?;
                Ok(Some(probe))
            }
            None => Ok(None),
        }
    }

    /// Return all stored probes in unspecified order.
    ///
    /// # Errors
    ///
    /// Returns [`ModelSentryError::Db`] on transaction errors.
    pub fn list_all(&self) -> Result<Vec<Probe>> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|e| ModelSentryError::Db(e.to_string()))?;
        let table: redb::ReadOnlyTable<&str, &[u8]> = read_txn
            .open_table(TABLE)
            .map_err(|e| ModelSentryError::Db(e.to_string()))?;
        let mut probes = Vec::new();
        for entry in table.iter()? {
            let (_, v) = entry?;
            let probe: Probe = serde_json::from_slice(v.value())?;
            probes.push(probe);
        }
        Ok(probes)
    }

    /// Delete a probe by id. Returns `false` if no record existed.
    ///
    /// # Errors
    ///
    /// Returns [`ModelSentryError::Db`] on transaction/commit errors.
    pub fn delete(&self, id: &ProbeId) -> Result<bool> {
        let write_txn = self
            .db
            .begin_write()
            .map_err(|e| ModelSentryError::Db(e.to_string()))?;
        let existed = {
            let mut table = write_txn
                .open_table(TABLE)
                .map_err(|e| ModelSentryError::Db(e.to_string()))?;
            let id_str = id.to_string();
            table.remove(id_str.as_str())?.is_some()
        };
        write_txn
            .commit()
            .map_err(|e| ModelSentryError::Db(e.to_string()))?;
        Ok(existed)
    }

    /// Update an existing probe (same as `insert` — redb overwrites by key).
    ///
    /// # Errors
    ///
    /// Same as [`insert`](Self::insert).
    pub fn update(&self, probe: &Probe) -> Result<()> {
        self.insert(probe)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use tempfile::TempDir;
    use uuid::Uuid;

    use modelsentry_common::{
        models::{Probe, ProbePrompt, ProbeSchedule, ProviderKind},
        types::ProbeId,
    };

    // remove unused super import – items accessed via crate::AppStore
    use crate::AppStore;

    fn open_test_db() -> (TempDir, AppStore) {
        let dir = TempDir::new().unwrap();
        let store = AppStore::open(&dir.path().join("test.db")).unwrap();
        (dir, store)
    }

    fn make_probe() -> Probe {
        Probe {
            id: ProbeId::new(),
            name: "my-probe".into(),
            provider: ProviderKind::Anthropic,
            model: "claude-3-7-sonnet-20250219".into(),
            prompts: vec![ProbePrompt {
                id: Uuid::new_v4(),
                text: "hello".into(),
                expected_contains: None,
                expected_not_contains: None,
            }],
            schedule: ProbeSchedule::EveryMinutes { minutes: 10 },
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn insert_and_get_probe() {
        let (_dir, store) = open_test_db();
        let probe = make_probe();
        store.probes().insert(&probe).unwrap();
        let got = store.probes().get(&probe.id).unwrap().unwrap();
        assert_eq!(got.id, probe.id);
        assert_eq!(got.name, probe.name);
    }

    #[test]
    fn get_nonexistent_probe_returns_none() {
        let (_dir, store) = open_test_db();
        let result = store.probes().get(&ProbeId::new()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn delete_probe_returns_false_if_not_found() {
        let (_dir, store) = open_test_db();
        let existed = store.probes().delete(&ProbeId::new()).unwrap();
        assert!(!existed);
    }

    #[test]
    fn list_all_returns_all_inserted_probes() {
        let (_dir, store) = open_test_db();
        let a = make_probe();
        let b = make_probe();
        store.probes().insert(&a).unwrap();
        store.probes().insert(&b).unwrap();
        let all = store.probes().list_all().unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn delete_probe_returns_true_when_found() {
        let (_dir, store) = open_test_db();
        let probe = make_probe();
        store.probes().insert(&probe).unwrap();
        assert!(store.probes().delete(&probe.id).unwrap());
        assert!(store.probes().get(&probe.id).unwrap().is_none());
    }
}
