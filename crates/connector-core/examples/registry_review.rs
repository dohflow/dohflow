//! Connector registry review cadence (ADR 0015 §7, personal-cfo-5jjz).
//!
//! Prints a GitHub Actions `::warning::` for every shipped provider whose
//! cost or terms review is older than six months. Always exits 0: a stale
//! review is a signal for a human to re-check the provider's pages, never a
//! reason to block an unrelated release. CI runs it as
//! `cargo run -p connector-core --example registry_review`.

// Link every shipped adapter crate (see tests/registry.rs).
use simplefin_adapter as _;

use connector_core::{all_registrations, ReviewedFact, REVIEW_CADENCE_MONTHS};

fn main() {
    // The one wall-clock read, at the CLI edge; the check itself is
    // `ConnectorEconomics::overdue_reviews`, tested with injected dates.
    let today = chrono::Utc::now().date_naive();
    let mut stale = 0_usize;
    for registration in all_registrations() {
        let id = registration.adapter.id();
        let economics = &registration.metadata.economics;
        for fact in economics.overdue_reviews(today) {
            let (what, reviewed) = match fact {
                ReviewedFact::Cost => ("cost", economics.cost_reviewed_at),
                ReviewedFact::Terms => ("terms", economics.terms_reviewed_at),
            };
            println!(
                "::warning title=Connector registry review overdue::{id}: {what} last reviewed \
                 {reviewed}, more than {REVIEW_CADENCE_MONTHS} months ago — re-check the \
                 provider's pages and update its ConnectorMetadata (ADR 0015 §7)"
            );
            stale += 1;
        }
    }
    println!("connector registry review cadence: {stale} overdue review(s) as of {today}");
}
