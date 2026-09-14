//! Canonical merchant-identity entity layer (ADR 0030 addendum; bead personal-cfo-zrpg).
//!
//! `merchant_identities` are real-world merchants; `merchant_aliases` map each
//! `categorization::normalize_merchant` key (7yh0) onto exactly one identity, so different
//! raw strings for the same merchant (`"AMZN MKTP US"`, `"AMAZON COM"`) resolve together. A
//! category learned for one alias then applies to all, and `is_ambiguous` flags merchants
//! that span categories so auto-categorization defers to disambiguation.
//!
//! This is the schema seam only — seeding, fuzzy alias matching, and the read-model
//! population are downstream beads (7yh0-v2, the ambiguous-merchant beads), each designed
//! against its real consumer.

use categorization::normalize_merchant;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::DbError;

/// A canonical merchant seeded at vault init (ADR 0030 addendum, personal-cfo-6p6b / -2a6r /
/// -9rtv / -x75h). Each ambiguous-merchant bead extends [`SEED_MERCHANTS`].
struct SeedMerchant {
    display_name: &'static str,
    ambiguous: bool,
    /// Descriptors that resolve by exact normalized key — online merchants whose key is
    /// stable (`AMAZON.COM`, `APPLE.COM/BILL`).
    exact_aliases: &'static [&'static str],
    /// Brand prefixes that resolve any key beginning with them on a word boundary — physical
    /// chains whose normalized key keeps the city (`COSTCO` matches `COSTCO WHSE SEATTLE`).
    /// Must be unambiguous brand tokens; the word boundary keeps `APPLE` off `APPLEBEE'S`.
    prefix_aliases: &'static [&'static str],
}

/// Canonical merchants seeded into every vault — genuinely category-spanning merchants
/// flagged ambiguous so the merchant-memory guard (7yh0-v2) defers them to disambiguation.
/// Online merchants use exact aliases; physical chains use brand-prefix aliases (their key
/// keeps the store city). Consistently-categorizable sub-brands (Amazon Prime, Whole Foods,
/// Apple Card payments) are deliberately left out so they categorize normally.
const SEED_MERCHANTS: &[SeedMerchant] = &[
    SeedMerchant {
        display_name: "Amazon",
        ambiguous: true,
        exact_aliases: &[
            "AMZN MKTP US",
            "AMAZON.COM",
            "AMAZON MKTPL",
            "AMAZON MARKETPLACE",
            "AMZN MKTP",
        ],
        prefix_aliases: &[],
    },
    SeedMerchant {
        display_name: "Apple",
        ambiguous: true,
        exact_aliases: &[
            "APPLE.COM/BILL",
            "APPLE.COM",
            "ITUNES.COM/BILL",
            "ITUNES.COM",
        ],
        prefix_aliases: &["APPLE"],
    },
    SeedMerchant {
        display_name: "Costco",
        ambiguous: true,
        exact_aliases: &["COSTCO.COM"],
        prefix_aliases: &["COSTCO"],
    },
    SeedMerchant {
        display_name: "Target",
        ambiguous: true,
        exact_aliases: &["TARGET.COM"],
        prefix_aliases: &["TARGET"],
    },
    SeedMerchant {
        display_name: "Walmart",
        ambiguous: true,
        exact_aliases: &["WALMART.COM"],
        prefix_aliases: &["WALMART", "WAL-MART", "WM SUPERCENTER"],
    },
    // P2P transfers: ambiguous when the descriptor carries no counterparty (a bare "VENMO").
    // When it does, `normalize_merchant` strips the processor prefix and the counterparty
    // survives as the key, which groups + categorizes on its own — so only the bare forms
    // are seeded. The richer amount/recurrence signals are a follow-on (personal-cfo-x75h).
    SeedMerchant {
        display_name: "Venmo",
        ambiguous: true,
        exact_aliases: &["VENMO"],
        prefix_aliases: &[],
    },
    SeedMerchant {
        display_name: "Zelle",
        ambiguous: true,
        exact_aliases: &["ZELLE"],
        prefix_aliases: &[],
    },
    SeedMerchant {
        display_name: "Cash App",
        ambiguous: true,
        exact_aliases: &["CASH APP", "CASHAPP"],
        prefix_aliases: &[],
    },
];

/// Idempotently seed the canonical merchants + aliases (ADR 0030 addendum). Run in the vault
/// bootstrap after the default taxonomy. Deterministic v5 ids + `INSERT OR IGNORE` make a
/// re-run a no-op, and a user's own alias mapping for the same normalized key is never
/// clobbered (the alias PK already exists → ignored).
///
/// # Errors
/// Returns [`DbError`] if a SQLite operation fails.
pub(crate) fn ensure_seed_merchants(conn: &Connection) -> Result<(), DbError> {
    let now = Utc::now().to_rfc3339();
    for m in SEED_MERCHANTS {
        let id = Uuid::new_v5(
            &Uuid::NAMESPACE_OID,
            format!("merchant-seed:{}", m.display_name).as_bytes(),
        );
        conn.execute(
            "INSERT OR IGNORE INTO merchant_identities
                (id, display_name, default_category_id, is_ambiguous, source, created_at)
             VALUES (?1, ?2, NULL, ?3, 'seed', ?4)",
            params![id, m.display_name, i64::from(m.ambiguous), now],
        )?;
        for raw in m.exact_aliases {
            seed_alias(conn, raw, id, "exact", &now)?;
        }
        for raw in m.prefix_aliases {
            seed_alias(conn, raw, id, "prefix", &now)?;
        }
    }
    Ok(())
}

/// Insert one seed alias (normalized), `INSERT OR IGNORE` so a user mapping is never clobbered.
fn seed_alias(
    conn: &Connection,
    raw: &str,
    identity_id: Uuid,
    match_type: &str,
    now: &str,
) -> Result<(), DbError> {
    let key = normalize_merchant(raw);
    if key.is_empty() {
        return Ok(());
    }
    conn.execute(
        "INSERT OR IGNORE INTO merchant_aliases
            (normalized_key, merchant_identity_id, source, confidence_bps, match_type, created_at)
         VALUES (?1, ?2, 'seed', 10000, ?3, ?4)",
        params![key, identity_id, match_type, now],
    )?;
    Ok(())
}

/// A canonical merchant entity resolved from a normalized merchant key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MerchantIdentity {
    /// Stable entity id.
    pub id: Uuid,
    /// Display name, e.g. `"Amazon"`.
    pub display_name: String,
    /// The entity's usual category when unambiguous — the auto-categorization seam. `None`
    /// for ambiguous merchants.
    pub default_category_id: Option<Uuid>,
    /// True for merchants that span categories (Amazon, Costco, Venmo): auto-categorization
    /// must defer to disambiguation / a split template rather than apply one category.
    pub is_ambiguous: bool,
    /// Provenance: `seed` | `user` | `auto`.
    pub source: String,
}

/// Create a merchant identity, returning its id.
///
/// # Errors
/// Returns [`DbError`] if the insert fails (e.g. an out-of-domain `source`).
pub(crate) fn create_identity(
    conn: &Connection,
    display_name: &str,
    default_category_id: Option<Uuid>,
    is_ambiguous: bool,
    source: &str,
) -> Result<Uuid, DbError> {
    let id = Uuid::now_v7();
    conn.execute(
        "INSERT INTO merchant_identities
            (id, display_name, default_category_id, is_ambiguous, source, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            id,
            display_name,
            default_category_id,
            i64::from(is_ambiguous),
            source,
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(id)
}

/// Point a normalized merchant key at an identity. Re-linking a key moves it — the key is the
/// primary key, since a deterministic key resolves to exactly one identity.
///
/// # Errors
/// Returns [`DbError`] if the insert fails (e.g. an out-of-domain `source`).
pub(crate) fn link_alias(
    conn: &Connection,
    normalized_key: &str,
    merchant_identity_id: Uuid,
    source: &str,
    confidence_bps: i64,
) -> Result<(), DbError> {
    conn.execute(
        "INSERT OR REPLACE INTO merchant_aliases
            (normalized_key, merchant_identity_id, source, confidence_bps, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            normalized_key,
            merchant_identity_id,
            source,
            confidence_bps,
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(())
}

/// Resolve a normalized merchant key (the `normalize_merchant` output) to its canonical
/// identity: an exact alias first, then a **prefix** alias whose token the key begins with on
/// a word boundary (`COSTCO` resolves `COSTCO WHSE SEATTLE`). The longest matching prefix
/// wins, so a more specific brand token is preferred.
///
/// # Errors
/// Returns [`DbError`] if the read fails.
pub(crate) fn resolve(
    conn: &Connection,
    normalized_key: &str,
) -> Result<Option<MerchantIdentity>, DbError> {
    const SELECT: &str =
        "SELECT mi.id, mi.display_name, mi.default_category_id, mi.is_ambiguous, mi.source
         FROM merchant_aliases ma
         JOIN merchant_identities mi ON mi.id = ma.merchant_identity_id";
    let map = |r: &rusqlite::Row| {
        Ok(MerchantIdentity {
            id: r.get(0)?,
            display_name: r.get(1)?,
            default_category_id: r.get(2)?,
            is_ambiguous: r.get::<_, i64>(3)? != 0,
            source: r.get(4)?,
        })
    };

    if let Some(found) = conn
        .query_row(
            &format!("{SELECT} WHERE ma.normalized_key = ?1"),
            params![normalized_key],
            map,
        )
        .optional()?
    {
        return Ok(Some(found));
    }
    // Prefix fallback: the key begins with a prefix alias's token + a space (word boundary),
    // so `APPLE` matches `APPLE STORE …` but never `APPLEBEE'S`.
    let row = conn
        .query_row(
            &format!(
                "{SELECT} WHERE ma.match_type = 'prefix' AND ?1 LIKE ma.normalized_key || ' %'
                 ORDER BY length(ma.normalized_key) DESC LIMIT 1"
            ),
            params![normalized_key],
            map,
        )
        .optional()?;
    Ok(row)
}

/// Count of canonical merchant identities — the observability / test seam.
///
/// # Errors
/// Returns [`DbError`] if the read fails.
pub(crate) fn identity_count(conn: &Connection) -> Result<u64, DbError> {
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM merchant_identities", [], |r| r.get(0))?;
    Ok(u64::try_from(n).unwrap_or(0))
}
