//! Typed storage for [`AlertRule`] and [`AlertEvent`] records.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use uuid::Uuid;

use modelsentry_common::{
    error::Result,
    models::{AlertEvent, AlertRule},
    types::{AlertRuleId, ProbeId},
};

const RULES_TABLE: TableDefinition<&str, &[u8]> =
    TableDefinition::new(modelsentry_common::constants::table::ALERT_RULES);
const EVENTS_TABLE: TableDefinition<&str, &[u8]> =
    TableDefinition::new(modelsentry_common::constants::table::ALERT_EVENTS);

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
    /// Returns a database error or a serialization error.
    pub fn insert_rule(&self, rule: &AlertRule) -> Result<()> {
        let bytes = serde_json::to_vec(rule)?;
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(RULES_TABLE)?;
            let id = rule.id.to_string();
            table.insert(id.as_str(), bytes.as_slice())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    /// Return all alert rules for a probe.
    ///
    /// # Errors
    ///
    /// Returns a database error on transaction errors.
    pub fn get_rules_for_probe(&self, probe_id: &ProbeId) -> Result<Vec<AlertRule>> {
        let read_txn = self.db.begin_read()?;
        let table: redb::ReadOnlyTable<&str, &[u8]> = read_txn.open_table(RULES_TABLE)?;
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
    /// Returns a database error on transaction/commit errors.
    pub fn delete_rule(&self, id: &AlertRuleId) -> Result<bool> {
        let write_txn = self.db.begin_write()?;
        let existed = {
            let mut table = write_txn.open_table(RULES_TABLE)?;
            let id_str = id.to_string();
            table.remove(id_str.as_str())?.is_some()
        };
        write_txn.commit()?;
        Ok(existed)
    }

    /// Persist an alert event.
    ///
    /// # Errors
    ///
    /// Returns a database error or a serialization error.
    pub fn insert_event(&self, event: &AlertEvent) -> Result<()> {
        let bytes = serde_json::to_vec(event)?;
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(EVENTS_TABLE)?;
            let id = event.id.to_string();
            table.insert(id.as_str(), bytes.as_slice())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    /// Return the `limit` most-recently-fired alert events.
    ///
    /// # Errors
    ///
    /// Returns a database error on transaction errors.
    pub fn list_events(&self, limit: usize) -> Result<Vec<AlertEvent>> {
        let read_txn = self.db.begin_read()?;
        let table: redb::ReadOnlyTable<&str, &[u8]> = read_txn.open_table(EVENTS_TABLE)?;
        let mut events = Vec::new();
        for entry in table.iter()? {
            let (_, v) = entry?;
            let event: AlertEvent = serde_json::from_slice(v.value())?;
            events.push(event);
        }
        events.sort_by_key(|e| std::cmp::Reverse(e.fired_at));
        Ok(events.into_iter().take(limit).collect())
    }

    /// Most recent `fired_at` timestamp for `rule_id`, or `None` if the rule has
    /// never fired. Drives alert cooldown / de-duplication: the engine suppresses
    /// a new notification while `now − last_fired < cooldown`.
    ///
    /// # Errors
    ///
    /// Returns a database error on transaction errors.
    pub fn last_fired_for_rule(&self, rule_id: &AlertRuleId) -> Result<Option<DateTime<Utc>>> {
        let read_txn = self.db.begin_read()?;
        let table: redb::ReadOnlyTable<&str, &[u8]> = read_txn.open_table(EVENTS_TABLE)?;
        let mut latest: Option<DateTime<Utc>> = None;
        for entry in table.iter()? {
            let (_, v) = entry?;
            let event: AlertEvent = serde_json::from_slice(v.value())?;
            if &event.rule_id == rule_id {
                latest = Some(latest.map_or(event.fired_at, |cur| cur.max(event.fired_at)));
            }
        }
        Ok(latest)
    }

    /// Mark an alert event as acknowledged. Returns `false` if the event is not found.
    ///
    /// # Errors
    ///
    /// Returns a database error on transaction/commit errors.
    pub fn acknowledge_event(&self, id: &Uuid) -> Result<bool> {
        let write_txn = self.db.begin_write()?;
        let updated = {
            let mut table = write_txn.open_table(EVENTS_TABLE)?;
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
        write_txn.commit()?;
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
            target_fpr: 0.01,
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
                combined_p_value: 0.001,
                statistic: 3.0,
                effect_size: 4.0,
                target_fpr: 0.01,
                method: modelsentry_common::constants::method::PER_PROMPT_CONFORMAL.to_string(),
                per_prompt: Vec::new(),
                drift_level: DriftLevel::High,
                interpretation: String::new(),
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
    fn last_fired_for_rule_returns_most_recent_and_none_when_absent() {
        let (_dir, store) = open_test_db();
        let probe_id = ProbeId::new();
        let rule = make_rule(&probe_id);

        // No events yet.
        assert!(
            store
                .alerts()
                .last_fired_for_rule(&rule.id)
                .unwrap()
                .is_none()
        );

        let mut older = make_event(&rule.id);
        let mut newer = make_event(&rule.id);
        older.fired_at = Utc::now();
        newer.fired_at = older.fired_at + chrono::Duration::seconds(30);
        // Insert out of order to prove it returns the max, not the last written.
        store.alerts().insert_event(&newer).unwrap();
        store.alerts().insert_event(&older).unwrap();

        // An event for a *different* rule must not count.
        let other_rule = make_rule(&probe_id);
        store
            .alerts()
            .insert_event(&make_event(&other_rule.id))
            .unwrap();

        let last = store.alerts().last_fired_for_rule(&rule.id).unwrap();
        assert_eq!(last, Some(newer.fired_at));
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
