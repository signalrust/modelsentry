//! Typed storage for [`AlertRule`] and [`AlertEvent`] records.

use std::sync::Arc;

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use uuid::Uuid;

use modelsentry_common::{
    error::{ModelSentryError, Result},
    models::{AlertEvent, AlertRule},
    types::{AlertRuleId, ProbeId},
};

const RULES_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("alert_rules");
const EVENTS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("alert_events");

/// Typed CRUD for [`AlertRule`] and [`AlertEvent`] records.
pub struct AlertRuleStore {
    db: Arc<Database>,
}

impl AlertRuleStore {
    pub(crate) fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Persist an alert rule.
    ///
    /// # Errors
    ///
    /// Returns [`ModelSentryError::Db`] or [`ModelSentryError::Serialization`].
    pub fn insert_rule(&self, rule: &AlertRule) -> Result<()> {
        let bytes = serde_json::to_vec(rule)?;
        let write_txn = self
            .db
            .begin_write()
            .map_err(|e| ModelSentryError::Db(e.to_string()))?;
        {
            let mut table = write_txn
                .open_table(RULES_TABLE)
                .map_err(|e| ModelSentryError::Db(e.to_string()))?;
            let id = rule.id.to_string();
            table.insert(id.as_str(), bytes.as_slice())?;
        }
        write_txn
            .commit()
            .map_err(|e| ModelSentryError::Db(e.to_string()))?;
        Ok(())
    }

    /// Return all alert rules for a probe.
    ///
    /// # Errors
    ///
    /// Returns [`ModelSentryError::Db`] on transaction errors.
    pub fn get_rules_for_probe(&self, probe_id: &ProbeId) -> Result<Vec<AlertRule>> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|e| ModelSentryError::Db(e.to_string()))?;
        let table: redb::ReadOnlyTable<&str, &[u8]> = read_txn
            .open_table(RULES_TABLE)
            .map_err(|e| ModelSentryError::Db(e.to_string()))?;
        let mut rules = Vec::new();
        for entry in table.iter()? {
            let (_, v) = entry?;
            let rule: AlertRule = serde_json::from_slice(v.value())?;
            if &rule.probe_id == probe_id {
                rules.push(rule);
            }
        }
        Ok(rules)
    }

    /// Delete an alert rule. Returns `false` if not found.
    ///
    /// # Errors
    ///
    /// Returns [`ModelSentryError::Db`] on transaction/commit errors.
    pub fn delete_rule(&self, id: &AlertRuleId) -> Result<bool> {
        let write_txn = self
            .db
            .begin_write()
            .map_err(|e| ModelSentryError::Db(e.to_string()))?;
        let existed = {
            let mut table = write_txn
                .open_table(RULES_TABLE)
                .map_err(|e| ModelSentryError::Db(e.to_string()))?;
            let id_str = id.to_string();
            table.remove(id_str.as_str())?.is_some()
        };
        write_txn
            .commit()
            .map_err(|e| ModelSentryError::Db(e.to_string()))?;
        Ok(existed)
    }

    /// Persist an alert event.
    ///
    /// # Errors
    ///
    /// Returns [`ModelSentryError::Db`] or [`ModelSentryError::Serialization`].
    pub fn insert_event(&self, event: &AlertEvent) -> Result<()> {
        let bytes = serde_json::to_vec(event)?;
        let write_txn = self
            .db
            .begin_write()
            .map_err(|e| ModelSentryError::Db(e.to_string()))?;
        {
            let mut table = write_txn
                .open_table(EVENTS_TABLE)
                .map_err(|e| ModelSentryError::Db(e.to_string()))?;
            let id = event.id.to_string();
            table.insert(id.as_str(), bytes.as_slice())?;
        }
        write_txn
            .commit()
            .map_err(|e| ModelSentryError::Db(e.to_string()))?;
        Ok(())
    }

    /// Return the `limit` most-recently-fired alert events.
    ///
    /// # Errors
    ///
    /// Returns [`ModelSentryError::Db`] on transaction errors.
    pub fn list_events(&self, limit: usize) -> Result<Vec<AlertEvent>> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|e| ModelSentryError::Db(e.to_string()))?;
        let table: redb::ReadOnlyTable<&str, &[u8]> = read_txn
            .open_table(EVENTS_TABLE)
            .map_err(|e| ModelSentryError::Db(e.to_string()))?;
        let mut events = Vec::new();
        for entry in table.iter()? {
            let (_, v) = entry?;
            let event: AlertEvent = serde_json::from_slice(v.value())?;
            events.push(event);
        }
        events.sort_by(|a, b| b.fired_at.cmp(&a.fired_at));
        Ok(events.into_iter().take(limit).collect())
    }

    /// Mark an alert event as acknowledged. Returns `false` if the event is not found.
    ///
    /// # Errors
    ///
    /// Returns [`ModelSentryError::Db`] on transaction/commit errors.
    pub fn acknowledge_event(&self, id: &Uuid) -> Result<bool> {
        let write_txn = self
            .db
            .begin_write()
            .map_err(|e| ModelSentryError::Db(e.to_string()))?;
        let updated = {
            let mut table = write_txn
                .open_table(EVENTS_TABLE)
                .map_err(|e| ModelSentryError::Db(e.to_string()))?;
            let id_str = id.to_string();
            // Read into an owned Vec to release the immutable borrow before insert.
            let existing: Option<Vec<u8>> = {
                let guard = table.get(id_str.as_str())?;
                guard.map(|g| g.value().to_vec())
            };
            match existing {
                None => false,
                Some(raw) => {
                    let mut event: AlertEvent = serde_json::from_slice(&raw)?;
                    event.acknowledged = true;
                    let bytes = serde_json::to_vec(&event)?;
                    table.insert(id_str.as_str(), bytes.as_slice())?;
                    true
                }
            }
        };
        write_txn
            .commit()
            .map_err(|e| ModelSentryError::Db(e.to_string()))?;
        Ok(updated)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use tempfile::TempDir;
    use uuid::Uuid;

    use modelsentry_common::{
        models::{AlertChannel, AlertEvent, AlertRule, DriftLevel, DriftReport},
        types::{AlertRuleId, BaselineId, ProbeId, RunId},
    };

    use crate::AppStore;

    fn open_test_db() -> (TempDir, AppStore) {
        let dir = TempDir::new().unwrap();
        let store = AppStore::open(&dir.path().join("test.db")).unwrap();
        (dir, store)
    }

    fn make_rule(probe_id: &ProbeId) -> AlertRule {
        AlertRule {
            id: AlertRuleId::new(),
            probe_id: probe_id.clone(),
            kl_threshold: 0.5,
            cosine_threshold: 0.3,
            channels: vec![AlertChannel::Webhook {
                url: "https://example.com/hook".into(),
            }],
            active: true,
        }
    }

    fn make_event(rule_id: &AlertRuleId) -> AlertEvent {
        AlertEvent {
            id: Uuid::new_v4(),
            rule_id: rule_id.clone(),
            drift_report: DriftReport {
                run_id: RunId::new(),
                baseline_id: BaselineId::new(),
                kl_divergence: 1.0,
                cosine_distance: 0.5,
                output_entropy_delta: 0.2,
                drift_level: DriftLevel::High,
                computed_at: Utc::now(),
            },
            fired_at: Utc::now(),
            acknowledged: false,
        }
    }

    #[test]
    fn acknowledge_event_sets_acknowledged_true() {
        let (_dir, store) = open_test_db();
        let probe_id = ProbeId::new();
        let rule = make_rule(&probe_id);
        let event = make_event(&rule.id);
        store.alerts().insert_event(&event).unwrap();
        assert!(store.alerts().acknowledge_event(&event.id).unwrap());
        let all = store.alerts().list_events(10).unwrap();
        assert!(all[0].acknowledged);
    }

    #[test]
    fn acknowledge_nonexistent_event_returns_false() {
        let (_dir, store) = open_test_db();
        let result = store.alerts().acknowledge_event(&Uuid::new_v4()).unwrap();
        assert!(!result);
    }

    #[test]
    fn insert_and_get_rules_for_probe() {
        let (_dir, store) = open_test_db();
        let probe_id = ProbeId::new();
        let rule = make_rule(&probe_id);
        store.alerts().insert_rule(&rule).unwrap();
        let rules = store.alerts().get_rules_for_probe(&probe_id).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].id, rule.id);
    }

    #[test]
    fn list_events_returns_newest_first() {
        let (_dir, store) = open_test_db();
        let probe_id = ProbeId::new();
        let rule = make_rule(&probe_id);
        let mut e1 = make_event(&rule.id);
        let mut e2 = make_event(&rule.id);
        // ensure distinct fired_at values
        e1.fired_at = Utc::now();
        e2.fired_at = e1.fired_at + chrono::Duration::seconds(10);
        store.alerts().insert_event(&e1).unwrap();
        store.alerts().insert_event(&e2).unwrap();
        let events = store.alerts().list_events(10).unwrap();
        assert_eq!(events[0].id, e2.id);
    }
}
