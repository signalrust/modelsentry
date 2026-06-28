//! Per-rule alpha-spend ledger backing rolling-window sequential control.
//!
//! `target_fpr` is a **per-run** rate, so every scheduled look at a probe risks
//! an independent false alarm and the expected count grows with the number of
//! runs. This ledger records the testing level ("alpha") spent on each look so
//! the alert engine can cap the **expected number of false alarms** per rule
//! within a rolling window. Entries are keyed chronologically per rule and
//! pruned once they age out of the window, so the table stays bounded by the
//! budget rather than growing with run history.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

use modelsentry_common::error::Result;
use modelsentry_common::types::AlertRuleId;

const TABLE: TableDefinition<&str, &[u8]> =
    TableDefinition::new(modelsentry_common::constants::table::ALERT_SPEND);

/// Zero-pad width for the nanosecond timestamp in a ledger key, so a rule's
/// entries sort chronologically: exactly the number of decimal digits in
/// `i64::MAX` (the largest key value), derived rather than hard-coded.
const NANOS_KEY_WIDTH: usize = i64::MAX.ilog10() as usize + 1;

/// Typed access to the per-rule alpha-spend ledger.
pub struct AlphaSpendStore {
    db: Arc<Database>,
}

impl AlphaSpendStore {
    pub(crate) fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Ledger key for a `(rule, nanos)` spend: `"{rule}|{nanos:019}"`. The
    /// fixed-width, zero-padded nanosecond suffix keeps a rule's entries in
    /// chronological order, so both the in-window sum and the prune are plain
    /// range scans. Negative (pre-epoch) nanoseconds — which never occur for
    /// live spends — clamp to 0 so the padding, and therefore the ordering,
    /// holds.
    fn key_at(rule_id: &AlertRuleId, nanos: i64) -> String {
        format!(
            "{rule_id}|{n:0width$}",
            n = nanos.max(0),
            width = NANOS_KEY_WIDTH
        )
    }

    /// Nanoseconds since the epoch for `at`, clamped into the key's
    /// representable range. chrono returns `None` outside ~[1678, 2262]; such
    /// instants (never produced for live spends) clamp to the nearest bound so
    /// key ordering still holds.
    fn nanos_of(at: DateTime<Utc>) -> i64 {
        at.timestamp_nanos_opt()
            .unwrap_or(if at.timestamp() >= 0 { i64::MAX } else { 0 })
    }

    /// Sum of alpha spent for `rule_id` at or after `since` (the in-window
    /// total the engine compares against the budget).
    ///
    /// # Errors
    ///
    /// Returns a database error on transaction errors.
    pub fn spent_since(&self, rule_id: &AlertRuleId, since: DateTime<Utc>) -> Result<f64> {
        let lo = Self::key_at(rule_id, Self::nanos_of(since));
        let hi = Self::key_at(rule_id, i64::MAX);
        let read_txn = self.db.begin_read()?;
        let table: redb::ReadOnlyTable<&str, &[u8]> = read_txn.open_table(TABLE)?;
        let mut total = 0.0_f64;
        for entry in table.range(lo.as_str()..=hi.as_str())? {
            let (_, v) = entry?;
            total += decode_alpha(v.value());
        }
        Ok(total)
    }

    /// Record an alpha debit of `alpha` for `rule_id` at `at`, then prune that
    /// rule's entries older than `prune_before` (those that have aged out of
    /// the window) so the ledger stays bounded.
    ///
    /// # Errors
    ///
    /// Returns a database error on transaction/commit errors.
    pub fn record_spend(
        &self,
        rule_id: &AlertRuleId,
        at: DateTime<Utc>,
        alpha: f64,
        prune_before: DateTime<Utc>,
    ) -> Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(TABLE)?;
            let key = Self::key_at(rule_id, Self::nanos_of(at));
            table.insert(key.as_str(), alpha.to_le_bytes().as_slice())?;

            // Half-open [epoch, prune_before): every entry strictly older than
            // the window start. Collect keys first so the read borrow is
            // released before the removes.
            let lo = Self::key_at(rule_id, 0);
            let hi = Self::key_at(rule_id, Self::nanos_of(prune_before));
            let stale: Vec<String> = {
                let mut keys = Vec::new();
                for entry in table.range(lo.as_str()..hi.as_str())? {
                    let (k, _) = entry?;
                    keys.push(k.value().to_string());
                }
                keys
            };
            for k in &stale {
                table.remove(k.as_str())?;
            }
        }
        write_txn.commit()?;
        Ok(())
    }
}

/// Decode a ledger value (8 little-endian bytes) back to an `f64`; a
/// malformed/short value reads as `0.0` rather than aborting the sum.
fn decode_alpha(bytes: &[u8]) -> f64 {
    bytes.try_into().map_or(0.0_f64, f64::from_le_bytes)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};
    use tempfile::TempDir;

    use modelsentry_common::types::AlertRuleId;

    use crate::AppStore;

    fn open_test_db() -> (TempDir, AppStore) {
        let dir = TempDir::new().unwrap();
        let store = AppStore::open(&dir.path().join("test.db")).unwrap();
        (dir, store)
    }

    #[test]
    fn spent_since_sums_only_in_window_and_only_this_rule() {
        let (_dir, store) = open_test_db();
        let rule = AlertRuleId::new();
        let other = AlertRuleId::new();
        let now = Utc::now();
        let window_start = now - Duration::days(30);
        // Far in the future so nothing is pruned during these inserts.
        let never_prune = now - Duration::days(3650);

        // Two in-window spends for `rule`.
        store
            .spends()
            .record_spend(&rule, now - Duration::days(1), 0.01, never_prune)
            .unwrap();
        store
            .spends()
            .record_spend(&rule, now - Duration::days(10), 0.01, never_prune)
            .unwrap();
        // One spend for `rule` that is outside the window.
        store
            .spends()
            .record_spend(&rule, now - Duration::days(40), 0.01, never_prune)
            .unwrap();
        // A spend belonging to a different rule must not be counted.
        store
            .spends()
            .record_spend(&other, now - Duration::days(1), 0.01, never_prune)
            .unwrap();

        let spent = store.spends().spent_since(&rule, window_start).unwrap();
        assert!(
            (spent - 0.02).abs() < 1e-9,
            "expected 0.02 in-window for this rule, got {spent}"
        );
    }

    #[test]
    fn record_spend_prunes_entries_older_than_window() {
        let (_dir, store) = open_test_db();
        let rule = AlertRuleId::new();
        let now = Utc::now();

        // Seed an old spend with a no-op prune cutoff so it survives insertion.
        let ancient = now - Duration::days(40);
        store
            .spends()
            .record_spend(&rule, ancient, 0.01, now - Duration::days(3650))
            .unwrap();

        // A fresh spend whose prune cutoff is the window start (30 days) must
        // evict the 40-day-old entry.
        store
            .spends()
            .record_spend(&rule, now, 0.01, now - Duration::days(30))
            .unwrap();

        // Summing from the epoch would include the ancient entry if it were
        // still present; only the fresh 0.01 should remain.
        let total = store
            .spends()
            .spent_since(&rule, now - Duration::days(3650))
            .unwrap();
        assert!(
            (total - 0.01).abs() < 1e-9,
            "ancient entry should have been pruned, total={total}"
        );
    }

    #[test]
    fn spent_since_is_zero_for_unknown_rule() {
        let (_dir, store) = open_test_db();
        let rule = AlertRuleId::new();
        let spent = store
            .spends()
            .spent_since(&rule, Utc::now() - Duration::days(30))
            .unwrap();
        assert!(spent.abs() < f64::EPSILON, "unknown rule spent {spent}");
    }
}
