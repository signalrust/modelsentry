//! Typed storage for [`ProbeRun`] records.
//!
//! A run is split across three tables so the hot path — listing the recent runs
//! for a probe — never decodes the heavy embeddings:
//! - **metadata** ([`table::RUNS`]): the run minus its embeddings, keyed by run
//!   id (direct fetch by id).
//! - **embeddings** ([`table::RUN_EMBEDDINGS`]): the per-prompt output
//!   embeddings, keyed by run id, read only by baseline capture.
//! - **index** ([`table::RUN_INDEX`]): `{probe_id}|{rev_ts}|{run_id}`, so the
//!   most-recent N runs for a probe are a bounded range scan — list latency no
//!   longer scales with total run count or embedding volume.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use redb::{Database, ReadableDatabase, TableDefinition};

use modelsentry_common::{
    constants::table,
    error::Result,
    models::ProbeRun,
    types::{ProbeId, RunId},
};

const RUNS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new(table::RUNS);
const EMBEDDINGS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new(table::RUN_EMBEDDINGS);
const INDEX_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new(table::RUN_INDEX);

/// Zero-pad width for the reversed-timestamp segment of an index key, so a
/// probe's runs sort chronologically: the number of decimal digits in `i64::MAX`
/// (the largest possible reversed value), derived rather than hard-coded.
const REV_TS_WIDTH: usize = i64::MAX.ilog10() as usize + 1;

/// Typed CRUD for [`ProbeRun`] records.
pub struct RunStore {
    db: Arc<Database>,
}

impl RunStore {
    pub(crate) fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Index key `"{probe_id}|{rev_ts}|{run_id}"`. `rev_ts = i64::MAX − nanos`
    /// (zero-padded) makes a *newer* run sort *earlier*, so a forward range scan
    /// over a probe's prefix yields newest-first without a sort. The trailing
    /// run id breaks ties and lets the listing recover the metadata key.
    fn index_key(probe_id: &ProbeId, started_at: DateTime<Utc>, run_id: &RunId) -> String {
        let nanos = started_at.timestamp_nanos_opt().unwrap_or(0).max(0);
        let rev = i64::MAX - nanos;
        let width = REV_TS_WIDTH;
        format!("{probe_id}|{rev:0width$}|{run_id}")
    }

    /// Persist a probe run: metadata to [`table::RUNS`], embeddings to
    /// [`table::RUN_EMBEDDINGS`], and an ordering entry to [`table::RUN_INDEX`],
    /// all in one transaction.
    ///
    /// # Errors
    ///
    /// Returns a database error or a serialization error.
    pub fn insert(&self, run: &ProbeRun) -> Result<()> {
        // Metadata = the run without its heavy embeddings (serialized with
        // `embeddings` empty; it deserializes back via `#[serde(default)]`).
        // Cloning then clearing is cheap relative to a run's provider calls and
        // happens once per run.
        let mut meta = run.clone();
        meta.embeddings = Vec::new();
        let meta_bytes = serde_json::to_vec(&meta)?;
        let embed_bytes = serde_json::to_vec(&run.embeddings)?;
        let id = run.id.to_string();
        let index_key = Self::index_key(&run.probe_id, run.started_at, &run.id);

        let write_txn = self.db.begin_write()?;
        {
            let mut runs = write_txn.open_table(RUNS_TABLE)?;
            runs.insert(id.as_str(), meta_bytes.as_slice())?;
            let mut embeds = write_txn.open_table(EMBEDDINGS_TABLE)?;
            embeds.insert(id.as_str(), embed_bytes.as_slice())?;
            let mut index = write_txn.open_table(INDEX_TABLE)?;
            index.insert(index_key.as_str(), id.as_bytes())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    /// Retrieve a run's **metadata** by id (embeddings are not loaded — fetch
    /// them with [`Self::embeddings`]).
    ///
    /// # Errors
    ///
    /// Returns a database error on transaction errors.
    pub fn get(&self, id: &RunId) -> Result<Option<ProbeRun>> {
        let read_txn = self.db.begin_read()?;
        let table: redb::ReadOnlyTable<&str, &[u8]> = read_txn.open_table(RUNS_TABLE)?;
        let id_str = id.to_string();
        match table.get(id_str.as_str())? {
            Some(guard) => Ok(Some(serde_json::from_slice(guard.value())?)),
            None => Ok(None),
        }
    }

    /// The per-prompt output embeddings for a run, or `None` if the run is
    /// unknown. Used by baseline capture to aggregate runs into clouds.
    ///
    /// # Errors
    ///
    /// Returns a database error or a deserialization error.
    pub fn embeddings(&self, id: &RunId) -> Result<Option<Vec<Vec<Vec<f32>>>>> {
        let read_txn = self.db.begin_read()?;
        let table: redb::ReadOnlyTable<&str, &[u8]> = read_txn.open_table(EMBEDDINGS_TABLE)?;
        let id_str = id.to_string();
        match table.get(id_str.as_str())? {
            Some(guard) => Ok(Some(serde_json::from_slice(guard.value())?)),
            None => Ok(None),
        }
    }

    /// Return the `limit` most-recent runs (**metadata only**) for `probe_id`,
    /// newest first — a bounded range scan over the time-ordered index, so cost
    /// is `O(limit)`, independent of total run count or embedding size.
    ///
    /// # Errors
    ///
    /// Returns a database error on transaction errors.
    pub fn list_for_probe(&self, probe_id: &ProbeId, limit: usize) -> Result<Vec<ProbeRun>> {
        let prefix = format!("{probe_id}|");
        let read_txn = self.db.begin_read()?;
        let index: redb::ReadOnlyTable<&str, &[u8]> = read_txn.open_table(INDEX_TABLE)?;
        let runs: redb::ReadOnlyTable<&str, &[u8]> = read_txn.open_table(RUNS_TABLE)?;
        let mut out = Vec::new();
        for entry in index.range(prefix.as_str()..)? {
            let (k, v) = entry?;
            // The index is one contiguous block per probe; stop at the next.
            if !k.value().starts_with(prefix.as_str()) {
                break;
            }
            if let Ok(run_id) = std::str::from_utf8(v.value()) {
                if let Some(guard) = runs.get(run_id)? {
                    out.push(serde_json::from_slice(guard.value())?);
                }
            }
            if out.len() >= limit {
                break;
            }
        }
        Ok(out)
    }

    /// Delete all runs (metadata, embeddings, and index entries) associated with
    /// `probe_id`. Returns the number of runs deleted. Bounded to this probe's
    /// runs via the index — no full-table scan.
    ///
    /// # Errors
    ///
    /// Returns a database error on transaction errors.
    pub fn delete_for_probe(&self, probe_id: &ProbeId) -> Result<usize> {
        let prefix = format!("{probe_id}|");
        // Collect (index_key, run_id) in a read txn so the write txn does not
        // hold a read borrow while iterating.
        let targets: Vec<(String, String)> = {
            let read_txn = self.db.begin_read()?;
            let index: redb::ReadOnlyTable<&str, &[u8]> = read_txn.open_table(INDEX_TABLE)?;
            let mut v = Vec::new();
            for entry in index.range(prefix.as_str()..)? {
                let (k, val) = entry?;
                if !k.value().starts_with(prefix.as_str()) {
                    break;
                }
                let run_id = String::from_utf8_lossy(val.value()).into_owned();
                v.push((k.value().to_owned(), run_id));
            }
            v
        };

        let count = targets.len();
        if count == 0 {
            return Ok(0);
        }

        let write_txn = self.db.begin_write()?;
        {
            let mut runs = write_txn.open_table(RUNS_TABLE)?;
            let mut embeds = write_txn.open_table(EMBEDDINGS_TABLE)?;
            let mut index = write_txn.open_table(INDEX_TABLE)?;
            for (index_key, run_id) in &targets {
                runs.remove(run_id.as_str())?;
                embeds.remove(run_id.as_str())?;
                index.remove(index_key.as_str())?;
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
            // Two prompts, one sample each — exercises the embeddings split.
            embeddings: vec![vec![vec![1.0, 2.0]], vec![vec![3.0, 4.0]]],
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

    #[test]
    fn get_and_list_return_metadata_without_embeddings() {
        let (_dir, store) = open_test_db();
        let probe_id = ProbeId::new();
        let run = make_run(&probe_id, 0);
        store.runs().insert(&run).unwrap();

        // Metadata reads must not carry the heavy embeddings (kept apart).
        let got = store.runs().get(&run.id).unwrap().unwrap();
        assert!(got.embeddings.is_empty(), "get must omit embeddings");
        assert_eq!(got.completions, run.completions, "metadata still present");

        let listed = store.runs().list_for_probe(&probe_id, 10).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].embeddings.is_empty(), "list must omit embeddings");
    }

    #[test]
    fn embeddings_round_trip_via_dedicated_table() {
        let (_dir, store) = open_test_db();
        let probe_id = ProbeId::new();
        let run = make_run(&probe_id, 0);
        store.runs().insert(&run).unwrap();

        let embeddings = store.runs().embeddings(&run.id).unwrap().unwrap();
        assert_eq!(embeddings, run.embeddings);

        // Unknown run → None.
        assert!(store.runs().embeddings(&RunId::new()).unwrap().is_none());
    }

    #[test]
    fn delete_for_probe_removes_metadata_index_and_embeddings() {
        let (_dir, store) = open_test_db();
        let probe_id = ProbeId::new();
        let other = ProbeId::new();
        let run = make_run(&probe_id, 0);
        let keep = make_run(&other, 0);
        store.runs().insert(&run).unwrap();
        store.runs().insert(&keep).unwrap();

        let deleted = store.runs().delete_for_probe(&probe_id).unwrap();
        assert_eq!(deleted, 1);

        // Everything for the deleted probe is gone — metadata, list, embeddings.
        assert!(store.runs().get(&run.id).unwrap().is_none());
        assert!(store.runs().embeddings(&run.id).unwrap().is_none());
        assert!(
            store
                .runs()
                .list_for_probe(&probe_id, 10)
                .unwrap()
                .is_empty()
        );

        // The other probe's run is untouched (delete was bounded to one probe).
        assert!(store.runs().get(&keep.id).unwrap().is_some());
        assert_eq!(store.runs().list_for_probe(&other, 10).unwrap().len(), 1);
    }
}
