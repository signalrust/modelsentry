//! Persistence layer for `ModelSentry` — typed wrappers around a `redb` database.
//!
//! Entry point is [`AppStore::open`], which creates or opens the database file
//! and initialises all tables.  Use the accessor methods ([`probes`],
//! [`baselines`], [`runs`], [`alerts`]) to obtain cheap, `Clone`-able store
//! handles backed by a shared [`redb::Database`].
//!
//! [`probes`]: AppStore::probes
//! [`baselines`]: AppStore::baselines
//! [`runs`]: AppStore::runs
//! [`alerts`]: AppStore::alerts

pub mod alert_store;
pub mod baseline_store;
pub mod probe_store;
pub mod run_store;

pub use alert_store::AlertRuleStore;
pub use baseline_store::BaselineStore;
pub use probe_store::ProbeStore;
pub use run_store::RunStore;

use std::{path::Path, sync::Arc};

use redb::{Database, TableDefinition};

use modelsentry_common::constants::table;
use modelsentry_common::error::Result;
use modelsentry_common::types::ProbeId;

// Pre-declare all tables so that a fresh database has them after `open()`.
const PROBES_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new(table::PROBES);
const BASELINES_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new(table::BASELINES);
const RUNS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new(table::RUNS);
const ALERT_RULES_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new(table::ALERT_RULES);
const ALERT_EVENTS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new(table::ALERT_EVENTS);

/// Combined handle to all store modules.
///
/// Cheaply cloneable — all clones share the same underlying [`Database`].
#[derive(Clone)]
pub struct AppStore {
    db: Arc<Database>,
}

impl AppStore {
    /// Open (or create) the database at `path` and initialise all tables.
    ///
    /// # Errors
    ///
    /// Returns a database error if the database file cannot be created
    /// or opened, or if table initialisation fails.
    pub fn open(path: &Path) -> Result<Self> {
        let db = Database::create(path)?;
        let store = Self { db: Arc::new(db) };
        store.init_tables()?;
        Ok(store)
    }

    /// Return a [`ProbeStore`] backed by this database.
    #[must_use]
    pub fn probes(&self) -> ProbeStore {
        ProbeStore::new(Arc::clone(&self.db))
    }

    /// Return a [`BaselineStore`] backed by this database.
    #[must_use]
    pub fn baselines(&self) -> BaselineStore {
        BaselineStore::new(Arc::clone(&self.db))
    }

    /// Return a [`RunStore`] backed by this database.
    #[must_use]
    pub fn runs(&self) -> RunStore {
        RunStore::new(Arc::clone(&self.db))
    }

    /// Return an [`AlertRuleStore`] backed by this database.
    #[must_use]
    pub fn alerts(&self) -> AlertRuleStore {
        AlertRuleStore::new(Arc::clone(&self.db))
    }

    /// Delete a probe and all its associated runs and baselines atomically.
    ///
    /// Returns `false` if the probe was not found (no partial work is done).
    ///
    /// # Errors
    ///
    /// Returns a database error or a serialization error.
    pub fn delete_probe_cascade(&self, id: &ProbeId) -> Result<bool> {
        let found = self.probes().delete(id)?;
        if !found {
            return Ok(false);
        }
        self.runs().delete_for_probe(id)?;
        self.baselines().delete_for_probe(id)?;
        Ok(true)
    }

    /// Eagerly create all tables so that read transactions never encounter
    /// a missing table on a fresh database.
    fn init_tables(&self) -> Result<()> {
        let write_txn = self.db.begin_write()?;
        write_txn.open_table(PROBES_TABLE)?;
        write_txn.open_table(BASELINES_TABLE)?;
        write_txn.open_table(RUNS_TABLE)?;
        write_txn.open_table(ALERT_RULES_TABLE)?;
        write_txn.open_table(ALERT_EVENTS_TABLE)?;
        write_txn.commit()?;
        Ok(())
    }
}
