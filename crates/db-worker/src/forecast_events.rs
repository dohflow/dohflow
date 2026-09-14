//! Typed creation of forecast assumption events (ADR 0026 §4, personal-cfo-6zep).
//!
//! The generalized writer behind the `create_forecast_assumption` IPC. A typed
//! [`AssumptionParams`] — additions (`one_time_event`), amount modifications
//! (`bill_amount` / `income_amount`), date modifications (`bill_date` /
//! `income_date`), and removals (`exclusion`) — plus the scenario it belongs to
//! (`None` = base) and the entity it targets, lowered to a [`NewAssumptionEvent`]
//! whose `params_json` is built **by hand** (`serde_json` stays off the write path;
//! see [`crate::forecast_persist`]).
//!
//! Invalid field combinations are unrepresentable: the IPC layer validates its wire
//! input into one of these variants before recording. The read side that consumes
//! the events is [`crate::forecast_overrides`] (modifications + removals) and
//! [`crate::manual_entry`] (additions); building the JSON here guarantees the
//! shapes match what those parsers expect.

use chrono::NaiveDate;
use core_money::Money;
use uuid::Uuid;

use crate::assumptions::{AssumptionKind, AssumptionSource, NewAssumptionEvent};
use crate::forecast_persist::json_str;
use crate::manual_entry;

/// The typed parameters of a forecast assumption event — one variant per supported
/// `create_forecast_assumption` shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssumptionParams {
    /// A one-off cash event (addition); `amount`'s sign is the direction.
    OneTimeEvent {
        amount: Money,
        date: NaiveDate,
        label: String,
    },
    /// Replace an income source's amount within the window `[effective_date,
    /// end_date]` (open start / open end when absent). Multiple compose by window
    /// (ADR 0026 §4, personal-cfo-w6o9).
    IncomeAmount {
        new_amount_minor: i64,
        effective_date: Option<NaiveDate>,
        end_date: Option<NaiveDate>,
    },
    /// Replace a recurring bill's amount within the window `[effective_date,
    /// end_date]` (open start / open end when absent). Multiple compose by window.
    BillAmount {
        new_amount_minor: i64,
        effective_date: Option<NaiveDate>,
        end_date: Option<NaiveDate>,
    },
    /// Shift an income source's schedule anchor.
    IncomeDate { new_anchor_date: NaiveDate },
    /// Shift a recurring bill's schedule anchor.
    BillDate { new_anchor_date: NaiveDate },
    /// Remove the target's occurrences, optionally only on/after `effective_date`.
    Exclusion { effective_date: Option<NaiveDate> },
    /// A recurring **monthly** liquid outflow — an extra debt payment (personal-cfo-6wk.19,
    /// the ADR 0036 "extra $X/mo against debt from D" overlay). Free-standing (no base entity);
    /// `amount` is the positive payment magnitude, projected as a negative outflow. Recurs on
    /// `anchor_date`'s day-of-month from `anchor_date` through `end_date` (open = the horizon).
    RecurringDebtPayment {
        amount: Money,
        anchor_date: NaiveDate,
        end_date: Option<NaiveDate>,
        label: String,
    },
    /// A planned change to one CATEGORY's discretionary spend (personal-cfo-4d8.27.6.2)
    /// — "reduce Dining by $200/month from August". Targets a category (not a bill or
    /// income), and adjusts the Layer-2 modelled draw rather than emitting an event, so
    /// it cannot double-count spend the band already carries (ADR 0026 §7 addendum).
    /// `delta_minor_per_month` is signed: negative spends less.
    VariableSpendOverride {
        delta_minor_per_month: i64,
        effective_date: Option<NaiveDate>,
        end_date: Option<NaiveDate>,
    },
}

impl AssumptionParams {
    /// The assumption kind this lowers to.
    fn kind(&self) -> AssumptionKind {
        match self {
            Self::OneTimeEvent { .. } => AssumptionKind::OneTimeEvent,
            Self::IncomeAmount { .. } => AssumptionKind::IncomeAmount,
            Self::BillAmount { .. } => AssumptionKind::BillAmount,
            Self::IncomeDate { .. } => AssumptionKind::IncomeDate,
            Self::BillDate { .. } => AssumptionKind::BillDate,
            Self::Exclusion { .. } => AssumptionKind::Exclusion,
            Self::RecurringDebtPayment { .. } => AssumptionKind::RecurringDebtPayment,
            Self::VariableSpendOverride { .. } => AssumptionKind::VariableSpendOverride,
        }
    }

    /// The default target entity type recorded for this kind. Cosmetic — the
    /// override loader keys by `target_entity_id` — so a free-standing one-off and
    /// an exclusion (which may target either an income or a bill) store `None`.
    fn target_entity_type(&self) -> Option<&'static str> {
        match self {
            Self::IncomeAmount { .. } | Self::IncomeDate { .. } => Some("income_source"),
            Self::BillAmount { .. } | Self::BillDate { .. } => Some("recurring_event"),
            Self::VariableSpendOverride { .. } => Some("category"),
            Self::OneTimeEvent { .. }
            | Self::Exclusion { .. }
            | Self::RecurringDebtPayment { .. } => None,
        }
    }

    /// The `params_json` for this event — built by hand so it matches exactly what
    /// the read side ([`crate::forecast_overrides`] / [`crate::manual_entry`])
    /// parses.
    fn params_json(&self) -> String {
        match self {
            Self::OneTimeEvent {
                amount,
                date,
                label,
                // A scenario one-time-event assumption carries no account attribution in v1
                // (personal-cfo-4d8.24.3 covers the Future Cash manual-entry path); it lands
                // Unallocated like before.
            } => manual_entry::build_params_json(*amount, *date, label, None),
            Self::IncomeAmount {
                new_amount_minor,
                effective_date,
                end_date,
            }
            | Self::BillAmount {
                new_amount_minor,
                effective_date,
                end_date,
            } => {
                // Hand-built, fixed field order: new_amount_minor, effective_date?,
                // end_date? — matches what `forecast_overrides` parses.
                let mut parts = format!("\"new_amount_minor\":{new_amount_minor}");
                if let Some(eff) = effective_date {
                    parts.push_str(&format!(
                        ",\"effective_date\":{}",
                        json_str(&eff.to_string())
                    ));
                }
                if let Some(end) = end_date {
                    parts.push_str(&format!(",\"end_date\":{}", json_str(&end.to_string())));
                }
                format!("{{{parts}}}")
            }
            Self::IncomeDate { new_anchor_date } | Self::BillDate { new_anchor_date } => format!(
                "{{\"new_anchor_date\":{}}}",
                json_str(&new_anchor_date.to_string()),
            ),
            Self::Exclusion { effective_date } => match effective_date {
                Some(eff) => format!("{{\"effective_date\":{}}}", json_str(&eff.to_string())),
                None => "{}".to_owned(),
            },
            Self::RecurringDebtPayment {
                amount,
                anchor_date,
                end_date,
                label,
            } => {
                // Hand-built, fixed field order: amount_minor, currency, anchor_date, label,
                // end_date? — matches what `crate::recurring_debt` serde-parses.
                let mut parts = format!(
                    "\"amount_minor\":{},\"currency\":{},\"anchor_date\":{},\"label\":{}",
                    amount.minor_units(),
                    json_str(amount.currency().code()),
                    json_str(&anchor_date.to_string()),
                    json_str(label),
                );
                if let Some(end) = end_date {
                    parts.push_str(&format!(",\"end_date\":{}", json_str(&end.to_string())));
                }
                format!("{{{parts}}}")
            }
            Self::VariableSpendOverride {
                delta_minor_per_month,
                effective_date,
                end_date,
            } => {
                // Hand-built, fixed field order: delta_minor_per_month, effective_date?,
                // end_date? — matches what `forecast_overrides::category_spend_adjustments`
                // parses.
                let mut parts = format!("\"delta_minor_per_month\":{delta_minor_per_month}");
                if let Some(eff) = effective_date {
                    parts.push_str(&format!(
                        ",\"effective_date\":{}",
                        json_str(&eff.to_string())
                    ));
                }
                if let Some(end) = end_date {
                    parts.push_str(&format!(",\"end_date\":{}", json_str(&end.to_string())));
                }
                format!("{{{parts}}}")
            }
        }
    }
}

/// A typed forecast assumption event to record: its [`AssumptionParams`] plus the
/// scenario it belongs to (`None` = base) and the entity it targets (for
/// modifications/removals). Lowered to a row-level [`NewAssumptionEvent`] by
/// [`as_event`](Self::as_event); the source is always `user_override`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForecastAssumptionSpec {
    /// Caller-supplied id (time-ordered `Uuid::now_v7`).
    pub id: Uuid,
    /// `Some` = scenario-scoped overlay; `None` = a base assumption.
    pub scenario_id: Option<Uuid>,
    /// The base income/bill a modification or exclusion targets; `None` for a
    /// free-standing one-off.
    pub target_entity_id: Option<Uuid>,
    pub params: AssumptionParams,
}

impl ForecastAssumptionSpec {
    /// Lower to the row-level [`NewAssumptionEvent`] (kind + hand-built params_json
    /// + `user_override` source).
    pub(crate) fn as_event(&self) -> NewAssumptionEvent {
        NewAssumptionEvent {
            id: self.id,
            kind: self.params.kind(),
            target_entity_type: self.params.target_entity_type().map(str::to_owned),
            target_entity_id: self.target_entity_id,
            params_json: self.params.params_json(),
            source: AssumptionSource::UserOverride,
            scenario_id: self.scenario_id,
            origin_run_id: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_money::Currency;

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn one_time_event_params_match_the_manual_entry_shape() {
        let params = AssumptionParams::OneTimeEvent {
            amount: Money::new(500_000, Currency::Usd),
            date: date("2026-08-01"),
            label: "Bonus".to_owned(),
        };
        assert_eq!(
            params.params_json(),
            r#"{"amount_minor":500000,"currency":"USD","date":"2026-08-01","label":"Bonus"}"#,
        );
        assert_eq!(params.kind(), AssumptionKind::OneTimeEvent);
        assert_eq!(params.target_entity_type(), None);
    }

    #[test]
    fn recurring_debt_payment_params_are_hand_built() {
        let params = AssumptionParams::RecurringDebtPayment {
            amount: Money::new(30_000, Currency::Usd),
            anchor_date: date("2026-08-01"),
            end_date: None,
            label: "Extra debt payment".to_owned(),
        };
        assert_eq!(
            params.params_json(),
            r#"{"amount_minor":30000,"currency":"USD","anchor_date":"2026-08-01","label":"Extra debt payment"}"#,
        );
        assert_eq!(params.kind(), AssumptionKind::RecurringDebtPayment);
        assert_eq!(params.target_entity_type(), None);

        // A bounded window appends end_date.
        let bounded = AssumptionParams::RecurringDebtPayment {
            amount: Money::new(30_000, Currency::Usd),
            anchor_date: date("2026-08-01"),
            end_date: Some(date("2027-08-01")),
            label: "Extra".to_owned(),
        };
        assert_eq!(
            bounded.params_json(),
            r#"{"amount_minor":30000,"currency":"USD","anchor_date":"2026-08-01","label":"Extra","end_date":"2027-08-01"}"#,
        );
    }

    #[test]
    fn amount_override_params_are_compact_and_effective_date_aware() {
        let whole = AssumptionParams::BillAmount {
            new_amount_minor: 250_000,
            effective_date: None,
            end_date: None,
        };
        assert_eq!(whole.params_json(), r#"{"new_amount_minor":250000}"#);
        assert_eq!(whole.target_entity_type(), Some("recurring_event"));

        let dated = AssumptionParams::IncomeAmount {
            new_amount_minor: 320_000,
            effective_date: Some(date("2026-09-01")),
            end_date: None,
        };
        assert_eq!(
            dated.params_json(),
            r#"{"new_amount_minor":320000,"effective_date":"2026-09-01"}"#,
        );
        assert_eq!(dated.target_entity_type(), Some("income_source"));

        // A bounded window emits both dates (personal-cfo-w6o9).
        let windowed = AssumptionParams::IncomeAmount {
            new_amount_minor: 100_000,
            effective_date: Some(date("2026-10-01")),
            end_date: Some(date("2026-12-31")),
        };
        assert_eq!(
            windowed.params_json(),
            r#"{"new_amount_minor":100000,"effective_date":"2026-10-01","end_date":"2026-12-31"}"#,
        );
    }

    #[test]
    fn date_override_and_exclusion_params() {
        assert_eq!(
            AssumptionParams::BillDate {
                new_anchor_date: date("2026-07-15"),
            }
            .params_json(),
            r#"{"new_anchor_date":"2026-07-15"}"#,
        );
        assert_eq!(
            AssumptionParams::Exclusion {
                effective_date: None
            }
            .params_json(),
            "{}",
        );
        assert_eq!(
            AssumptionParams::Exclusion {
                effective_date: Some(date("2026-10-01")),
            }
            .params_json(),
            r#"{"effective_date":"2026-10-01"}"#,
        );
    }

    #[test]
    fn spec_lowers_to_a_user_override_event() {
        let target = Uuid::now_v7();
        let scenario = Uuid::now_v7();
        let id = Uuid::now_v7();
        let event = ForecastAssumptionSpec {
            id,
            scenario_id: Some(scenario),
            target_entity_id: Some(target),
            params: AssumptionParams::BillAmount {
                new_amount_minor: 250_000,
                effective_date: None,
                end_date: None,
            },
        }
        .as_event();
        assert_eq!(event.id, id);
        assert_eq!(event.kind, AssumptionKind::BillAmount);
        assert_eq!(event.source, AssumptionSource::UserOverride);
        assert_eq!(event.scenario_id, Some(scenario));
        assert_eq!(event.target_entity_id, Some(target));
        assert_eq!(event.target_entity_type.as_deref(), Some("recurring_event"));
    }
}
