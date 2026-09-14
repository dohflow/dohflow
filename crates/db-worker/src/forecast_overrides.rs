//! Forecast override + exclusion application (ADR 0026 §4, personal-cfo-6zep).
//!
//! How the projection HONORS assumption events that target a base entity — beyond
//! folding additions ([`crate::manual_entry`]): amount modifications
//! (`bill_amount` / `income_amount`), date modifications (`bill_date` /
//! `income_date`), and removals (`exclusion`). [`entity_overrides`] composes the
//! active events (base + the active scenario layer, latest-wins) into a per-entity
//! map; [`crate::forecast`] consults it while expanding income/bill occurrences, so
//! the same machinery serves base overrides *and* scenario overlays. Params are
//! parsed with `serde_json` (the read path).

use std::collections::HashMap;

use chrono::NaiveDate;
use forecast_engine::layer2::SpendAdjustment;
use rusqlite::Connection;
use uuid::Uuid;

use crate::DbError;

/// From which point an entity is excluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExcludeFrom {
    /// Excluded entirely (all occurrences in the horizon).
    All,
    /// Excluded for occurrences on or after this date.
    Date(NaiveDate),
}

/// One windowed amount override: replace the amount for occurrences whose date falls
/// in `[start, end]` (an absent bound is open). `seq` is the event's creation order,
/// so overlapping windows resolve later-created-wins (ADR 0026 §4, personal-cfo-w6o9).
#[derive(Debug, Clone, Copy)]
struct AmountSegment {
    start: Option<NaiveDate>,
    end: Option<NaiveDate>,
    amount_minor: i64,
    seq: usize,
}

impl AmountSegment {
    /// Whether this window covers `date`.
    fn covers(&self, date: NaiveDate) -> bool {
        self.start.is_none_or(|s| date >= s) && self.end.is_none_or(|e| date <= e)
    }
}

/// The active override/exclusion for one base entity, composed from its targeting
/// assumption events. Amount modifications **compose by window** (the latest-created
/// covering segment wins; uncovered dates fall back to base); the anchor shift and
/// exclusion stay latest-wins.
#[derive(Debug, Clone, Default)]
pub(crate) struct EntityOverride {
    amount_segments: Vec<AmountSegment>,
    new_anchor: Option<NaiveDate>,
    exclude: Option<ExcludeFrom>,
}

impl EntityOverride {
    /// The schedule anchor to expand from: a date override, else the base anchor.
    pub(crate) fn anchor_or(&self, base: NaiveDate) -> NaiveDate {
        self.new_anchor.unwrap_or(base)
    }

    /// Whether the whole entity is excluded (skip it before expanding occurrences).
    pub(crate) fn fully_excluded(&self) -> bool {
        self.exclude == Some(ExcludeFrom::All)
    }

    /// The (positive) amount to project for an occurrence on `date`, or `None` when
    /// that occurrence is excluded. `base_minor` is the entity's stored amount. Among
    /// the windowed amount overrides that cover `date`, the latest-created wins; if
    /// none covers it, the base amount stands.
    pub(crate) fn amount_for(&self, date: NaiveDate, base_minor: i64) -> Option<i64> {
        if let Some(ExcludeFrom::Date(from)) = self.exclude {
            if date >= from {
                return None;
            }
        }
        if self.exclude == Some(ExcludeFrom::All) {
            return None;
        }
        let winner = self
            .amount_segments
            .iter()
            .filter(|seg| seg.covers(date))
            .max_by_key(|seg| seg.seq);
        Some(winner.map_or(base_minor, |seg| seg.amount_minor))
    }
}

/// Load the active override/exclusion map for a forecast run: base events
/// (`scenario_id IS NULL`) plus every scenario in `scenarios`, composed in that order.
///
/// **Selection order is the precedence** (ADR 0059 §1): a later-stacked scenario's event
/// beats an earlier one's for the same entity and field, and creation order breaks ties
/// only *within* a scenario. The effective key is `(selection_rank, created_at, id)` — a
/// total order, so a run is reproducible.
///
/// **Base ranks below every scenario** (ADR 0059 §2), regardless of when it was created.
/// Pure creation-ordering let a base event authored after a scenario's event win, so the
/// overlay silently failed to overlay — and after ADR 0055 that is routine, since applying
/// a scenario promotes its events into base with a fresh `created_at`.
///
/// An empty `scenarios` means base only, which is what the old `None` meant.
pub(crate) fn entity_overrides(
    conn: &Connection,
    scenarios: &[Uuid],
) -> Result<HashMap<Uuid, EntityOverride>, DbError> {
    // Rank 0 is base; each selected scenario takes its 1-based position. Built here so the
    // SQL stays a simple ordered read and the precedence lives in one readable place.
    let rank_of = |scenario_id: Option<Uuid>| -> Option<usize> {
        match scenario_id {
            None => Some(0),
            Some(id) => scenarios.iter().position(|s| *s == id).map(|i| i + 1),
        }
    };

    let placeholders = vec!["?"; scenarios.len()].join(", ");
    let scenario_clause = if scenarios.is_empty() {
        String::new()
    } else {
        format!(" OR scenario_id IN ({placeholders})")
    };
    let sql = format!(
        "SELECT kind, target_entity_id, params_json, scenario_id
         FROM forecast_assumption_events
         WHERE status = 'active' AND target_entity_id IS NOT NULL
           AND (scenario_id IS NULL{scenario_clause})
         ORDER BY created_at, id"
    );
    let mut stmt = conn.prepare(&sql)?;
    let binds: Vec<&dyn rusqlite::ToSql> = scenarios
        .iter()
        .map(|s| s as &dyn rusqlite::ToSql)
        .collect();

    let mut map: HashMap<Uuid, EntityOverride> = HashMap::new();
    let mut rows = stmt.query(rusqlite::params_from_iter(binds))?;
    // Composition index: creation order WITHIN a rank, ranks ordered by selection. Rows
    // arrive in creation order, so a stable sort by rank yields the full precedence.
    let mut pending: Vec<(usize, usize, String, Uuid, String)> = Vec::new();
    let mut arrival = 0usize;
    while let Some(row) = rows.next()? {
        let kind: String = row.get(0)?;
        let target: Uuid = row.get(1)?;
        let params: String = row.get(2)?;
        let scenario_id: Option<Uuid> = row.get(3)?;
        // A row from a scenario that is not selected cannot appear (the WHERE clause
        // filters it), so an unrankable row would mean the query and this map disagree.
        let Some(rank) = rank_of(scenario_id) else {
            continue;
        };
        pending.push((rank, arrival, kind, target, params));
        arrival += 1;
    }
    // Stable sort by rank keeps creation order inside each rank while ordering the ranks
    // by the user's selection — the `(selection_rank, created_at, id)` total order.
    pending.sort_by_key(|(rank, arrival, _, _, _)| (*rank, *arrival));

    for (seq, (_, _, kind, target, params)) in pending.into_iter().enumerate() {
        let value: serde_json::Value = serde_json::from_str(&params)
            .map_err(|e| DbError::InvalidCommand(format!("bad override params: {e}")))?;
        let entry = map.entry(target).or_default();
        match kind.as_str() {
            "bill_amount" | "income_amount" => {
                if let Some(amount) = value
                    .get("new_amount_minor")
                    .and_then(serde_json::Value::as_i64)
                {
                    entry.amount_segments.push(AmountSegment {
                        start: parse_opt_date(&value, "effective_date")?,
                        end: parse_opt_date(&value, "end_date")?,
                        amount_minor: amount,
                        seq,
                    });
                }
            }
            "bill_date" | "income_date" => {
                if let Some(anchor) = parse_opt_date(&value, "new_anchor_date")? {
                    entry.new_anchor = Some(anchor);
                }
            }
            "exclusion" => {
                entry.exclude = Some(match parse_opt_date(&value, "effective_date")? {
                    Some(date) => ExcludeFrom::Date(date),
                    None => ExcludeFrom::All,
                });
            }
            _ => {}
        }
    }
    Ok(map)
}

/// Parse an optional `YYYY-MM-DD` field; absent → `None`, present-but-invalid → err.
fn parse_opt_date(value: &serde_json::Value, key: &str) -> Result<Option<NaiveDate>, DbError> {
    match value.get(key).and_then(serde_json::Value::as_str) {
        Some(raw) => Ok(Some(NaiveDate::parse_from_str(raw, "%Y-%m-%d").map_err(
            |e| DbError::InvalidCommand(format!("bad override {key} {raw:?}: {e}")),
        )?)),
        None => Ok(None),
    }
}

/// Load the planned per-category spend changes for a run (personal-cfo-4d8.27.6.2).
///
/// Separate from [`entity_overrides`] because these are keyed by CATEGORY, not by a
/// bill/income entity: `variable_spend_override` is the one assumption kind whose target
/// is a category, and it adjusts the statistical spend layer rather than a scheduled
/// amount. Base (`scenario_id IS NULL`) and the selected scenario's events both apply,
/// matching every other overlay loader.
///
/// # Errors
/// [`DbError`] on a read failure or malformed params.
pub(crate) fn category_spend_adjustments(
    conn: &Connection,
    scenarios: &[Uuid],
) -> Result<Vec<SpendAdjustment>, DbError> {
    // Additive across the selection: each category adjustment contributes its own cut, so
    // membership is enough — this is not an override race (ADR 0059 §1 governs those).
    let clause = if scenarios.is_empty() {
        String::new()
    } else {
        format!(
            " OR scenario_id IN ({})",
            vec!["?"; scenarios.len()].join(", ")
        )
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT target_entity_id, params_json
         FROM forecast_assumption_events
         WHERE status = 'active' AND kind = 'variable_spend_override'
           AND target_entity_id IS NOT NULL
           AND (scenario_id IS NULL{clause})
         ORDER BY created_at, id"
    ))?;
    let binds: Vec<&dyn rusqlite::ToSql> = scenarios
        .iter()
        .map(|s| s as &dyn rusqlite::ToSql)
        .collect();
    let mut out = Vec::new();
    let mut rows = stmt.query(rusqlite::params_from_iter(binds))?;
    while let Some(row) = rows.next()? {
        let category: Uuid = row.get(0)?;
        let params: String = row.get(1)?;
        let value: serde_json::Value = serde_json::from_str(&params)
            .map_err(|e| DbError::InvalidCommand(format!("bad spend override params: {e}")))?;
        let Some(delta) = value
            .get("delta_minor_per_month")
            .and_then(serde_json::Value::as_i64)
        else {
            continue;
        };
        out.push(SpendAdjustment {
            // The model keys categories by the id string the observations carry
            // (`aggregate::read_variable_spend_history`), so this must match that shape.
            category: category.to_string(),
            delta_cents_per_month: delta,
            start: parse_opt_date(&value, "effective_date")?,
            end: parse_opt_date(&value, "end_date")?,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn seg(start: Option<&str>, end: Option<&str>, amount_minor: i64, seq: usize) -> AmountSegment {
        AmountSegment {
            start: start.map(d),
            end: end.map(d),
            amount_minor,
            seq,
        }
    }

    /// The personal-cfo-w6o9 bug: a paycheck step-down across leave. Each window
    /// applies in its range and the base resumes outside them. (The old single-value
    /// override kept only the last change, so this would have failed.)
    #[test]
    fn windowed_amount_overrides_compose_with_base_resuming() {
        let over = EntityOverride {
            amount_segments: vec![
                seg(Some("2026-10-01"), Some("2026-11-30"), 200_000, 0),
                seg(Some("2026-12-01"), None, 0, 1),
            ],
            ..Default::default()
        };
        let base = 400_000;
        assert_eq!(over.amount_for(d("2026-09-15"), base), Some(base)); // before any window
        assert_eq!(over.amount_for(d("2026-10-15"), base), Some(200_000)); // first window
        assert_eq!(over.amount_for(d("2026-12-15"), base), Some(0)); // second (open-ended)
    }

    /// A bounded `[start,end]` override returns to base after its end date — the
    /// "back to baseline on Jan 1" the dogfooding report wanted.
    #[test]
    fn a_bounded_window_returns_to_base_after_its_end() {
        let over = EntityOverride {
            amount_segments: vec![seg(Some("2026-12-15"), Some("2027-01-01"), 0, 0)],
            ..Default::default()
        };
        let base = 400_000;
        assert_eq!(over.amount_for(d("2026-12-20"), base), Some(0)); // inside the window
        assert_eq!(over.amount_for(d("2027-01-02"), base), Some(base)); // after end -> base
    }

    /// Overlapping windows resolve later-created-wins.
    #[test]
    fn overlapping_windows_resolve_latest_created_wins() {
        let over = EntityOverride {
            amount_segments: vec![
                seg(Some("2026-10-01"), None, 300_000, 0),
                seg(Some("2026-12-15"), None, 0, 1),
            ],
            ..Default::default()
        };
        let base = 400_000;
        assert_eq!(over.amount_for(d("2026-11-01"), base), Some(300_000)); // only the first covers
        assert_eq!(over.amount_for(d("2026-12-20"), base), Some(0)); // both cover -> latest (seq 1)
    }

    /// An open override (no dates) still applies everywhere — the legacy shape.
    #[test]
    fn an_open_override_applies_to_every_date() {
        let over = EntityOverride {
            amount_segments: vec![seg(None, None, 123_456, 0)],
            ..Default::default()
        };
        assert_eq!(over.amount_for(d("2030-01-01"), 999), Some(123_456));
    }
}
