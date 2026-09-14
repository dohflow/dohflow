//! Spend rolled up by category over a date range (personal-cfo-4d8.27.8.2, ADR 0052).
//!
//! Feeds the interrogatable spend breakdown on Transactions. Three rules from ADR 0052
//! decide what the numbers mean, and all three are the difference between a chart the
//! user can trust and one that quietly disagrees with the list beneath it:
//!
//! - **Splits win over the parent categorization** (§3). A split transaction carries
//!   per-line categories (ADR 0034), so a $200 trip itemized into Groceries $150 /
//!   Household $50 must count as two amounts, not $200 against whichever single category
//!   the parent happens to hold.
//! - **Only EXPENSE categories are spend** (§4). The taxonomy's `type` is the honest
//!   discriminator, and the two-user-posting shape is not enough on its own: the dominant
//!   transfer in an import-first app is a ONE-SIDED row (a "Credit Card Payment" the
//!   importer matched by name), which has a system counter-posting and would otherwise
//!   read as $500 of spending — on top of the card's own charges, so the same money twice.
//!   It also keeps a categorized paycheck from landing as a NEGATIVE spend bar, which
//!   would corrupt any percentage-of-total the chart draws.
//! - **Transfers are not spend** (§4). A transfer is internal movement and lands as *two*
//!   postings, so counting it would both invent spending and double it. Belt and braces
//!   with the type filter above, since a user can categorize anything.
//! - **Refunds net** (§4). A positive amount in a category reduces that category rather
//!   than being dropped, so a returned purchase leaves no phantom spending.

use std::collections::HashMap;

use chrono::NaiveDate;
use rusqlite::Connection;
use uuid::Uuid;

// `escape_like` is THE list's own wildcard escaping, imported rather than copied. A
// second copy here would be the exact parallel-maintenance failure this module's shared
// id-set exists to avoid: fix one and not the other, and a search containing `%` totals
// differently in the chart than it lists below.
use crate::{escape_like, DbError};

/// The chart's rows, plus what the very same query excluded from them
/// (personal-cfo-90eg, ADR 0052 §4).
///
/// The exclusions ride along with the rows deliberately. Computing them in a second query
/// is how the explanation and the chart drift apart: any later change to the date bounds,
/// the currency filter or the facet clause would have to be mirrored, and the first time it
/// was not the footnote would confidently explain a difference that no longer existed.
#[derive(Debug, Clone, Default)]
pub struct SpendBreakdown {
    pub rows: Vec<CategorySpend>,
    /// Outflow in range that carries no category at all, so the chart cannot place it.
    pub uncategorized_minor: i64,
    /// Outflow in range that moved between the household's own accounts — internal
    /// movement, not spend.
    pub transfers_minor: i64,
}

/// One category's spend over the requested range, with its children for drill-down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategorySpend {
    pub category_id: Uuid,
    pub name: String,
    /// The parent this sits under, or `None` at the requested root level.
    pub parent_id: Option<Uuid>,
    /// Net outflow in minor units, **positive for money spent** (refunds subtract).
    /// Includes every descendant's spend — this is the rolled-up figure the chart draws.
    pub total_minor: i64,
    /// Spend attributed to this category itself, excluding descendants. `total_minor`
    /// minus the children's totals; non-zero when a parent is used directly.
    pub own_minor: i64,
    /// Whether the category has children, so the UI knows a cell can be drilled into
    /// without a second query.
    pub has_children: bool,
}

/// The transaction-list facets, mirrored onto the aggregate (ADR 0052 §2).
///
/// Deliberately NOT carrying the category facet: on this surface a category selection is
/// the chart's DRILL LEVEL (`parent`), not a filter over the aggregate. Applying it as
/// both would collapse the breakdown to a single bar and constrain the same thing twice.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpendFilters {
    /// Free-text over memo / counterparty / note / account name.
    pub query: Option<String>,
    /// Restrict to these accounts; **empty means no constraint**. Mirrors
    /// `TransactionPageQuery::account_ids` exactly — the Debt page gives a multi-card
    /// selection a spending breakdown, and a chart that honoured only the first card
    /// while the list below showed all of them is the disagreement ADR 0052 §2 exists to
    /// prevent (ADR 0057 §2).
    pub account_ids: Vec<Uuid>,
    pub tag_id: Option<Uuid>,
    pub unreviewed_only: bool,
}

impl SpendFilters {
    fn is_empty(&self) -> bool {
        self.query.as_deref().is_none_or(|q| q.trim().is_empty())
            && self.account_ids.is_empty()
            && self.tag_id.is_none()
            && !self.unreviewed_only
    }
}

/// The transactions the LIST would show for these facets, as a **non-correlated**
/// `IN (SELECT …)` plus its bound values — or `None` when nothing is filtered.
///
/// Resolving the set ONCE and constraining both spend branches with it is the whole
/// design. The aggregate is a UNION of two very different shapes (postings for un-split
/// transactions, `split_lines` for split ones) and the split side joins neither
/// `accounts` nor the review/provenance tables at all. Bolting four predicates onto each
/// branch by hand would mean two hand-maintained copies of the list's semantics, and the
/// first one to drift makes a bar disagree with the rows it drills into — the single
/// failure this surface cannot afford (ADR 0052 §2).
///
/// Parameters are numbered from `?4`, since the caller holds `?1`–`?3` for from / to /
/// currency; the same fragment is therefore embedded in both branches unchanged.
fn matching_transactions(
    filters: &SpendFilters,
) -> Option<(String, Vec<Box<dyn rusqlite::ToSql>>)> {
    if filters.is_empty() {
        return None;
    }
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    let mut next = 4;
    let mut bind = |value: Box<dyn rusqlite::ToSql>| {
        params.push(value);
        let n = next;
        next += 1;
        format!("?{n}")
    };

    // The tag facet is an INNER JOIN rather than a subquery, matching the list:
    // (transaction_id, tag_id) is the PK, so it matches at most once and cannot
    // multiply rows.
    let tag_join = match filters.tag_id {
        Some(tag_id) => {
            let p = bind(Box::new(tag_id));
            format!(
                "JOIN transaction_tags tagf ON tagf.transaction_id = ltf.id \
                 AND tagf.tag_id = {p}"
            )
        }
        None => String::new(),
    };

    let mut clauses = vec!["ltf.voided_at IS NULL".to_owned()];
    if let Some(needle) = filters
        .query
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let pattern = format!("%{}%", escape_like(needle));
        // The same four fields the list spans; NULL fields simply do not match.
        let ps: Vec<String> = (0..4).map(|_| bind(Box::new(pattern.clone()))).collect();
        clauses.push(format!(
            "(tdf.memo LIKE {} ESCAPE '\\' OR tdf.counterparty LIKE {} ESCAPE '\\'
              OR tdf.note LIKE {} ESCAPE '\\' OR af.name LIKE {} ESCAPE '\\')",
            ps[0], ps[1], ps[2], ps[3]
        ));
    }
    if !filters.account_ids.is_empty() {
        let ps: Vec<String> = filters
            .account_ids
            .iter()
            .map(|id| bind(Box::new(*id)))
            .collect();
        clauses.push(format!("af.id IN ({})", ps.join(", ")));
    }
    if filters.unreviewed_only {
        // Kept in sync with TXN_ROW_COLUMNS' derived-reviewed expression (ADR 0032):
        // an imported row is unreviewed until reviewed, a hand-entered one is reviewed.
        clauses.push(
            "COALESCE(trf.reviewed, CASE WHEN provf.entity_id IS NULL THEN 1 ELSE 0 END) = 0"
                .to_owned(),
        );
    }
    let where_sql = clauses.join(" AND ");

    // DISTINCT because a transfer has a posting per account: without it the same
    // transaction id would appear twice, which an `IN (…)` tolerates but which would
    // mislead anyone reading the plan.
    let sql = format!(
        "SELECT DISTINCT lpf.transaction_id
           FROM ledger_postings lpf
           JOIN ledger_transactions ltf ON ltf.id = lpf.transaction_id
           JOIN ledger_accounts laf ON laf.id = lpf.ledger_account_id
           JOIN accounts af ON af.ledger_account_id = laf.id
           {tag_join}
           LEFT JOIN transaction_details tdf ON tdf.transaction_id = ltf.id
           LEFT JOIN transaction_reviews trf ON trf.transaction_id = ltf.id
           LEFT JOIN (SELECT entity_id FROM source_provenance_links
                       WHERE entity_type = 'ledger_transaction' GROUP BY entity_id) provf
                  ON provf.entity_id = ltf.id
          WHERE {where_sql}"
    );
    Some((sql, params))
}

/// Spend by category over `[from, to]` (inclusive dates), rolled up to the children of
/// `parent` — `None` for the taxonomy roots — narrowed by the list's `filters`.
///
/// The returned rows are the requested level only; each carries its whole subtree's
/// total, so drilling in is another call with `parent = that id`.
///
/// # Errors
/// [`DbError`] on a read failure.
pub(crate) fn spend_by_category(
    conn: &Connection,
    from: NaiveDate,
    to: NaiveDate,
    parent: Option<Uuid>,
    currency: &str,
    filters: &SpendFilters,
) -> Result<SpendBreakdown, DbError> {
    // 1. Per-category leaf totals over the range.
    //
    // Two disjoint sources, unioned: split lines where a transaction HAS them, and the
    // transaction's own categorization where it does not. The `NOT EXISTS` guard is what
    // keeps a split transaction from being counted twice.
    //
    // `lp.minor_units < 0` on the posting selects outflows for the un-split side; splits
    // carry their own signed amounts, so they are negated to the same "positive = spent"
    // convention. Transfers are excluded by requiring that the transaction has no second
    // USER posting (ADR 0052 §4).
    // The list's facets, resolved once and applied identically to both branches below.
    let matching = matching_transactions(filters);
    let filter_clause = match &matching {
        Some((sql, _)) => format!("AND lt.id IN ({sql})"),
        None => String::new(),
    };
    let mut stmt = conn.prepare(&format!(
        "WITH spend AS (
             -- Un-split transactions: the posting amount, attributed to its category.
             SELECT tc.category_id AS category_id, -SUM(lp.minor_units) AS minor
               FROM ledger_postings lp
               JOIN ledger_transactions lt ON lt.id = lp.transaction_id
               JOIN ledger_accounts la ON la.id = lp.ledger_account_id
               JOIN accounts a ON a.ledger_account_id = la.id
               JOIN transaction_categorizations tc ON tc.transaction_id = lt.id
               JOIN categories c ON c.id = tc.category_id
              WHERE lt.voided_at IS NULL
                AND c.type = 'expense' 
                AND lp.currency = ?3
                AND substr(lt.occurred_at, 1, 10) >= ?1
                AND substr(lt.occurred_at, 1, 10) <= ?2
                AND NOT EXISTS (SELECT 1 FROM split_lines sl WHERE sl.transaction_id = lt.id)
                AND NOT EXISTS (
                    SELECT 1 FROM ledger_postings lp2
                      JOIN ledger_accounts la2 ON la2.id = lp2.ledger_account_id
                      JOIN accounts a2 ON a2.ledger_account_id = la2.id
                     WHERE lp2.transaction_id = lt.id AND a2.id <> a.id)
                {filter_clause}
              GROUP BY tc.category_id
             UNION ALL
             -- Split transactions: each line, attributed to the line's own category.
             SELECT sl.category_id AS category_id, -SUM(sl.amount_minor) AS minor
               FROM split_lines sl
               JOIN ledger_transactions lt ON lt.id = sl.transaction_id
               JOIN categories c ON c.id = sl.category_id
              WHERE lt.voided_at IS NULL
                AND c.type = 'expense'
                AND sl.currency = ?3
                AND sl.category_id IS NOT NULL
                -- Same transfer guard as above: a split of a transfer would otherwise
                -- turn internal movement into spend.
                AND NOT EXISTS (
                    SELECT 1 FROM ledger_postings lp2
                      JOIN ledger_accounts la2 ON la2.id = lp2.ledger_account_id
                      JOIN accounts a2 ON a2.ledger_account_id = la2.id
                      JOIN ledger_postings lp3 ON lp3.transaction_id = lp2.transaction_id
                      JOIN ledger_accounts la3 ON la3.id = lp3.ledger_account_id
                      JOIN accounts a3 ON a3.ledger_account_id = la3.id
                     WHERE lp2.transaction_id = lt.id AND a2.id <> a3.id)
                AND substr(lt.occurred_at, 1, 10) >= ?1
                AND substr(lt.occurred_at, 1, 10) <= ?2
                {filter_clause}
              GROUP BY sl.category_id
         )
         SELECT c.id, c.name, c.parent_id, COALESCE(SUM(spend.minor), 0)
           FROM categories c
           LEFT JOIN spend ON spend.category_id = c.id
          GROUP BY c.id, c.name, c.parent_id"
    ))?;

    struct Node {
        name: String,
        parent_id: Option<Uuid>,
        own: i64,
    }
    let mut nodes: HashMap<Uuid, Node> = HashMap::new();
    // `?1`–`?3` then the filter's `?4…`, in the order `matching_transactions` bound them.
    let mut bound: Vec<Box<dyn rusqlite::ToSql>> = vec![
        Box::new(from.to_string()),
        Box::new(to.to_string()),
        Box::new(currency.to_owned()),
    ];
    if let Some((_, filter_params)) = matching {
        bound.extend(filter_params);
    }
    let mut rows = stmt.query(rusqlite::params_from_iter(bound.iter()))?;
    while let Some(row) = rows.next()? {
        let id: Uuid = row.get(0)?;
        nodes.insert(
            id,
            Node {
                name: row.get(1)?,
                parent_id: row.get(2)?,
                own: row.get(3)?,
            },
        );
    }

    // 2. Roll each category's own spend up through its ancestors, so a parent's total
    // includes every descendant. Walking ancestors per node (rather than recursing down)
    // keeps this linear in the taxonomy and needs no ordering assumptions; the schema's
    // cycle trigger guarantees the walk terminates, and the visited set makes that
    // robust even against a hand-edited vault.
    let mut totals: HashMap<Uuid, i64> = HashMap::new();
    for (id, node) in &nodes {
        if node.own == 0 {
            totals.entry(*id).or_insert(0);
            continue;
        }
        *totals.entry(*id).or_insert(0) += node.own;
        let mut cursor = node.parent_id;
        let mut seen = vec![*id];
        while let Some(ancestor) = cursor {
            if seen.contains(&ancestor) {
                break;
            }
            seen.push(ancestor);
            *totals.entry(ancestor).or_insert(0) += node.own;
            cursor = nodes.get(&ancestor).and_then(|n| n.parent_id);
        }
    }

    // 3. Emit the requested level, in a deterministic order (biggest spend first, then
    // name) so the chart's bars do not reshuffle between identical reads.
    let mut out: Vec<CategorySpend> = nodes
        .iter()
        .filter(|(_, node)| node.parent_id == parent)
        .map(|(id, node)| CategorySpend {
            category_id: *id,
            name: node.name.clone(),
            parent_id: node.parent_id,
            total_minor: totals.get(id).copied().unwrap_or(0),
            own_minor: node.own,
            has_children: nodes.values().any(|n| n.parent_id == Some(*id)),
        })
        .collect();
    out.sort_by(|a, b| {
        b.total_minor
            .cmp(&a.total_minor)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.category_id.cmp(&b.category_id))
    });

    // 4. What the same range and facets left OUT, so the surface can explain the gap
    //    between this chart and the list beside it rather than let them silently disagree.
    let (uncategorized_minor, transfers_minor) =
        excluded_outflows(conn, from, to, currency, filters)?;

    Ok(SpendBreakdown {
        rows: out,
        uncategorized_minor,
        transfers_minor,
    })
}

/// The outflow the chart could not place, over the SAME range, currency and facets.
///
/// Deliberately built from the same `matching_transactions` clause and the same date/currency
/// binds as the spend query above — the two must move together, or the footnote will one day
/// explain a difference that no longer exists.
///
/// Returns `(uncategorized, transfers)`, both positive minor units.
fn excluded_outflows(
    conn: &Connection,
    from: NaiveDate,
    to: NaiveDate,
    currency: &str,
    filters: &SpendFilters,
) -> Result<(i64, i64), DbError> {
    let matching = matching_transactions(filters);
    let filter_clause = match &matching {
        Some((sql, _)) => format!("AND lt.id IN ({sql})"),
        None => String::new(),
    };

    // `is_transfer` is the same two-sided test the spend query uses to drop transfers: a
    // transaction with a second USER posting is internal movement.
    let sql = format!(
        "SELECT
             COALESCE(SUM(CASE WHEN NOT is_transfer AND category_id IS NULL THEN owed END), 0),
             COALESCE(SUM(CASE WHEN is_transfer THEN owed END), 0)
           FROM (
             SELECT lt.id AS txn_id,
                    -SUM(lp.minor_units) AS owed,
                    MAX(tc.category_id IS NOT NULL) AS has_category,
                    MIN(tc.category_id) AS category_id,
                    EXISTS (
                      SELECT 1 FROM ledger_postings lp2
                        JOIN ledger_accounts la2 ON la2.id = lp2.ledger_account_id
                        JOIN accounts a2 ON a2.ledger_account_id = la2.id
                       WHERE lp2.transaction_id = lt.id AND a2.id <> a.id) AS is_transfer
               FROM ledger_postings lp
               JOIN ledger_transactions lt ON lt.id = lp.transaction_id
               JOIN ledger_accounts la ON la.id = lp.ledger_account_id
               JOIN accounts a ON a.ledger_account_id = la.id
               LEFT JOIN transaction_categorizations tc ON tc.transaction_id = lt.id
              WHERE lt.voided_at IS NULL
                AND lp.minor_units < 0
                AND lp.currency = ?3
                AND substr(lt.occurred_at, 1, 10) >= ?1
                AND substr(lt.occurred_at, 1, 10) <= ?2
                {filter_clause}
              GROUP BY lt.id, a.id)"
    );

    let mut stmt = conn.prepare(&sql)?;
    let mut binds: Vec<Box<dyn rusqlite::ToSql>> = vec![
        Box::new(from.to_string()),
        Box::new(to.to_string()),
        Box::new(currency.to_owned()),
    ];
    if let Some((_, params)) = matching {
        binds.extend(params);
    }
    let row = stmt.query_row(rusqlite::params_from_iter(binds.iter()), |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
    })?;
    Ok(row)
}
